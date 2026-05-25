mod providers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tauri::{Emitter, Manager};

static APPS_READY: AtomicBool = AtomicBool::new(false);

type Registry = Arc<RwLock<providers::PluginRegistry>>;

fn socket_path() -> std::path::PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(runtime_dir).join("portunus.sock")
}

fn try_signal_running() -> bool {
    use std::io::Write;
    match std::os::unix::net::UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(b"show\n").is_ok(),
        Err(_) => false,
    }
}

fn start_socket_listener(app: tauri::AppHandle) {
    use std::io::BufRead;
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("portunus: failed to bind socket: {e}");
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = std::io::BufReader::new(stream).read_line(&mut line);
            if line.trim() == "show" {
                let _ = app.emit("window-show", ());
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }
    });
}

#[tauri::command]
fn search(query: String, registry: tauri::State<'_, Registry>) -> Vec<providers::SearchResult> {
    registry.read().unwrap().search(&query)
}

fn split_exec(exec: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in exec.chars() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

#[tauri::command]
fn launch_app(app: tauri::AppHandle, exec: String) {
    let args: Vec<String> = split_exec(&exec)
        .into_iter()
        .filter(|s| !(s.len() == 2 && s.starts_with('%')))
        .collect();

    if let Some((program, rest)) = args.split_first() {
        let _ = std::process::Command::new(program)
            .args(rest)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn is_apps_ready() -> bool {
    APPS_READY.load(Ordering::Acquire)
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if std::env::args().any(|a| a == "--show") {
        if !try_signal_running() {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return;
    }

    let registry: Registry = Arc::new(RwLock::new(providers::PluginRegistry::new()));
    let bg_registry = Arc::clone(&registry);

    tauri::Builder::default()
        .manage(registry)
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            start_socket_listener(app.handle().clone());
            bg_registry.write().unwrap().register(providers::calc::CalcProvider);
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let file_provider = providers::files::FileProvider::new();
                bg_registry.write().unwrap().register(file_provider);
                let app_provider = providers::apps::AppProvider::new();
                bg_registry.write().unwrap().register(app_provider);
                APPS_READY.store(true, Ordering::Release);
                let _ = handle.emit("apps-ready", ());
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![search, launch_app, hide_window, is_apps_ready])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

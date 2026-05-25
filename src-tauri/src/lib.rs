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

type PdfRenderMsg = (String, std::sync::mpsc::Sender<Result<Vec<u8>, String>>);

struct PdfWorkerHandle {
    tx: std::sync::mpsc::SyncSender<PdfRenderMsg>,
}

fn start_pdf_worker() -> PdfWorkerHandle {
    let (tx, rx) = std::sync::mpsc::sync_channel::<PdfRenderMsg>(4);
    std::thread::spawn(move || {
        use image::ImageFormat;
        use pdfium_render::prelude::*;
        let pdfium = Pdfium::bind_to_system_library()
            .map(Pdfium::new)
            .map_err(|e| {
                eprintln!("[pdf] bind_to_system_library failed: {e}");
                e.to_string()
            });
        while let Ok((path, reply)) = rx.recv() {
            let result = match &pdfium {
                Err(msg) => Err(msg.clone()),
                Ok(pdfium) => (|| {
                    eprintln!("[pdf] rendering: {path}");
                    let doc = pdfium.load_pdf_from_file(&path, None).map_err(|e| {
                        let msg = e.to_string();
                        eprintln!("[pdf] load_pdf_from_file failed: {msg}");
                        msg
                    })?;
                    eprintln!("[pdf] loaded, {} page(s)", doc.pages().len());
                    let page = doc.pages().get(0).map_err(|e| {
                        let msg = e.to_string();
                        eprintln!("[pdf] get page 0 failed: {msg}");
                        msg
                    })?;
                    let bitmap = page
                        .render_with_config(&PdfRenderConfig::new().set_target_width(800))
                        .map_err(|e| {
                            let msg = e.to_string();
                            eprintln!("[pdf] render failed: {msg}");
                            msg
                        })?;
                    let mut bytes = Vec::new();
                    bitmap
                        .as_image()
                        .into_rgb8()
                        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)
                        .map_err(|e| {
                            let msg = e.to_string();
                            eprintln!("[pdf] jpeg encode failed: {msg}");
                            msg
                        })?;
                    eprintln!("[pdf] done, {} bytes", bytes.len());
                    Ok(bytes)
                })(),
            };
            let _ = reply.send(result);
        }
    });
    PdfWorkerHandle { tx }
}

#[tauri::command]
async fn render_pdf_page(path: String, worker: tauri::State<'_, PdfWorkerHandle>) -> Result<Vec<u8>, String> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Result<Vec<u8>, String>>();
    worker.tx.try_send((path, reply_tx)).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        reply_rx.recv().unwrap_or_else(|e| Err(e.to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
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
        .manage(start_pdf_worker())
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
                let recent_provider = providers::recent::RecentProvider::new();
                bg_registry.write().unwrap().register(recent_provider);
                let app_provider = providers::apps::AppProvider::new();
                bg_registry.write().unwrap().register(app_provider);
                APPS_READY.store(true, Ordering::Release);
                let _ = handle.emit("apps-ready", ());
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![search, launch_app, hide_window, is_apps_ready, render_pdf_page])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

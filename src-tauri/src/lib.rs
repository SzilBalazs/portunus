mod config;
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

fn try_signal_running(cmd: &str) -> bool {
    use std::io::Write;
    match std::os::unix::net::UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(format!("{cmd}\n").as_bytes()).is_ok(),
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
            let cmd = line.trim();
            // "show" or "show:<initial-query>"
            if cmd == "show" || cmd.starts_with("show:") {
                let initial_query = cmd.strip_prefix("show:").map(str::to_string);
                if let Some(q) = initial_query {
                    let _ = app.emit("window-show-query", q);
                } else {
                    let _ = app.emit("window-show", ());
                }
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

fn start_pdf_worker(render_width: u32) -> PdfWorkerHandle {
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
                        .render_with_config(
                            &PdfRenderConfig::new().set_target_width(render_width as i32),
                        )
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
async fn render_pdf_page(
    path: String,
    worker: tauri::State<'_, PdfWorkerHandle>,
) -> Result<Vec<u8>, String> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Result<Vec<u8>, String>>();
    worker
        .tx
        .try_send((path, reply_tx))
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        reply_rx.recv().unwrap_or_else(|e| Err(e.to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn read_text_preview(path: String) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    const MAX_LINES: usize = 300;
    const MAX_BYTES: usize = 32 * 2048;
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = Vec::new();
    let mut total = 0usize;
    for line in BufReader::new(file).lines().take(MAX_LINES) {
        let line = line.map_err(|e| e.to_string())?;
        total += line.len() + 1;
        if total > MAX_BYTES {
            break;
        }
        lines.push(line);
    }
    Ok(lines.join("\n"))
}

#[tauri::command]
async fn render_image_preview(path: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use image::ImageFormat;
        const MAX_WIDTH: u32 = 800;
        let img = image::open(&path).map_err(|e| e.to_string())?;
        let img = if img.width() > MAX_WIDTH {
            img.thumbnail(MAX_WIDTH, u32::MAX)
        } else {
            img
        };
        let mut bytes = Vec::new();
        img.into_rgb8()
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
struct FolderEntry {
    name: String,
    is_dir: bool,
    size: Option<u64>,
}

#[tauri::command]
fn list_folder(path: String) -> Vec<FolderEntry> {
    const MAX: usize = 200;
    let mut entries: Vec<FolderEntry> = std::fs::read_dir(&path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .take(MAX)
        .map(|e| {
            let meta = e.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            FolderEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
                size: meta.and_then(|m| if m.is_file() { Some(m.len()) } else { None }),
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

#[tauri::command]
fn paste_clipboard(app: tauri::AppHandle, id: String) {
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("cliphist decode {} | wl-copy", id))
        .spawn();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn decode_clipboard_entry(id: String) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("cliphist")
        .args(["decode", &id])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

type TimerState = Arc<providers::timer::TimerState>;

#[tauri::command]
fn create_timer(
    duration_secs: u64,
    label: String,
    timer_state: tauri::State<'_, TimerState>,
) -> u32 {
    timer_state.create(duration_secs, label)
}

#[tauri::command]
fn stop_timer(id: u32, timer_state: tauri::State<'_, TimerState>) {
    timer_state.stop(id);
}

#[derive(serde::Serialize, Clone)]
struct TimerExpiredPayload {
    id: u32,
    label: String,
}

fn start_timer_watcher(app: tauri::AppHandle, state: Arc<providers::timer::TimerState>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        for entry in state.drain_expired() {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            let _ = app.emit(
                "timer-expired",
                TimerExpiredPayload {
                    id: entry.id,
                    label: entry.label,
                },
            );
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if std::env::args().any(|a| a == "--show") {
        if !try_signal_running("show") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().any(|a| a == "--clipboard") {
        if !try_signal_running("show:clipboard") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return;
    }

    let cfg = config::Config::load();
    let search_cfg = cfg.search.clone();
    let files_cfg = cfg.files;
    let recent_cfg = cfg.recent;
    let providers_cfg = cfg.providers;
    let pdf_render_width = cfg.pdf.render_width;
    let max_results = cfg.general.max_results;

    let registry: Registry = Arc::new(RwLock::new(providers::PluginRegistry::new(max_results)));
    let bg_registry = Arc::clone(&registry);

    let timer_state: TimerState = providers::timer::TimerState::new();
    let watcher_timer_state = Arc::clone(&timer_state);
    let provider_timer_state = Arc::clone(&timer_state);

    tauri::Builder::default()
        .manage(registry)
        .manage(start_pdf_worker(pdf_render_width))
        .manage(timer_state)
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            start_socket_listener(app.handle().clone());
            start_timer_watcher(app.handle().clone(), watcher_timer_state);
            // Timer and clipboard providers are always enabled — no I/O required.
            bg_registry
                .write()
                .unwrap()
                .register(providers::timer::TimerProvider::new(provider_timer_state));
            bg_registry
                .write()
                .unwrap()
                .register(providers::clipboard::ClipboardProvider);
            if providers_cfg.calc {
                bg_registry
                    .write()
                    .unwrap()
                    .register(providers::calc::CalcProvider);
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if providers_cfg.files {
                    let file_provider =
                        providers::files::FileProvider::new(&files_cfg, &search_cfg);
                    bg_registry.write().unwrap().register(file_provider);
                }
                if providers_cfg.recent {
                    let recent_provider =
                        providers::recent::RecentProvider::new(&recent_cfg, &search_cfg);
                    bg_registry.write().unwrap().register(recent_provider);
                }
                if providers_cfg.apps {
                    let app_provider = providers::apps::AppProvider::new(&search_cfg);
                    bg_registry.write().unwrap().register(app_provider);
                }
                APPS_READY.store(true, Ordering::Release);
                let _ = handle.emit("apps-ready", ());
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            search,
            launch_app,
            hide_window,
            is_apps_ready,
            render_pdf_page,
            create_timer,
            stop_timer,
            read_text_preview,
            render_image_preview,
            list_folder,
            paste_clipboard,
            decode_clipboard_entry
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

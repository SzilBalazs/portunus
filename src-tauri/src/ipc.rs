use std::sync::Arc;

use tauri::{Emitter, Manager};

pub fn socket_path() -> std::path::PathBuf {
    crate::paths::xdg_runtime_dir().join("portunus.sock")
}

pub fn try_signal_running(cmd: &str) -> bool {
    use std::io::Write;
    match std::os::unix::net::UnixStream::connect(socket_path()) {
        Ok(mut stream) => stream.write_all(format!("{cmd}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

pub fn start_socket_listener(
    app: tauri::AppHandle,
    reindex_fn: Option<Arc<dyn Fn() + Send + Sync>>,
    reload_fn: Arc<dyn Fn() + Send + Sync>,
    reload_extensions_fn: Arc<dyn Fn() + Send + Sync>,
    reload_one_extension_fn: Arc<dyn Fn(String) + Send + Sync>,
) {
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
            let app = app.clone();
            let reindex_fn = reindex_fn.clone();
            let reload_fn = Arc::clone(&reload_fn);
            let reload_extensions_fn = Arc::clone(&reload_extensions_fn);
            let reload_one_extension_fn = Arc::clone(&reload_one_extension_fn);
            std::thread::spawn(move || {
                use std::io::BufRead;
                // Prevent a stalled client from blocking this handler forever.
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut line = String::new();
                let _ = std::io::BufReader::new(stream).read_line(&mut line);
                let cmd = line.trim();
                if cmd == "show" || cmd.starts_with("show:") {
                    let initial_query = cmd.strip_prefix("show:").map(str::to_string);
                    if let Some(window) = app.get_webview_window("main") {
                        let already_visible = window.is_visible().unwrap_or(false);
                        if let Some(q) = initial_query {
                            // Always apply a query command (e.g. --clipboard), even if already shown.
                            // Append a trailing space so prefix-based providers like ClipboardProvider activate.
                            let q_with_space = if q.ends_with(' ') { q } else { format!("{q} ") };
                            let _ = app.emit("window-show-query", q_with_space);
                            if !already_visible {
                                let _ = window.show();
                            }
                            let _ = window.set_focus();
                        } else if !already_visible {
                            // Plain --show: no-op when the window is already visible.
                            let _ = app.emit("window-show", ());
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                } else if cmd == "reindex" {
                    if let Some(f) = reindex_fn {
                        std::thread::spawn(move || f());
                    }
                } else if cmd == "reload-config" {
                    std::thread::spawn(move || reload_fn());
                } else if cmd == "reload-extensions" {
                    std::thread::spawn(move || reload_extensions_fn());
                } else if let Some(name) = cmd.strip_prefix("reload-extension:") {
                    // Targeted reload - `portunus ext dev`'s iteration loop.
                    let name = name.trim().to_string();
                    if !name.is_empty() {
                        std::thread::spawn(move || reload_one_extension_fn(name));
                    }
                } else if cmd == "reload-theme" {
                    // Re-read the external matugen.css. Lightweight: no provider
                    // rebuild, just nudge the frontend to re-fetch + re-inject.
                    let _ = app.emit("theme-css-changed", ());
                }
            });
        }
    });
}

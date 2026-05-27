mod config;
mod content_index;
mod frecency;
mod providers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tauri::{Emitter, Manager};

static APPS_READY: AtomicBool = AtomicBool::new(false);

type Registry = Arc<RwLock<providers::PluginRegistry>>;
type FrecencyState = Option<Arc<frecency::FrecencyStore>>;

#[derive(serde::Serialize, Clone)]
struct ContentIndexProgress {
    indexed: usize,
    total: usize,
}

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

fn start_socket_listener(
    app: tauri::AppHandle,
    reindex_fn: Option<Arc<dyn Fn() + Send + Sync>>,
) {
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
                if let Some(f) = &reindex_fn {
                    let f = Arc::clone(f);
                    std::thread::spawn(move || f());
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
fn launch_app(
    app: tauri::AppHandle,
    exec: String,
    id: Option<String>,
    kind: Option<String>,
    frecency: tauri::State<'_, FrecencyState>,
) {
    if let (Some(id), Some(kind), Some(store)) = (&id, &kind, frecency.as_ref()) {
        store.record_launch(id, kind);
    }

    let args: Vec<String> = split_exec(&exec)
        .into_iter()
        .filter(|s| !(s.len() == 2 && s.starts_with('%')))
        .collect();

    if let Some((program, rest)) = args.split_first() {
        use std::os::unix::process::CommandExt;
        let _ = std::process::Command::new(program)
            .args(rest)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
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

fn make_progress_cb(handle: tauri::AppHandle) -> Arc<dyn Fn(usize, usize) + Send + Sync> {
    Arc::new(move |indexed, total| {
        let _ = handle.emit("content-index-progress", ContentIndexProgress { indexed, total });
    })
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
        if !try_signal_running("show:clipboard ") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().any(|a| a == "--reindex") {
        if !try_signal_running("reindex") {
            // No running instance — run standalone with stderr progress
            let cfg = config::Config::load();
            if cfg.content.enabled {
                match content_index::ContentIndex::open() {
                    Ok(index) => {
                        let index = Arc::new(index);
                        index.clear().ok();
                        content_index::run_content_indexer(
                            Arc::clone(&index),
                            &cfg.content,
                            Some(Arc::new(|indexed, total| {
                                eprint!("\r[content] {indexed}/{total}");
                                if indexed >= total {
                                    eprintln!();
                                }
                            })),
                        );
                        eprintln!("[content] reindex complete");
                    }
                    Err(e) => eprintln!("[content] failed to open index: {e}"),
                }
            } else {
                eprintln!("[content] content indexing is disabled in config");
            }
        }
        return;
    }

    let cfg = config::Config::load();
    let search_cfg = cfg.search.clone();
    let frecency_cfg = cfg.frecency;
    let files_cfg = cfg.files;
    let recent_cfg = cfg.recent;
    let providers_cfg = cfg.providers;
    let pdf_render_width = cfg.pdf.render_width;
    let max_results = cfg.general.max_results;
    let log_scores = cfg.debug.log_scores;
    let content_cfg = cfg.content;

    // Open content index early (fast — just opens/creates the SQLite file).
    let content_index: Option<Arc<content_index::ContentIndex>> = if content_cfg.enabled {
        match content_index::ContentIndex::open() {
            Ok(idx) => Some(Arc::new(idx)),
            Err(e) => {
                eprintln!("[content] failed to open index: {e}");
                None
            }
        }
    } else {
        None
    };

    let registry: Registry = Arc::new(RwLock::new(providers::PluginRegistry::new(max_results)));
    let bg_registry = Arc::clone(&registry);

    let frecency_state: FrecencyState = if frecency_cfg.enabled {
        match frecency::FrecencyStore::open(frecency_cfg.half_life_days) {
            Ok(store) => {
                let arc = Arc::new(store);
                registry
                    .write()
                    .unwrap()
                    .set_frecency(Arc::clone(&arc), frecency_cfg.weight);
                Some(arc)
            }
            Err(e) => {
                eprintln!("[frecency] failed to open DB: {e} — frecency disabled");
                None
            }
        }
    } else {
        None
    };

    tauri::Builder::default()
        .manage(registry)
        .manage(frecency_state)
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            // Build reindex closure here so it can capture the AppHandle for progress events.
            let reindex_fn: Option<Arc<dyn Fn() + Send + Sync>> =
                content_index.as_ref().map(|idx| {
                    let idx = Arc::clone(idx);
                    let cc = content_cfg.clone();
                    let h = app.handle().clone();
                    Arc::new(move || {
                        eprintln!("[content] reindex requested");
                        idx.clear().ok();
                        let idx2 = Arc::clone(&idx);
                        let cc2 = cc.clone();
                        let h2 = h.clone();
                        std::thread::spawn(move || {
                            content_index::run_content_indexer(
                                idx2,
                                &cc2,
                                Some(make_progress_cb(h2)),
                            );
                        });
                    }) as Arc<dyn Fn() + Send + Sync>
                });

            start_socket_listener(app.handle().clone(), reindex_fn);

            providers::files::setup(app.handle(), pdf_render_width);
            providers::timer::setup(app.handle(), &bg_registry);

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
            if providers_cfg.dict {
                let dict_provider = providers::dict::DictProvider::new();
                if dict_provider.available {
                    bg_registry.write().unwrap().register(dict_provider);
                }
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if providers_cfg.files {
                    let file_provider =
                        providers::files::FileProvider::new(&files_cfg, &search_cfg, log_scores);
                    bg_registry.write().unwrap().register(file_provider);
                }
                if providers_cfg.recent {
                    let recent_provider = providers::recent::RecentProvider::new(
                        &recent_cfg,
                        &search_cfg,
                        log_scores,
                    );
                    bg_registry.write().unwrap().register(recent_provider);
                }
                if providers_cfg.apps {
                    let app_provider =
                        providers::apps::AppProvider::new(&search_cfg, log_scores);
                    bg_registry.write().unwrap().register(app_provider);
                }

                // Register ContentProvider immediately so existing index is searchable,
                // then run the indexer in a sub-thread so it doesn't block apps-ready.
                if let Some(idx) = content_index {
                    bg_registry
                        .write()
                        .unwrap()
                        .register(providers::content::ContentProvider::new(Arc::clone(&idx)));
                    let cc = content_cfg.clone();
                    let h = handle.clone();
                    std::thread::spawn(move || {
                        content_index::run_content_indexer(
                            idx,
                            &cc,
                            Some(make_progress_cb(h)),
                        );
                    });
                }

                APPS_READY.store(true, Ordering::Release);
                let _ = handle.emit("apps-ready", ());
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Core
            search,
            launch_app,
            hide_window,
            is_apps_ready,
            // Timer provider
            providers::timer::create_timer,
            providers::timer::stop_timer,
            // Files provider
            providers::files::render_pdf_page,
            providers::files::read_text_preview,
            providers::files::render_image_preview,
            providers::files::list_folder,
            // Clipboard provider
            providers::clipboard::paste_clipboard,
            providers::clipboard::decode_clipboard_entry,
            // Dict provider
            providers::dict::get_dict_definitions,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

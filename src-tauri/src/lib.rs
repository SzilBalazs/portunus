mod config;
mod content_index;
mod frecency;
mod providers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
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
    reload_fn: Arc<dyn Fn() + Send + Sync>,
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
            } else if cmd == "reload-config" {
                let f = Arc::clone(&reload_fn);
                std::thread::spawn(move || f());
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

// ── hot-reload ────────────────────────────────────────────────────────────────

fn rebuild_providers(
    new_cfg: &config::Config,
    old_cfg: &config::Config,
    shared: &config::SharedConfig,
    registry: &Registry,
    content_state: &Arc<Mutex<Option<Arc<content_index::ContentIndex>>>>,
    progress_cb: &Arc<dyn Fn(usize, usize) + Send + Sync>,
) {
    // Update per-search scalars instantly (no rebuild needed).
    shared.write().unwrap().update_from(new_cfg);

    // Update registry-level settings (max_results, frecency_weight).
    {
        let mut reg = registry.write().unwrap();
        reg.update_settings(new_cfg.general.max_results, new_cfg.frecency.weight);
    }

    // ── Selectively rebuild index-backed providers ────────────────────────────

    if new_cfg.files != old_cfg.files || new_cfg.providers.files != old_cfg.providers.files {
        let files_cfg = new_cfg.files.clone();
        let enabled = new_cfg.providers.files;
        let shared2 = Arc::clone(shared);
        let reg2 = Arc::clone(registry);
        std::thread::spawn(move || {
            let new: Option<Box<dyn providers::Provider>> = if enabled {
                Some(Box::new(providers::files::FileProvider::new(&files_cfg, shared2)))
            } else {
                None
            };
            reg2.write().unwrap().replace("files", new);
            eprintln!("[config] files provider rebuilt");
        });
    }

    if new_cfg.recent != old_cfg.recent || new_cfg.providers.recent != old_cfg.providers.recent {
        let recent_cfg = new_cfg.recent.clone();
        let enabled = new_cfg.providers.recent;
        let shared2 = Arc::clone(shared);
        let reg2 = Arc::clone(registry);
        std::thread::spawn(move || {
            let new: Option<Box<dyn providers::Provider>> = if enabled {
                Some(Box::new(providers::recent::RecentProvider::new(&recent_cfg, shared2)))
            } else {
                None
            };
            reg2.write().unwrap().replace("recent", new);
            eprintln!("[config] recent provider rebuilt");
        });
    }

    if new_cfg.providers.apps != old_cfg.providers.apps {
        let enabled = new_cfg.providers.apps;
        let shared2 = Arc::clone(shared);
        let reg2 = Arc::clone(registry);
        std::thread::spawn(move || {
            let new: Option<Box<dyn providers::Provider>> = if enabled {
                Some(Box::new(providers::apps::AppProvider::new(shared2)))
            } else {
                None
            };
            reg2.write().unwrap().replace("apps", new);
            eprintln!("[config] apps provider rebuilt");
        });
    }

    // ── Cheap providers: toggle under write lock directly ─────────────────────

    if new_cfg.providers.calc != old_cfg.providers.calc {
        let mut reg = registry.write().unwrap();
        if new_cfg.providers.calc {
            reg.register(providers::calc::CalcProvider);
            eprintln!("[config] calc provider enabled");
        } else {
            reg.replace("calc", None);
            eprintln!("[config] calc provider disabled");
        }
    }

    if new_cfg.providers.dict != old_cfg.providers.dict {
        let mut reg = registry.write().unwrap();
        if new_cfg.providers.dict {
            let p = providers::dict::DictProvider::new();
            if p.available {
                reg.register(p);
            }
            eprintln!("[config] dict provider enabled");
        } else {
            reg.replace("dict", None);
            eprintln!("[config] dict provider disabled");
        }
    }

    if new_cfg.content != old_cfg.content {
        let new_content_cfg = new_cfg.content.clone();
        let old_content_cfg = old_cfg.content.clone();
        let reg2 = Arc::clone(registry);
        let ci_state = Arc::clone(content_state);
        let cb = Arc::clone(progress_cb);
        std::thread::spawn(move || {
            // Hold the lock for the full operation (register → index) so
            // two rapid config saves can't race each other on the same DB tables.
            let mut guard = ci_state.lock().unwrap();
            if new_content_cfg.enabled {
                let idx = match guard.as_ref() {
                    Some(existing) => Arc::clone(existing),
                    None => match content_index::ContentIndex::open() {
                        Ok(idx) => {
                            let arc = Arc::new(idx);
                            *guard = Some(Arc::clone(&arc));
                            arc
                        }
                        Err(e) => {
                            eprintln!("[content] failed to open index: {e}");
                            return;
                        }
                    },
                };
                // OCR settings change the extracted text without touching mtime/size,
                // so the incremental check would wrongly skip affected files. All other
                // config changes (dirs, extensions, max_file_bytes) are handled correctly
                // by run_content_indexer's mtime+size check and remove_stale.
                let ocr_changed = old_content_cfg.ocr_images != new_content_cfg.ocr_images
                    || old_content_cfg.ocr_pdf_fallback != new_content_cfg.ocr_pdf_fallback
                    || old_content_cfg.ocr_language != new_content_cfg.ocr_language;
                if ocr_changed {
                    idx.clear().ok();
                }
                reg2.write().unwrap().replace(
                    "content",
                    Some(Box::new(providers::content::ContentProvider::new(Arc::clone(&idx)))),
                );
                content_index::run_content_indexer(idx, &new_content_cfg, Some(cb));
                eprintln!("[content] reindex complete");
            } else {
                *guard = None;
                reg2.write().unwrap().replace("content", None);
                eprintln!("[content] content provider disabled");
            }
        });
    }

    eprintln!("[config] reload complete");
}

fn start_config_watcher(
    shared: config::SharedConfig,
    registry: Registry,
    last_cfg: Arc<Mutex<config::Config>>,
    content_state: Arc<Mutex<Option<Arc<content_index::ContentIndex>>>>,
    progress_cb: Arc<dyn Fn(usize, usize) + Send + Sync>,
) {
    use notify_debouncer_full::{new_debouncer, notify::RecursiveMode, notify::Watcher};
    use std::time::Duration;

    let config_dir = {
        let config_home = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            format!("{home}/.config")
        });
        std::path::PathBuf::from(config_home).join("portunus")
    };

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();

        // Watch the DIRECTORY (not the file directly) because many editors save via
        // atomic rename: write temp file → rename over target. Watching the directory
        // catches IN_MOVED_TO which inotify fires on rename.
        let debouncer = loop {
            let tx2 = tx.clone();
            match new_debouncer(Duration::from_millis(500), None, move |res| {
                let _ = tx2.send(res);
            }) {
                Ok(mut d) => {
                    if config_dir.exists() {
                        if d.watcher().watch(&config_dir, RecursiveMode::NonRecursive).is_ok() {
                            break d;
                        }
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
                Err(e) => {
                    eprintln!("[config-watcher] failed to create debouncer: {e}");
                    return;
                }
            }
        };
        let _ = debouncer; // keep alive

        for res in rx {
            let events = match res {
                Ok(evs) => evs,
                Err(errs) => {
                    for e in errs {
                        eprintln!("[config-watcher] {e}");
                    }
                    continue;
                }
            };

            let touched = events.iter().any(|e| {
                e.event.paths.iter().any(|p| {
                    p.file_name().and_then(|n| n.to_str()) == Some("config.toml")
                })
            });
            if !touched {
                continue;
            }

            eprintln!("[config] change detected, reloading…");
            let new_cfg = config::Config::load();
            let mut last = last_cfg.lock().unwrap();
            rebuild_providers(&new_cfg, &last, &shared, &registry, &content_state, &progress_cb);
            *last = new_cfg;
        }
    });
}

// ── entry point ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("portunus — macOS Spotlight-style launcher for Linux

USAGE:
  portunus [FLAG]

FLAGS:
  --show            Show the launcher window (signals running instance)
  --clipboard       Show the launcher pre-filled with \"clipboard\"
  --reindex         Rebuild the content search index
  --reload-config   Reload config from file without restarting
  --help, -h        Show this help message");
        return;
    }

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
    if std::env::args().any(|a| a == "--reload-config") {
        if !try_signal_running("reload-config") {
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

    let shared_config: config::SharedConfig = Arc::new(RwLock::new(
        config::SharedSearchConfig::from_config(&cfg),
    ));

    let frecency_cfg = cfg.frecency.clone();
    let files_cfg = cfg.files.clone();
    let recent_cfg = cfg.recent.clone();
    let providers_cfg = cfg.providers.clone();
    let max_results = cfg.general.max_results;
    let content_cfg = cfg.content.clone();

    // last_cfg is shared between the config watcher and the socket reload_fn.
    let last_cfg: Arc<Mutex<config::Config>> = Arc::new(Mutex::new(cfg.clone()));

    // Open content index early (fast — just opens/creates the SQLite file).
    // Wrapped in Arc<Mutex<Option<...>>> so rebuild_providers can open/replace it at runtime.
    let content_state: Arc<Mutex<Option<Arc<content_index::ContentIndex>>>> =
        Arc::new(Mutex::new(if content_cfg.enabled {
            match content_index::ContentIndex::open() {
                Ok(idx) => Some(Arc::new(idx)),
                Err(e) => {
                    eprintln!("[content] failed to open index: {e}");
                    None
                }
            }
        } else {
            None
        }));

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

            // progress_cb is built once and shared by reindex_fn, reload_fn, and the watcher.
            let progress_cb: Arc<dyn Fn(usize, usize) + Send + Sync> =
                make_progress_cb(app.handle().clone());

            // reindex_fn: used by the --reindex socket command.
            let reindex_ci = Arc::clone(&content_state);
            let reindex_cc = content_cfg.clone();
            let reindex_cb = Arc::clone(&progress_cb);
            let reindex_fn: Option<Arc<dyn Fn() + Send + Sync>> = Some(Arc::new(move || {
                // Hold content_state for the full clear+index to serialise with config-triggered rebuilds.
                // reindex_fn is always called inside a spawned thread (socket handler), so blocking is fine.
                let guard = reindex_ci.lock().unwrap();
                if let Some(idx) = guard.as_ref().map(Arc::clone) {
                    eprintln!("[content] reindex requested");
                    idx.clear().ok();
                    content_index::run_content_indexer(idx, &reindex_cc, Some(Arc::clone(&reindex_cb)));
                    eprintln!("[content] reindex complete");
                } else {
                    eprintln!("[content] reindex requested but content indexing is disabled");
                }
                // guard drops here, releasing the lock
            }));

            // Build reload_fn closure shared between socket listener and config watcher.
            let reload_shared = Arc::clone(&shared_config);
            let reload_registry = Arc::clone(&bg_registry);
            let reload_last = Arc::clone(&last_cfg);
            let reload_ci = Arc::clone(&content_state);
            let reload_cb = Arc::clone(&progress_cb);
            let reload_fn: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let new_cfg = config::Config::load();
                let mut last = reload_last.lock().unwrap();
                rebuild_providers(
                    &new_cfg,
                    &last,
                    &reload_shared,
                    &reload_registry,
                    &reload_ci,
                    &reload_cb,
                );
                *last = new_cfg;
            });

            start_socket_listener(app.handle().clone(), reindex_fn, Arc::clone(&reload_fn));
            start_config_watcher(
                Arc::clone(&shared_config),
                Arc::clone(&bg_registry),
                Arc::clone(&last_cfg),
                Arc::clone(&content_state),
                Arc::clone(&progress_cb),
            );

            providers::files::setup(app.handle());
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
            let shared_bg = Arc::clone(&shared_config);
            let startup_ci = Arc::clone(&content_state);
            let startup_cb = Arc::clone(&progress_cb);
            std::thread::spawn(move || {
                if providers_cfg.files {
                    let file_provider =
                        providers::files::FileProvider::new(&files_cfg, Arc::clone(&shared_bg));
                    bg_registry.write().unwrap().register(file_provider);
                }
                if providers_cfg.recent {
                    let recent_provider = providers::recent::RecentProvider::new(
                        &recent_cfg,
                        Arc::clone(&shared_bg),
                    );
                    bg_registry.write().unwrap().register(recent_provider);
                }
                if providers_cfg.apps {
                    let app_provider =
                        providers::apps::AppProvider::new(Arc::clone(&shared_bg));
                    bg_registry.write().unwrap().register(app_provider);
                }

                // Register ContentProvider immediately so existing index is searchable,
                // then run the indexer in a sub-thread so it doesn't block apps-ready.
                let initial_idx = startup_ci.lock().unwrap().as_ref().map(Arc::clone);
                if let Some(idx) = initial_idx {
                    bg_registry
                        .write()
                        .unwrap()
                        .register(providers::content::ContentProvider::new(Arc::clone(&idx)));
                    let cc = content_cfg.clone();
                    let cb = Arc::clone(&startup_cb);
                    std::thread::spawn(move || {
                        content_index::run_content_indexer(idx, &cc, Some(cb));
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

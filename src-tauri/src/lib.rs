mod cli;
mod config;
mod content_index;
mod frecency;
mod ipc;
mod preview;
mod provider_reload;
mod providers;
mod watcher;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{Emitter, Manager};

static APPS_READY: AtomicBool = AtomicBool::new(false);

pub(crate) type Registry = Arc<RwLock<providers::PluginRegistry>>;
pub(crate) type FrecencyState = Option<Arc<frecency::FrecencyStore>>;
pub(crate) type ContentWatcherTx =
    Arc<Mutex<Option<std::sync::mpsc::Sender<config::ContentConfig>>>>;
pub(crate) type FileWatcherTx =
    Arc<Mutex<Option<std::sync::mpsc::Sender<config::FilesConfig>>>>;
pub(crate) type SharedFileEntries = Arc<RwLock<Vec<providers::files::FileEntry>>>;

#[derive(serde::Serialize, Clone)]
struct ContentIndexProgress {
    indexed: usize,
    total: usize,
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

// ── entry point ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if cli::handle_cli_args() {
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

    // Populated once the content watcher thread starts; None until then.
    let content_watcher_tx: ContentWatcherTx = Arc::new(Mutex::new(None));
    // Populated once the file watcher thread starts; None until then.
    let file_watcher_tx: FileWatcherTx = Arc::new(Mutex::new(None));
    // Shared between FileProvider and the file watcher; populated in the startup thread.
    let file_entries: SharedFileEntries = Arc::new(RwLock::new(vec![]));

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

            // notify_cb fires "search-invalidated" so the frontend re-runs the current query.
            let notify_handle = app.handle().clone();
            let notify_cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let _ = notify_handle.emit("search-invalidated", ());
            });

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
            let reload_watcher_tx = Arc::clone(&content_watcher_tx);
            let reload_notify = Arc::clone(&notify_cb);
            let reload_file_entries = Arc::clone(&file_entries);
            let reload_file_watcher_tx = Arc::clone(&file_watcher_tx);
            let reload_fn: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let new_cfg = config::Config::load();
                let mut last = reload_last.lock().unwrap();
                provider_reload::rebuild_providers(
                    &new_cfg,
                    &last,
                    &reload_shared,
                    &reload_registry,
                    &reload_ci,
                    &reload_cb,
                    &reload_watcher_tx,
                    &reload_notify,
                    &reload_file_entries,
                    &reload_file_watcher_tx,
                );
                *last = new_cfg;
            });

            ipc::start_socket_listener(app.handle().clone(), reindex_fn, Arc::clone(&reload_fn));
            watcher::start_config_watcher(
                Arc::clone(&shared_config),
                Arc::clone(&bg_registry),
                Arc::clone(&last_cfg),
                Arc::clone(&content_state),
                Arc::clone(&progress_cb),
                Arc::clone(&content_watcher_tx),
                Arc::clone(&notify_cb),
                Arc::clone(&file_entries),
                Arc::clone(&file_watcher_tx),
            );

            preview::setup(app.handle());
            providers::timer::setup(app.handle(), &bg_registry);

            if providers::clipboard::ClipboardProvider::is_available() {
                bg_registry
                    .write()
                    .unwrap()
                    .register(providers::clipboard::ClipboardProvider);
            }
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
            let startup_cb_notify = Arc::clone(&notify_cb);
            let startup_watcher_tx = Arc::clone(&content_watcher_tx);
            let startup_file_entries = Arc::clone(&file_entries);
            let startup_file_watcher_tx = Arc::clone(&file_watcher_tx);
            std::thread::spawn(move || {
                if providers_cfg.files {
                    let entries_vec = providers::files::FileProvider::walk_dirs(&files_cfg);
                    *startup_file_entries.write().unwrap() = entries_vec;
                    let file_provider = providers::files::FileProvider::with_entries(
                        Arc::clone(&startup_file_entries),
                        Arc::clone(&shared_bg),
                    );
                    bg_registry.write().unwrap().register(file_provider);
                    let tx = watcher::start_file_watcher(
                        Arc::clone(&startup_file_entries),
                        files_cfg.clone(),
                        Arc::clone(&startup_cb_notify),
                    );
                    *startup_file_watcher_tx.lock().unwrap() = Some(tx);
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

                // Start the filesystem watcher for live content index updates.
                if content_cfg.enabled {
                    let watcher_ci = Arc::clone(&startup_ci);
                    let watcher_notify = Arc::clone(&startup_cb_notify);
                    let tx = watcher::start_content_watcher(
                        watcher_ci,
                        content_cfg.clone(),
                        watcher_notify,
                        Arc::clone(&shared_bg),
                    );
                    *startup_watcher_tx.lock().unwrap() = Some(tx);
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
            // File preview
            preview::render_pdf_page,
            preview::read_text_preview,
            preview::render_image_preview,
            preview::list_folder,
            // Clipboard provider
            providers::clipboard::paste_clipboard,
            providers::clipboard::decode_clipboard_entry,
            // Dict provider
            providers::dict::get_dict_definitions,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

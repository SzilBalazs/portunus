mod cli;
mod config;
mod content_index;
mod frecency;
mod ipc;
mod preview;
mod provider_reload;
mod providers;
mod util;
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
pub(crate) type ConfigState = Arc<Mutex<config::Config>>;
pub(crate) type ContentState = Arc<Mutex<Option<Arc<content_index::ContentIndex>>>>;

pub struct TriggerFullReindexFn(pub Arc<dyn Fn() + Send + Sync>);

#[derive(serde::Serialize, Clone)]
struct ContentIndexProgress {
    indexed: usize,
    total: usize,
}

#[tauri::command]
fn search(query: String, registry: tauri::State<'_, Registry>) -> Vec<providers::SearchResult> {
    registry.read().unwrap_or_else(|e| e.into_inner()).search(&query)
}

#[derive(serde::Serialize, Clone)]
struct DepStatus {
    /// Stable identifier the frontend can match against (e.g. "dict").
    id: &'static str,
    /// The underlying tool name shown to the user.
    label: &'static str,
    /// The feature that stops working when this dependency is missing.
    feature: &'static str,
    available: bool,
    /// Package the user should install to get this dependency.
    install_hint: &'static str,
}

/// Reports the runtime availability of optional system dependencies so the
/// Settings UI can warn the user instead of silently degrading. Each probe is
/// cheap (PATH lookup), except pdfium which attempts a library bind.
#[tauri::command]
fn check_dependencies() -> Vec<DepStatus> {
    use util::binary_in_path;
    vec![
        DepStatus {
            id: "cliphist",
            label: "cliphist",
            feature: "Clipboard history",
            available: binary_in_path("cliphist"),
            install_hint: "cliphist",
        },
        DepStatus {
            id: "wl-copy",
            label: "wl-copy",
            feature: "Clipboard paste",
            available: binary_in_path("wl-copy"),
            install_hint: "wl-clipboard",
        },
        DepStatus {
            id: "dict",
            label: "dict",
            feature: "Dictionary lookups",
            available: binary_in_path("dict"),
            install_hint: "dictd",
        },
        DepStatus {
            id: "poppler",
            label: "pdftotext",
            feature: "PDF content indexing",
            available: binary_in_path("pdftotext"),
            install_hint: "poppler",
        },
        DepStatus {
            id: "tesseract",
            label: "tesseract",
            feature: "OCR (images & scanned PDFs)",
            available: binary_in_path("tesseract"),
            install_hint: "tesseract + tesseract-data-eng",
        },
        DepStatus {
            id: "pdfium",
            label: "libpdfium",
            feature: "PDF preview",
            available: preview::pdfium_available(),
            install_hint: "pdfium (e.g. pdfium-bin)",
        },
    ]
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

#[tauri::command]
fn get_config(state: tauri::State<ConfigState>) -> config::Config {
    state.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(config: config::Config, state: tauri::State<ConfigState>) -> Result<(), String> {
    let result = config.save();
    if result.is_ok() {
        *state.lock().unwrap() = config;
    }
    result
}

#[tauri::command]
fn open_settings_window(app: tauri::AppHandle, section: Option<String>) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        if let Some(s) = section {
            let _ = win.emit("navigate-to-section", s);
        }
    }
}

#[tauri::command]
fn trigger_full_reindex(reindex: tauri::State<'_, TriggerFullReindexFn>) {
    let f = Arc::clone(&reindex.0);
    std::thread::spawn(move || f());
}

/// Reports whether the content index currently holds no documents.
/// Used by the settings UI to decide whether enabling content search is a
/// (potentially slow) first-time build that needs confirmation, or a cheap
/// incremental run over an already-populated index.
#[tauri::command]
fn is_content_index_empty(state: tauri::State<'_, ContentState>) -> bool {
    // try_lock so we never block the UI behind an in-progress reindex (which
    // holds this lock for its full duration). A live index is only contended
    // while content is enabled — exactly the case where the answer is moot.
    if let Ok(guard) = state.try_lock() {
        if let Some(idx) = guard.as_ref() {
            return idx.is_empty();
        }
    }
    // No open index (content disabled) — count rows in the on-disk DB directly.
    // `open` is resilient: a corrupt DB is recreated empty rather than surfacing an
    // error, so a `true` here means genuinely empty (the first-build prompt is then
    // accurate). A residual Err is a hard I/O/permission failure with no rebuild
    // recovery, so we log it loudly instead of silently pretending the index is empty.
    match content_index::ContentIndex::open() {
        Ok(idx) => idx.is_empty(),
        Err(e) => {
            eprintln!("[content] is_content_index_empty: failed to open index: {e}");
            true
        }
    }
}

/// Drains the last config parse error (set by `Config::load`) so the settings UI
/// can show a one-time banner. Returns None when the config loaded cleanly.
#[tauri::command]
fn take_config_error() -> Option<String> {
    config::LAST_LOAD_ERROR
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
}

/// Full clear-and-rebuild of the content index using the current on-disk config.
/// Shared by the `--reindex` socket command and the settings "Apply & Reindex"
/// trigger. Loads the config fresh so it always reflects the latest saved state,
/// opens the index if it isn't open yet (first-time enable), and re-registers the
/// provider so results are searchable. Holds the content lock for the whole run
/// so concurrent config-triggered rebuilds serialise rather than racing the DB.
fn run_full_reindex(
    content_state: &ContentState,
    registry: &Registry,
    progress_cb: &Arc<dyn Fn(usize, usize) + Send + Sync>,
    notify_cb: &Arc<dyn Fn() + Send + Sync>,
) {
    let cfg = config::Config::load();
    if !cfg.content.enabled {
        eprintln!("[content] reindex requested but content indexing is disabled");
        return;
    }
    let mut guard = content_state.lock().unwrap();
    let idx = match guard.as_ref() {
        Some(idx) => Arc::clone(idx),
        None => match content_index::ContentIndex::open() {
            Ok(idx) => {
                let arc = Arc::new(idx);
                *guard = Some(Arc::clone(&arc));
                arc
            }
            Err(e) => {
                eprintln!("[content] reindex: failed to open index: {e}");
                return;
            }
        },
    };
    registry.write().unwrap().replace(
        "content",
        Some(Box::new(providers::content::ContentProvider::new(Arc::clone(&idx)))),
    );
    eprintln!("[content] full reindex started");
    idx.clear().ok();
    content_index::run_content_indexer(idx, &cfg.content, Some(Arc::clone(progress_cb)));
    eprintln!("[content] full reindex complete");
    notify_cb();
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

    // last_cfg is used by the config watcher and reload_fn as the diff baseline.
    // Must be a separate Arc from config_state so that save_config (which updates
    // config_state) doesn't also advance last_cfg, which would cause the watcher
    // to see no diff and skip provider rebuilds.
    let last_cfg: Arc<Mutex<config::Config>> = Arc::new(Mutex::new(cfg.clone()));
    // config_state is the frontend-visible config (read by get_config, written by save_config).
    let config_state: ConfigState = Arc::new(Mutex::new(cfg.clone()));

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
        .manage(config_state)
        .manage(Arc::clone(&content_state))
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            // Pre-create the settings window hidden so it's ready instantly when opened.
            let _ = tauri::WebviewWindowBuilder::new(
                app,
                "settings",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Portunus Settings")
            .inner_size(800.0, 560.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(false)
            .skip_taskbar(false)
            .center()
            .visible(false)
            .build();

            // progress_cb is built once and shared by reindex_fn, reload_fn, and the watcher.
            let progress_cb: Arc<dyn Fn(usize, usize) + Send + Sync> =
                make_progress_cb(app.handle().clone());

            // notify_cb fires "search-invalidated" so the frontend re-runs the current query.
            let notify_handle = app.handle().clone();
            let notify_cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let _ = notify_handle.emit("search-invalidated", ());
            });

            // reindex_fn: used by the --reindex socket command. Always called from
            // a spawned thread (socket handler), so blocking on the content lock is fine.
            let reindex_ci = Arc::clone(&content_state);
            let reindex_reg = Arc::clone(&bg_registry);
            let reindex_cb = Arc::clone(&progress_cb);
            let reindex_notify = Arc::clone(&notify_cb);
            let reindex_fn: Option<Arc<dyn Fn() + Send + Sync>> = Some(Arc::new(move || {
                run_full_reindex(&reindex_ci, &reindex_reg, &reindex_cb, &reindex_notify);
            }));

            // trigger_reindex_fn: called by the trigger_full_reindex Tauri command
            // (settings "Apply & Reindex"). Identical full clear+rebuild as --reindex.
            let tri_ci = Arc::clone(&content_state);
            let tri_reg = Arc::clone(&bg_registry);
            let tri_cb = Arc::clone(&progress_cb);
            let tri_notify = Arc::clone(&notify_cb);
            let trigger_reindex_fn: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                run_full_reindex(&tri_ci, &tri_reg, &tri_cb, &tri_notify);
            });
            app.manage(TriggerFullReindexFn(Arc::clone(&trigger_reindex_fn)));

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

            // Start watchers before start_config_watcher so that file_watcher_tx /
            // content_watcher_tx are populated before any config-change event can fire.
            // (Previously both were started inside the background startup thread, which
            // created a race where a config change during startup found tx = None.)
            if providers_cfg.files {
                let tx = watcher::start_file_watcher(
                    Arc::clone(&file_entries),
                    files_cfg.clone(),
                    Arc::clone(&notify_cb),
                );
                *file_watcher_tx.lock().unwrap() = Some(tx);
            }
            // Always start the content watcher, even if content is currently disabled,
            // so that re-enabling later will immediately receive filesystem events.
            {
                let tx = watcher::start_content_watcher(
                    Arc::clone(&content_state),
                    content_cfg.clone(),
                    Arc::clone(&notify_cb),
                    Arc::clone(&shared_config),
                );
                *content_watcher_tx.lock().unwrap() = Some(tx);
            }

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
                } else {
                    eprintln!("[portunus] dict: `dict` not found — dictionary provider disabled");
                }
            }

            let handle = app.handle().clone();
            let shared_bg = Arc::clone(&shared_config);
            let startup_ci = Arc::clone(&content_state);
            let startup_cb = Arc::clone(&progress_cb);
            let startup_file_entries = Arc::clone(&file_entries);
            std::thread::spawn(move || {
                if providers_cfg.files {
                    let entries_vec = providers::files::FileProvider::walk_dirs(&files_cfg);
                    *startup_file_entries.write().unwrap() = entries_vec;
                    let file_provider = providers::files::FileProvider::with_entries(
                        Arc::clone(&startup_file_entries),
                        Arc::clone(&shared_bg),
                    );
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
            get_config,
            save_config,
            open_settings_window,
            trigger_full_reindex,
            is_content_index_empty,
            check_dependencies,
            take_config_error,
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

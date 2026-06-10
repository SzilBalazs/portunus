mod cli;
mod clipboard_ocr;
mod config;
mod content_index;
mod extensions;
mod frecency;
mod ipc;
mod layer_shell;
mod office;
mod preview;
mod provider_reload;
mod providers;
mod runtime_assets;
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
pub(crate) type ClipboardOcrState = Option<Arc<clipboard_ocr::ClipboardOcrStore>>;

/// The `bool` is `full`: `true` clears the index before rebuilding (needed only
/// when OCR settings change, which invalidates already-cached text); `false`
/// runs a cache-preserving incremental pass (dir/extension/depth/size edits).
pub struct TriggerFullReindexFn(pub Arc<dyn Fn(bool) + Send + Sync>);

/// Clone of the config watcher's diff baseline (`last_cfg`). The Apply path
/// advances its `content` so the file-watcher, which fires on the same save,
/// sees no content diff and skips a redundant second reindex. See
/// `trigger_full_reindex`.
pub struct WatcherBaseline(pub Arc<Mutex<config::Config>>);

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
            id: "wtype",
            label: "wtype",
            feature: "Smart paste (auto Ctrl+V)",
            available: binary_in_path("wtype"),
            install_hint: "wtype",
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
            available: runtime_assets::poppler_bundled() || binary_in_path("pdftotext"),
            install_hint: "poppler",
        },
        DepStatus {
            id: "tesseract",
            label: "tesseract",
            feature: "OCR (images & scanned PDFs)",
            // OCR is linked in via leptess; it needs language data, which is
            // bundled in packaged builds or provided by a system tesseract.
            available: runtime_assets::tessdata_path().is_some() || binary_in_path("tesseract"),
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

/// Reveal a file in the system file manager with the file itself selected/focused.
/// Uses the freedesktop `org.freedesktop.FileManager1.ShowItems` D-Bus method, which
/// is honored by Nautilus, Dolphin, Nemo, Thunar, etc. Falls back to opening the
/// parent directory with `xdg-open` if the D-Bus call cannot be made.
#[tauri::command]
fn reveal_file(app: tauri::AppHandle, path: String) {
    use std::os::unix::process::CommandExt;

    let uri = format!("file://{}", encode_path_uri(&path));

    let dbus = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
            &format!("array:string:{uri}"),
            "string:",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    let ok = matches!(dbus, Ok(status) if status.success());

    if !ok {
        // Fallback: open the parent directory (no file selection).
        let parent = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        let _ = std::process::Command::new("xdg-open")
            .arg(parent)
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

/// Percent-encode a filesystem path for use in a `file://` URI. Encodes everything
/// outside the RFC 3986 unreserved set plus `/`, which stays as the path separator.
fn encode_path_uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
    util::lock(&state).clone()
}

/// Read the external matugen theme CSS (`<config_dir>/matugen.css`) for the
/// `matugen` theme. Returns `None` if the file is absent/unreadable, in which
/// case the frontend leaves the `[data-theme="matugen"]` selector unmatched and
/// vars fall back to the App.css `:root` defaults.
#[tauri::command]
fn get_custom_theme_css() -> Option<String> {
    std::fs::read_to_string(config::config_dir().join("matugen.css")).ok()
}

#[tauri::command]
fn save_config(config: config::Config, state: tauri::State<ConfigState>) -> Result<(), String> {
    let result = config.save();
    if result.is_ok() {
        *util::lock(&state) = config;
    }
    result
}

#[tauri::command]
fn open_settings_window(app: tauri::AppHandle, section: Option<String>) {
    // Hide the launcher so it doesn't sit on top of (or steal layer-shell
    // keyboard focus from) the settings window.
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.hide();
    }
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        if let Some(s) = section {
            let _ = win.emit("navigate-to-section", s);
        }
    }
}

#[tauri::command]
fn trigger_full_reindex(
    full: bool,
    reindex: tauri::State<'_, TriggerFullReindexFn>,
    config: tauri::State<'_, ConfigState>,
    baseline: tauri::State<'_, WatcherBaseline>,
) {
    // The caller (Settings "Apply") just persisted the config via save_config, which
    // also wakes the config-file watcher → provider_reload. Advance the watcher's
    // diff baseline to the freshly-saved content *now* (synchronously, before the
    // 500ms-debounced watcher event can fire) so it sees no content diff and skips
    // a redundant second reindex. This explicit trigger is then the sole content
    // indexer. Only `.content` is touched, so files/apps diffs still apply.
    {
        let current = util::lock(&config).content.clone();
        util::lock(&baseline.0).content = current;
    }
    let f = Arc::clone(&reindex.0);
    std::thread::spawn(move || f(full));
}

/// Pre-index estimate for a single directory: exact file counts plus a coarse
/// min/max time range. The OCR/size/threads flags are passed in from the live
/// (possibly still-staged) settings UI rather than read from on-disk config, so
/// the estimate reflects unapplied toggles too.
#[tauri::command]
fn estimate_dir_index(
    path: String,
    depth: usize,
    extensions: Option<Vec<String>>,
    max_file_bytes: u64,
    ocr_images: bool,
    ocr_pdf_fallback: bool,
    threads: usize,
) -> content_index::DirEstimate {
    let cfg = config::ContentConfig {
        max_file_bytes,
        ocr_images,
        ocr_pdf_fallback,
        threads,
        ..Default::default()
    };
    content_index::estimate_dir(&path, depth, extensions.as_ref(), &cfg)
}

/// Reports whether the content index currently holds no documents.
/// Used by the settings UI to decide whether enabling content search is a
/// (potentially slow) first-time build that needs confirmation, or a cheap
/// incremental run over an already-populated index.
#[tauri::command]
fn is_content_index_empty(state: tauri::State<'_, ContentState>) -> bool {
    // try_lock so we never block the UI behind an in-progress reindex (which
    // holds this lock for its full duration). A live index is only contended
    // while content is enabled - exactly the case where the answer is moot.
    if let Ok(guard) = state.try_lock() {
        if let Some(idx) = guard.as_ref() {
            return idx.is_empty();
        }
    }
    // No open index (content disabled) - count rows in the on-disk DB directly.
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

/// Rebuilds the content index using the current on-disk config. Shared by the
/// `--reindex` socket command and the settings "Apply & Reindex" trigger. Loads
/// the config fresh so it always reflects the latest saved state, opens the index
/// if it isn't open yet (first-time enable), and re-registers the provider so
/// results are searchable. Holds the content lock for the whole run so concurrent
/// config-triggered rebuilds serialise rather than racing the DB.
///
/// `full` controls cache reuse: `true` clears the index first (a hard rebuild -
/// the `--reindex` path and any OCR-settings change, which invalidate cached
/// text). `false` keeps existing rows so `run_content_indexer`'s mtime-skip and
/// `remove_stale` apply only the delta - the cache-preserving path for dir /
/// extension / depth / size edits.
fn run_full_reindex(
    content_state: &ContentState,
    registry: &Registry,
    progress_cb: &Arc<dyn Fn(usize, usize) + Send + Sync>,
    notify_cb: &Arc<dyn Fn() + Send + Sync>,
    full: bool,
) {
    let cfg = config::Config::load();
    if !cfg.content.enabled {
        eprintln!("[content] reindex requested but content indexing is disabled");
        return;
    }
    // Coalesce overlapping reindex triggers so only one drives the progress bar.
    let _reindex_guard = match content_index::ReindexGuard::acquire() {
        Some(g) => g,
        None => {
            eprintln!("[content] reindex already in progress; skipping");
            return;
        }
    };
    let mut guard = util::lock(content_state);
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
    if full {
        eprintln!("[content] full reindex started (clearing index)");
        idx.clear().ok();
    } else {
        eprintln!("[content] incremental reindex started (cache preserved)");
    }
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

    // The AppImage's linuxdeploy GTK hook hard-codes `GDK_BACKEND=x11`, which
    // drops the window onto XWayland and loses the compositor's fractional/HiDPI
    // scaling (the window renders tiny). Portunus is a Wayland app, so when we
    // are running from the AppImage inside a Wayland session prefer the Wayland
    // backend. Must happen before any GTK init. `PORTUNUS_FORCE_X11` opts out.
    if std::env::var_os("APPDIR").is_some()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("PORTUNUS_FORCE_X11").is_none()
    {
        std::env::set_var("GDK_BACKEND", "wayland");
    }

    let cfg = config::Config::load();

    let shared_config: config::SharedConfig = Arc::new(RwLock::new(
        config::SharedSearchConfig::from_config(&cfg),
    ));

    let frecency_cfg = cfg.frecency.clone();
    let files_cfg = cfg.files.clone();
    let providers_cfg = cfg.providers.clone();
    let dict_cfg = cfg.dict.clone();
    let max_results = cfg.general.max_results;
    let layer_shell_enabled = cfg.general.layer_shell;
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

    // Open content index early (fast - just opens/creates the SQLite file).
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

    // Extension KV store: opened once, shared by host functions, sync, and
    // the Tauri commands.
    let ext_kv: Arc<extensions::kv::ExtensionKv> =
        Arc::new(extensions::kv::ExtensionKv::open().unwrap_or_else(|e| {
            eprintln!("[extensions] failed to open kv store ({e}) - falling back to in-memory");
            extensions::kv::ExtensionKv::open_in_memory()
        }));
    let extensions_enabled = cfg.extensions.enabled.clone();

    // Clipboard OCR cache: opened unconditionally (cheap SQLite file). The
    // background OCR pass and clipboard_list read/write it; gated per-pass by
    // config.clipboard.ocr_images.
    let clipboard_ocr_state: ClipboardOcrState = match clipboard_ocr::ClipboardOcrStore::open() {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            eprintln!("[clipboard] failed to open OCR cache: {e} - image OCR search disabled");
            None
        }
    };

    let registry: Registry = Arc::new(RwLock::new(providers::PluginRegistry::new(max_results)));
    if dict_cfg.enabled {
        registry
            .write()
            .unwrap()
            .set_dict_fill(Some((dict_cfg.fill_threshold, dict_cfg.fill_max)));
    }
    let bg_registry = Arc::clone(&registry);

    let frecency_state: FrecencyState = if frecency_cfg.enabled {
        match frecency::FrecencyStore::open(frecency_cfg.half_life_days) {
            Ok(store) => {
                let arc = Arc::new(store);
                let history_max_bonus = (cfg.search.history_weight as f32 / 100.0) * 1_500_000.0;
                registry
                    .write()
                    .unwrap()
                    .set_frecency(Arc::clone(&arc), history_max_bonus);
                Some(arc)
            }
            Err(e) => {
                eprintln!("[frecency] failed to open DB: {e} - frecency disabled");
                None
            }
        }
    } else {
        None
    };

    tauri::Builder::default()
        .manage(registry)
        .manage(frecency_state.clone())
        .manage(config_state)
        .manage(Arc::clone(&content_state))
        .manage(clipboard_ocr_state)
        .manage(extensions::ExtensionKvState(Arc::clone(&ext_kv)))
        .setup(move |app| {
            // Resolve bundled native assets (pdfium, poppler, tessdata) before
            // any provider or preview path needs them.
            runtime_assets::init(app.path().resource_dir().ok());

            if let Some(window) = app.get_webview_window("main") {
                if layer_shell_enabled {
                    layer_shell::apply(&window);
                }
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
            // --reindex is a poweruser hard rebuild: always a full clear.
            let reindex_fn: Option<Arc<dyn Fn() + Send + Sync>> = Some(Arc::new(move || {
                run_full_reindex(&reindex_ci, &reindex_reg, &reindex_cb, &reindex_notify, true);
            }));

            // trigger_reindex_fn: called by the trigger_full_reindex Tauri command
            // (settings "Apply & Reindex"). `full` is chosen by the UI - true only
            // when OCR settings changed; otherwise an incremental, cache-preserving run.
            let tri_ci = Arc::clone(&content_state);
            let tri_reg = Arc::clone(&bg_registry);
            let tri_cb = Arc::clone(&progress_cb);
            let tri_notify = Arc::clone(&notify_cb);
            let trigger_reindex_fn: Arc<dyn Fn(bool) + Send + Sync> = Arc::new(move |full: bool| {
                run_full_reindex(&tri_ci, &tri_reg, &tri_cb, &tri_notify, full);
            });
            app.manage(TriggerFullReindexFn(Arc::clone(&trigger_reindex_fn)));
            // Share the watcher's diff baseline with trigger_full_reindex so the
            // Apply path can mark content as already-applied (see that command).
            app.manage(WatcherBaseline(Arc::clone(&last_cfg)));

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
            let reload_ext_kv = Arc::clone(&ext_kv);
            let reload_frecency = frecency_state.clone();
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
                    &reload_ext_kv,
                    &reload_frecency,
                );
                *last = new_cfg;
            });

            // --reload-extensions: force-rebuild every loaded extension so a
            // recompiled wasm is picked up - the extension author's hot-reload loop.
            let ext_reload_registry = Arc::clone(&bg_registry);
            let ext_reload_kv = Arc::clone(&ext_kv);
            let ext_reload_frecency = frecency_state.clone();
            let ext_reload_notify = Arc::clone(&notify_cb);
            let reload_extensions_fn: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let enabled = config::Config::load().extensions.enabled;
                extensions::sync(
                    &ext_reload_registry,
                    &enabled,
                    &ext_reload_kv,
                    &ext_reload_frecency,
                    true,
                    Some(Arc::clone(&ext_reload_notify)),
                );
                eprintln!("[extensions] reloaded");
                ext_reload_notify();
            });

            ipc::start_socket_listener(
                app.handle().clone(),
                reindex_fn,
                Arc::clone(&reload_fn),
                Arc::clone(&reload_extensions_fn),
            );

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
                Arc::clone(&ext_kv),
                frecency_state.clone(),
            );

            preview::setup(app.handle(), Arc::clone(&shared_config));
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
            let handle = app.handle().clone();
            let shared_bg = Arc::clone(&shared_config);
            let startup_ci = Arc::clone(&content_state);
            let startup_cb = Arc::clone(&progress_cb);
            let startup_file_entries = Arc::clone(&file_entries);
            let startup_ext_kv = Arc::clone(&ext_kv);
            let startup_frecency = frecency_state.clone();
            let startup_notify = Arc::clone(&notify_cb);

            // Interval scheduler for extensions with [background] refresh.
            extensions::start_refresh_scheduler(
                Arc::clone(&bg_registry),
                Arc::clone(&notify_cb),
            );

            std::thread::spawn(move || {
                // Built here, not in the setup closure: DictProvider::new() now
                // builds an embedded word index (BK-tree), too heavy for the
                // window-show path.
                if dict_cfg.enabled {
                    let dict_provider = providers::dict::DictProvider::new(&dict_cfg);
                    if dict_provider.available {
                        bg_registry.write().unwrap().register(dict_provider);
                    } else {
                        eprintln!("[portunus] dict: `dict` not found - dictionary provider disabled");
                    }
                }
                if providers_cfg.files {
                    providers::files::FileProvider::populate(&startup_file_entries, &files_cfg);
                    let file_provider = providers::files::FileProvider::with_entries(
                        Arc::clone(&startup_file_entries),
                        Arc::clone(&shared_bg),
                    );
                    bg_registry.write().unwrap().register(file_provider);
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

                // Extensions load after apps-ready: wasm compilation must never
                // delay the launcher becoming usable.
                extensions::sync(
                    &bg_registry,
                    &extensions_enabled,
                    &startup_ext_kv,
                    &startup_frecency,
                    false,
                    Some(startup_notify),
                );
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Core
            search,
            launch_app,
            reveal_file,
            hide_window,
            is_apps_ready,
            get_config,
            get_custom_theme_css,
            save_config,
            open_settings_window,
            trigger_full_reindex,
            estimate_dir_index,
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
            preview::read_office_preview,
            preview::read_spreadsheet_preview,
            // Clipboard provider
            providers::clipboard::paste_clipboard,
            providers::clipboard::decode_clipboard_entry,
            providers::clipboard::clipboard_list,
            providers::clipboard::index_clipboard_ocr,
            providers::clipboard::clipboard_delete,
            providers::clipboard::clipboard_capabilities,
            // Dict provider
            providers::dict::get_dict_definitions,
            // Extensions
            extensions::list_extensions,
            extensions::extension_activate,
            extensions::extension_preview,
            extensions::rescan_extensions,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

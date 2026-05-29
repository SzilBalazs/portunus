use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify_debouncer_full::{new_debouncer, notify::RecursiveMode, notify::Watcher, DebouncedEvent};

use crate::{config, content_index, provider_reload, providers, ContentWatcherTx, FileWatcherTx, Registry, SharedFileEntries};

/// Spawns a watcher thread that watches dirs derived from `initial_cfg`.
/// New dirs added by subsequent config updates (sent via the returned Sender) are added to the watch set.
/// `get_dirs` extracts the list of directories to watch from a config snapshot.
/// `on_events` is called for each debounced batch of filesystem events.
fn run_dir_watcher<Cfg>(
    initial_cfg: Cfg,
    get_dirs: impl Fn(&Cfg) -> Vec<PathBuf> + Send + 'static,
    mut on_events: impl FnMut(&[DebouncedEvent], &Cfg) + Send + 'static,
) -> std::sync::mpsc::Sender<Cfg>
where
    Cfg: Send + Clone + 'static,
{
    let (cfg_tx, cfg_rx) = std::sync::mpsc::channel::<Cfg>();

    std::thread::spawn(move || {
        let (ev_tx, ev_rx) = std::sync::mpsc::channel();
        let mut debouncer = match new_debouncer(Duration::from_millis(500), None, move |res| {
            let _ = ev_tx.send(res);
        }) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[watcher] failed to create debouncer: {e}");
                return;
            }
        };

        let mut current_cfg = initial_cfg;
        let mut watched_dirs: HashSet<PathBuf> = HashSet::new();

        // Watch dirs from the initial config.
        for dir in get_dirs(&current_cfg) {
            if dir.is_dir() && watched_dirs.insert(dir.clone()) {
                if let Err(e) = debouncer.watcher().watch(&dir, RecursiveMode::Recursive) {
                    eprintln!("[watcher] watch {:?}: {e}", dir);
                }
            }
        }

        loop {
            // Drain any pending config updates, adding newly-mentioned dirs to the watch set.
            while let Ok(new_cfg) = cfg_rx.try_recv() {
                for dir in get_dirs(&new_cfg) {
                    if dir.is_dir() && watched_dirs.insert(dir.clone()) {
                        if let Err(e) = debouncer.watcher().watch(&dir, RecursiveMode::Recursive) {
                            eprintln!("[watcher] watch {:?}: {e}", dir);
                        }
                    }
                }
                current_cfg = new_cfg;
            }

            match ev_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Ok(events)) => on_events(&events, &current_cfg),
                Ok(Err(errs)) => {
                    for e in errs {
                        eprintln!("[watcher] {e}");
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    cfg_tx
}

// ── Config file watcher ───────────────────────────────────────────────────────

pub fn start_config_watcher(
    shared: config::SharedConfig,
    registry: Registry,
    last_cfg: Arc<Mutex<config::Config>>,
    content_state: Arc<Mutex<Option<Arc<content_index::ContentIndex>>>>,
    progress_cb: Arc<dyn Fn(usize, usize) + Send + Sync>,
    content_watcher_tx: ContentWatcherTx,
    notify_cb: Arc<dyn Fn() + Send + Sync>,
    file_entries: SharedFileEntries,
    file_watcher_tx: FileWatcherTx,
) {
    use notify_debouncer_full::notify::Watcher as _;

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
        // The config dir normally exists by now (Config::load creates it at startup),
        // so this retry loop is a safety net. Bound it instead of spinning forever:
        // if it never appears, give up and disable live reload rather than leaking a
        // thread that wakes every 2s for the life of the process.
        const MAX_ATTEMPTS: u32 = 30; // ~60s
        let mut attempts = 0;
        let debouncer = loop {
            attempts += 1;
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
                    if attempts >= MAX_ATTEMPTS {
                        eprintln!(
                            "[config-watcher] {} did not appear after {attempts} attempts; \
                             live config reload disabled",
                            config_dir.display()
                        );
                        return;
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
            provider_reload::rebuild_providers(
                &new_cfg,
                &last,
                &shared,
                &registry,
                &content_state,
                &progress_cb,
                &content_watcher_tx,
                &notify_cb,
                &file_entries,
                &file_watcher_tx,
            );
            *last = new_cfg;
        }
    });
}

// ── Content filesystem watcher ────────────────────────────────────────────────

pub fn start_content_watcher(
    content_state: Arc<Mutex<Option<Arc<content_index::ContentIndex>>>>,
    initial_cfg: config::ContentConfig,
    notify_cb: Arc<dyn Fn() + Send + Sync>,
    shared: config::SharedConfig,
) -> std::sync::mpsc::Sender<config::ContentConfig> {
    run_dir_watcher(
        initial_cfg,
        |cfg: &config::ContentConfig| {
            cfg.dirs.iter().map(|d| config::Config::expand_path(&d.path)).collect()
        },
        move |events, current_cfg| {
            if !current_cfg.enabled {
                return;
            }
            let log = shared.read().unwrap().log_watcher;
            let idx = content_state.lock().unwrap().as_ref().map(Arc::clone);
            if let Some(idx) = idx {
                let mut changed = false;
                for ev in events {
                    if log {
                        eprintln!("[content-watcher] event {:?} paths={:?}", ev.event.kind, ev.event.paths);
                    }
                    for path in &ev.event.paths {
                        let result = content_index::process_event_path(&idx, path, current_cfg, log);
                        if log {
                            eprintln!("[content-watcher] process_event_path({:?}) -> {}", path, result);
                        }
                        if result {
                            changed = true;
                        }
                    }
                }
                if log {
                    eprintln!("[content-watcher] batch done: changed={changed}");
                }
                if changed {
                    notify_cb();
                }
            } else if log {
                eprintln!("[content-watcher] event received but content index is None");
            }
        },
    )
}

// ── File provider filesystem watcher ─────────────────────────────────────────

fn find_dir_for<'a>(
    path: &std::path::Path,
    dirs: &'a [(PathBuf, usize)],
) -> Option<(&'a std::path::Path, usize)> {
    dirs.iter()
        .find(|(base, _)| path.starts_with(base))
        .map(|(base, depth)| (base.as_path(), *depth))
}

pub fn start_file_watcher(
    entries: SharedFileEntries,
    initial_cfg: config::FilesConfig,
    notify_cb: Arc<dyn Fn() + Send + Sync>,
) -> std::sync::mpsc::Sender<config::FilesConfig> {
    run_dir_watcher(
        initial_cfg,
        |cfg: &config::FilesConfig| {
            cfg.dirs.iter().map(|d| config::Config::expand_path(&d.path)).collect()
        },
        move |events, current_cfg| {
            use notify_debouncer_full::notify::{event::ModifyKind, EventKind};

            let dirs_with_depth: Vec<(PathBuf, usize)> = current_cfg
                .dirs
                .iter()
                .map(|d| (config::Config::expand_path(&d.path), d.depth))
                .collect();

            let mut to_add: Vec<providers::files::FileEntry> = vec![];
            let mut to_remove: Vec<String> = vec![];

            for ev in events {
                let paths = &ev.event.paths;
                match &ev.event.kind {
                    EventKind::Create(_) => {
                        for path in paths {
                            if let Some((base, depth)) = find_dir_for(path, &dirs_with_depth) {
                                to_add.extend(providers::files::FileProvider::entries_for_path(
                                    path, base, depth,
                                ));
                            }
                        }
                    }
                    EventKind::Remove(_) => {
                        for path in paths {
                            to_remove.push(path.to_string_lossy().into_owned());
                        }
                    }
                    // Handle all rename variants uniformly:
                    // a path that still exists is the destination; one that is gone is the source.
                    EventKind::Modify(ModifyKind::Name(_)) => {
                        for path in paths {
                            if path.exists() {
                                if let Some((base, depth)) = find_dir_for(path, &dirs_with_depth) {
                                    to_add.extend(providers::files::FileProvider::entries_for_path(
                                        path, base, depth,
                                    ));
                                }
                            } else {
                                to_remove.push(path.to_string_lossy().into_owned());
                            }
                        }
                    }
                    _ => {}
                }
            }

            if !to_add.is_empty() || !to_remove.is_empty() {
                let mut guard = entries.write().unwrap();
                for p in &to_remove {
                    let prefix = format!("{p}/");
                    guard.retain(|e| e.path != *p && !e.path.starts_with(&prefix));
                }
                guard.extend(to_add);
                drop(guard);
                notify_cb();
            }
        },
    )
}

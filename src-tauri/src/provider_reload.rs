use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::extensions::kv::ExtensionKv;
use crate::{
    config, content_index, providers, ContentWatcherTx, FileWatcherTx, FrecencyState, Registry,
    SharedFileEntries,
};

/// Spawn a thread that calls `build()` to produce an optional provider, replaces
/// it in the registry under `id`, logs `name`, then fires `notify_cb`.
/// Used for the simple (apps) rebuild cases where the only difference is
/// what gets constructed.
fn spawn_rebuild(
    registry: &Registry,
    notify_cb: &Arc<dyn Fn() + Send + Sync>,
    id: &'static str,
    name: &'static str,
    build: impl FnOnce() -> Option<Box<dyn providers::Provider>> + Send + 'static,
) {
    let reg = Arc::clone(registry);
    let ncb = Arc::clone(notify_cb);
    std::thread::spawn(move || {
        let new = build();
        reg.write().unwrap().replace(id, new);
        eprintln!("[config] {name} provider rebuilt");
        ncb();
    });
}

pub fn rebuild_providers(
    new_cfg: &config::Config,
    old_cfg: &config::Config,
    shared: &config::SharedConfig,
    registry: &Registry,
    content_state: &Arc<Mutex<Option<Arc<content_index::ContentIndex>>>>,
    progress_cb: &Arc<dyn Fn(usize, usize) + Send + Sync>,
    content_watcher_tx: &ContentWatcherTx,
    notify_cb: &Arc<dyn Fn() + Send + Sync>,
    file_entries: &SharedFileEntries,
    file_watcher_tx: &FileWatcherTx,
    ext_kv: &Arc<ExtensionKv>,
    // Kept for parity with sync() callers even though the targeted
    // per-extension reload below never purges orphan state.
    _frecency: &FrecencyState,
) {
    // Update per-search scalars instantly (no rebuild needed).
    shared.write().unwrap().update_from(new_cfg);

    // Update registry-level settings (max_results, history_max_bonus).
    {
        let history_max_bonus = (new_cfg.search.history_weight as f32 / 100.0) * 1_500_000.0;
        let mut reg = registry.write().unwrap();
        reg.update_settings(new_cfg.general.max_results, history_max_bonus);
    }

    // ── Selectively rebuild index-backed providers ────────────────────────────

    let files_index_changed = !new_cfg.files.index_eq(&old_cfg.files)
        || new_cfg.providers.files != old_cfg.providers.files;
    if files_index_changed {
        let files_cfg = new_cfg.files.clone();
        let old_files_cfg = old_cfg.files.clone();
        let was_enabled = old_cfg.providers.files;
        let now_enabled = new_cfg.providers.files;
        let shared2 = Arc::clone(shared);
        let reg2 = Arc::clone(registry);
        let ncb = Arc::clone(notify_cb);
        let fe = Arc::clone(file_entries);
        let fw_tx = Arc::clone(file_watcher_tx);
        std::thread::spawn(move || {
            if now_enabled {
                let old_by_path: HashMap<&str, &config::DirEntry> =
                    old_files_cfg.dirs.iter().map(|d| (d.path.as_str(), d)).collect();
                let new_by_path: HashMap<&str, &config::DirEntry> =
                    files_cfg.dirs.iter().map(|d| (d.path.as_str(), d)).collect();

                // Case A: pure additions only - walk new dirs and extend.
                // Case B: any removal or depth change - full re-walk to avoid
                //         nested-dir prefix-removal bugs (e.g. removing ~/Docs
                //         when ~/Docs/Projects is still configured).
                let only_additions = old_files_cfg.dirs.iter().all(|d| new_by_path.contains_key(d.path.as_str()))
                    && files_cfg.dirs.iter().all(|d| {
                        match old_by_path.get(d.path.as_str()) {
                            Some(old) => old.depth == d.depth,
                            None => true,
                        }
                    });

                if only_additions {
                    let added_cfg = config::FilesConfig {
                        dirs: files_cfg.dirs.iter()
                            .filter(|d| !old_by_path.contains_key(d.path.as_str()))
                            .cloned()
                            .collect(),
                        show_dotfiles: files_cfg.show_dotfiles,
                        colored_icons: files_cfg.colored_icons,
                    };
                    let new_entries = providers::files::FileProvider::walk_dirs(&added_cfg);
                    fe.write().unwrap().extend(new_entries);
                } else {
                    providers::files::FileProvider::populate(&fe, &files_cfg);
                }

                if !was_enabled {
                    let p = providers::files::FileProvider::with_entries(Arc::clone(&fe), shared2);
                    reg2.write().unwrap().replace("files", Some(Box::new(p)));
                }
            } else {
                *fe.write().unwrap() = vec![];
                reg2.write().unwrap().replace("files", None);
            }
            if let Some(tx) = fw_tx.lock().unwrap().as_ref() {
                let _ = tx.send(files_cfg);
            }
            eprintln!("[config] files provider rebuilt");
            ncb();
        });
    } else if new_cfg.files != old_cfg.files {
        // Display-only change (colored_icons): no re-walk, just refresh the UI.
        notify_cb();
    }

    if new_cfg.providers.apps != old_cfg.providers.apps {
        let enabled = new_cfg.providers.apps;
        let shared2 = Arc::clone(shared);
        spawn_rebuild(registry, notify_cb, "apps", "apps", move || {
            enabled.then(|| Box::new(providers::apps::AppProvider::new(shared2)) as _)
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
        notify_cb();
    }

    if new_cfg.dict != old_cfg.dict {
        let mut reg = registry.write().unwrap();
        if new_cfg.dict.enabled {
            let p = providers::dict::DictProvider::new(&new_cfg.dict);
            if p.available {
                reg.replace("dict", Some(Box::new(p)));
            } else {
                reg.replace("dict", None);
            }
            reg.set_dict_fill(Some((new_cfg.dict.fill_threshold, new_cfg.dict.fill_max)));
            eprintln!("[config] dict provider enabled");
        } else {
            reg.replace("dict", None);
            reg.set_dict_fill(None);
            eprintln!("[config] dict provider disabled");
        }
        notify_cb();
    }

    if new_cfg.extensions != old_cfg.extensions {
        // Targeted reload: only extensions whose entry (enabled flag or
        // settings table) actually changed are rebuilt; everything else keeps
        // its warm instances.
        let changed: Vec<String> = new_cfg
            .extensions
            .keys()
            .chain(old_cfg.extensions.keys())
            .filter(|n| new_cfg.extensions.get(*n) != old_cfg.extensions.get(*n))
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let extensions_cfg = new_cfg.extensions.clone();
        let reg2 = Arc::clone(registry);
        let ncb = Arc::clone(notify_cb);
        let kv = Arc::clone(ext_kv);
        // Wasm compilation is slow - build instances off-thread and only
        // take the registry write lock for pointer swaps.
        std::thread::spawn(move || {
            for name in &changed {
                crate::extensions::sync_one(
                    &reg2,
                    name,
                    &extensions_cfg,
                    &kv,
                    Some(Arc::clone(&ncb)),
                );
            }
            eprintln!("[config] {} extension(s) reloaded", changed.len());
            ncb();
        });
    }

    if new_cfg.content != old_cfg.content {
        let new_content_cfg = new_cfg.content.clone();
        let old_content_cfg = old_cfg.content.clone();
        // Notify the filesystem watcher of the new config so it can watch any added dirs.
        if let Some(tx) = content_watcher_tx.lock().unwrap().as_ref() {
            let _ = tx.send(new_content_cfg.clone());
        }
        let reg2 = Arc::clone(registry);
        let ci_state = Arc::clone(content_state);
        let cb = Arc::clone(progress_cb);
        let ncb = Arc::clone(notify_cb);
        let max_results = new_cfg.general.max_results;
        std::thread::spawn(move || {
            // Hold the lock for the full operation so two rapid config saves
            // can't race each other on the same DB tables.
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

                // Register provider with the current index so existing data is
                // immediately searchable, even before any reindex completes.
                reg2.write().unwrap().replace(
                    "content",
                    Some(Box::new(providers::content::ContentProvider::new(
                        Arc::clone(&idx),
                        max_results,
                    ))),
                );

                // "Heavy" changes require a full clear+rebuild, which is expensive.
                // We never trigger that automatically - the settings UI stages these
                // edits and the user confirms via "Apply & Reindex" (trigger_full_reindex),
                // or a poweruser runs `portunus --reindex` after a manual config edit.
                // Here we only register the provider (above) and apply cheap incremental
                // changes; heavy changes are left for the explicit reindex path.
                let ocr_changed = old_content_cfg.ocr_images != new_content_cfg.ocr_images
                    || old_content_cfg.ocr_pdf_fallback != new_content_cfg.ocr_pdf_fallback
                    || old_content_cfg.ocr_language != new_content_cfg.ocr_language;
                // First-enable: content was disabled before AND the index is empty.
                // Re-enabling a populated index is just a cheap incremental run.
                let first_enable = !old_content_cfg.enabled && idx.is_empty();
                let max_bytes_increased =
                    new_content_cfg.max_file_bytes > old_content_cfg.max_file_bytes;

                if ocr_changed || first_enable || max_bytes_increased {
                    eprintln!(
                        "[content] heavy settings change detected; full reindex deferred \
                         (apply via Settings or `portunus --reindex`)"
                    );
                } else if old_content_cfg.contents_eq(&new_content_cfg) {
                    // Only indexing-speed settings (threads) changed - the index
                    // contents are unaffected, so a reindex would be pure waste and
                    // would race the progress bar against any in-flight run.
                    eprintln!("[content] non-content settings change; skipping reindex");
                } else {
                    // Cheap, non-destructive incremental update - same as the startup routine.
                    // Picks up added dirs, extension/depth changes, and removed dirs.
                    // Guarded so a config save mid-reindex doesn't start a second run.
                    match content_index::ReindexGuard::acquire() {
                        Some(_guard) => {
                            content_index::run_content_indexer(idx, &new_content_cfg, Some(cb));
                            eprintln!("[content] incremental reindex complete");
                            ncb();
                        }
                        None => eprintln!(
                            "[content] reindex already in progress; skipping incremental update"
                        ),
                    }
                }
            } else {
                *guard = None;
                reg2.write().unwrap().replace("content", None);
                eprintln!("[content] content provider disabled");
                ncb();
            }
        });
    }

    eprintln!("[config] reload complete");
}

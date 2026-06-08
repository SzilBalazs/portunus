//! WASM extension system: discovery, lifecycle, and Tauri commands.
//!
//! Extensions live in `$XDG_DATA_HOME/portunus/extensions/<name>/` as
//! `manifest.toml` + `extension.wasm`. Discovery is passive - a dropped-in
//! extension shows up disabled in Settings and only runs once the user has
//! reviewed its permissions and enabled it.

pub mod hostfns;
pub mod kv;
pub mod manifest;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use portunus_ext_sdk::{ExtensionResult, PreviewContent};
use tauri::Manager;

use crate::providers::wasm::WasmProvider;
use crate::{ConfigState, FrecencyState, Registry};
use kv::ExtensionKv;

/// Shared KV store handle, managed as Tauri state at startup.
pub struct ExtensionKvState(pub Arc<ExtensionKv>);

pub fn extensions_dir() -> PathBuf {
    let data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{home}/.local/share")
    });
    PathBuf::from(data_home).join("portunus").join("extensions")
}

/// One on-disk extension: a validated manifest or the reason it failed.
pub struct Discovered {
    pub name: String,
    pub manifest: Option<(manifest::ExtensionManifest, PathBuf)>,
    pub error: Option<String>,
}

/// Scans the extensions dir. Never touches wasm - manifest parsing only.
pub fn discover() -> Vec<Discovered> {
    let dir = extensions_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<Discovered> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            Some(match manifest::load(&e.path()) {
                Ok(m) => Discovered { name, manifest: Some(m), error: None },
                Err(err) => Discovered { name, manifest: None, error: Some(err) },
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Reconciles loaded extensions with disk + config. `force` rebuilds even
/// already-loaded extensions (the `--reload-extensions` hot-reload path, so a
/// recompiled wasm is picked up). Compiles modules - call from a background
/// thread; the registry write lock is only taken for pointer swaps.
///
/// Extensions with a `[background]` section get a detached `refresh("load")`
/// so their kv cache warms immediately; `notify_cb` fires afterwards so an
/// open launcher picks the fresh data up.
pub fn sync(
    registry: &Registry,
    enabled: &HashMap<String, bool>,
    kv_store: &Arc<ExtensionKv>,
    frecency: &FrecencyState,
    force: bool,
    notify_cb: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let discovered = discover();
    let on_disk: Vec<&Discovered> = discovered.iter().collect();

    // Clean up state belonging to extensions whose directory is gone. The
    // orphan census is the union of every store that records per-extension
    // state - kv alone would miss extensions that only have frecency history.
    let disk_names: std::collections::HashSet<&str> =
        on_disk.iter().map(|d| d.name.as_str()).collect();
    let mut known: std::collections::HashSet<String> =
        kv_store.extension_names().into_iter().collect();
    if let Some(store) = frecency {
        known.extend(store.extension_names());
    }
    for orphan in known.into_iter().filter(|n| !disk_names.contains(n.as_str())) {
        kv_store.delete_extension(&orphan);
        if let Some(store) = frecency {
            store.delete_prefix(&format!("ext:{orphan}:"));
        }
    }

    // Drop loaded extensions that are now disabled or gone from disk.
    let loaded = crate::util::read(registry).extension_names();
    for name in &loaded {
        let want = disk_names.contains(name.as_str())
            && enabled.get(name).copied().unwrap_or(false);
        if !want {
            crate::util::write(registry).set_extension(name, None);
        }
    }

    // Load (or force-rebuild) enabled extensions. Compile outside the lock.
    for d in on_disk {
        if !enabled.get(&d.name).copied().unwrap_or(false) {
            continue;
        }
        let Some((m, wasm_path)) = d.manifest.clone() else {
            continue; // invalid manifest - surfaced via list_extensions
        };
        if !force && loaded.contains(&d.name) {
            continue;
        }
        match WasmProvider::load(m, wasm_path, kv_store.clone()) {
            Ok(p) => {
                let p = Arc::new(p);
                crate::util::write(registry).set_extension(&d.name, Some(p.clone()));
                // Warm the cache off-thread; sync must never wait on network.
                if p.background_interval_secs().is_some() {
                    let ncb = notify_cb.clone();
                    std::thread::spawn(move || {
                        if p.refresh("load").is_ok() {
                            if let Some(ncb) = ncb {
                                ncb();
                            }
                        }
                    });
                }
            }
            Err(e) => {
                eprintln!("[ext:{}] {e}", d.name);
                crate::util::write(registry).set_extension(&d.name, None);
            }
        }
    }
}

/// Interval scheduler for extensions declaring `[background]`. One global
/// thread; refreshes run sequentially on each extension's dedicated
/// background instance, so neither the keystroke path nor other extensions
/// are blocked by a slow refresh.
///
/// First sighting of an extension schedules it one full interval out - the
/// load-time refresh in `sync()` already covered "now". Five consecutive
/// failures stop its schedule until the extension is reloaded.
pub fn start_refresh_scheduler(registry: Registry, notify_cb: Arc<dyn Fn() + Send + Sync>) {
    const TICK: std::time::Duration = std::time::Duration::from_secs(30);
    const MAX_REFRESH_FAILURES: u32 = 5;

    std::thread::spawn(move || {
        let mut next_run: HashMap<String, std::time::Instant> = HashMap::new();
        let mut failures: HashMap<String, u32> = HashMap::new();
        loop {
            std::thread::sleep(TICK);

            // Snapshot due extensions under a brief read lock; run outside it.
            let scheduled: Vec<(String, u64, Arc<WasmProvider>)> = {
                let reg = crate::util::read(&registry);
                reg.extension_names()
                    .into_iter()
                    .filter_map(|name| {
                        let p = reg.extension(&name)?;
                        let interval = p.background_interval_secs()?;
                        Some((name, interval, p))
                    })
                    .collect()
            };

            // Drop schedule state for unloaded extensions (so a reload
            // restarts both the schedule and the failure counter).
            let live: std::collections::HashSet<&str> =
                scheduled.iter().map(|(n, _, _)| n.as_str()).collect();
            next_run.retain(|n, _| live.contains(n.as_str()));
            failures.retain(|n, _| live.contains(n.as_str()));

            let now = std::time::Instant::now();
            for (name, interval_secs, ext) in scheduled {
                let interval = std::time::Duration::from_secs(interval_secs);
                let Some(&due) = next_run.get(&name) else {
                    next_run.insert(name, now + interval);
                    continue;
                };
                if now < due || failures.get(&name).copied().unwrap_or(0) >= MAX_REFRESH_FAILURES
                {
                    continue;
                }
                next_run.insert(name.clone(), now + interval);
                match ext.refresh("scheduled") {
                    Ok(()) => {
                        failures.remove(&name);
                        notify_cb();
                    }
                    Err(_) => {
                        let n = failures.entry(name.clone()).or_insert(0);
                        *n += 1;
                        if *n >= MAX_REFRESH_FAILURES {
                            eprintln!(
                                "[ext:{name}] background refresh stopped after {n} consecutive failures (reload to retry)"
                            );
                        }
                    }
                }
            }
        }
    });
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct PermissionsInfo {
    network: Vec<String>,
    kv: bool,
    clipboard: bool,
    open_url: bool,
}

#[derive(serde::Serialize)]
pub struct ExtensionInfo {
    name: String,
    version: String,
    description: String,
    author: String,
    permissions: Option<PermissionsInfo>,
    enabled: bool,
    loaded: bool,
    /// Manifest/load error, or the most recent runtime error - the author's
    /// primary debugging signal, shown in the Settings tab.
    error: Option<String>,
    /// True when the extension failed repeatedly and was benched this session.
    benched: bool,
    /// Set when the manifest declares `[background]` - shown as a chip.
    background_interval_secs: Option<u64>,
}

#[tauri::command]
pub fn list_extensions(
    config: tauri::State<'_, ConfigState>,
    registry: tauri::State<'_, Registry>,
) -> Vec<ExtensionInfo> {
    let enabled_map = crate::util::lock(&config).extensions.enabled.clone();
    let reg = crate::util::read(&registry);
    discover()
        .into_iter()
        .map(|d| {
            let provider = reg.extension(&d.name);
            let enabled = enabled_map.get(&d.name).copied().unwrap_or(false);
            let error = provider
                .as_ref()
                .and_then(|p| p.last_error())
                .or(d.error);
            ExtensionInfo {
                name: d.name,
                version: d.manifest.as_ref().map(|(m, _)| m.version.clone()).unwrap_or_default(),
                description: d
                    .manifest
                    .as_ref()
                    .map(|(m, _)| m.description.clone())
                    .unwrap_or_default(),
                author: d.manifest.as_ref().map(|(m, _)| m.author.clone()).unwrap_or_default(),
                permissions: d.manifest.as_ref().map(|(m, _)| PermissionsInfo {
                    network: m.permissions.network.clone(),
                    kv: m.permissions.kv,
                    clipboard: m.permissions.clipboard,
                    open_url: m.permissions.open_url,
                }),
                enabled,
                loaded: provider.is_some(),
                benched: provider.as_ref().is_some_and(|p| p.is_benched()),
                background_interval_secs: d
                    .manifest
                    .as_ref()
                    .and_then(|(m, _)| m.background.as_ref().map(|b| b.interval_secs())),
                error,
            }
        })
        .collect()
}

/// Splits `ext:<name>:<local>` and returns the loaded provider plus nothing
/// else the caller needs to parse - the id grammar lives here only.
fn provider_for_id(
    registry: &crate::providers::PluginRegistry,
    id: &str,
) -> Result<Arc<WasmProvider>, String> {
    let name = id
        .strip_prefix("ext:")
        .and_then(|rest| rest.split(':').next())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| format!("not an extension result id: {id}"))?;
    registry
        .extension(name)
        .ok_or_else(|| format!("extension \"{name}\" is not loaded"))
}

#[tauri::command]
pub fn extension_activate(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    frecency: tauri::State<'_, FrecencyState>,
    id: String,
    ext: ExtensionResult,
    action: Option<String>,
) -> Result<(), String> {
    // Hide first: activation may briefly wait on an in-flight search call
    // holding the instance lock (up to the search budget) - the launcher must
    // dismiss instantly on Enter, like launch_app does. Failures still land
    // in last_error / Settings.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let provider = provider_for_id(&crate::util::read(&registry), &id)?;
    provider.activate(ext, action)?;
    if let Some(store) = frecency.as_ref() {
        store.record_launch(&id, "extension");
    }
    Ok(())
}

#[tauri::command]
pub async fn extension_preview(
    registry: tauri::State<'_, Registry>,
    id: String,
    ext: ExtensionResult,
) -> Result<Option<PreviewContent>, String> {
    // Resolve the provider synchronously (fast read-lock), then run the wasm
    // call on the blocking thread pool so rapid navigation never stalls the
    // Tauri async runtime while the extension does its network I/O.
    let provider = provider_for_id(&crate::util::read(&registry), &id)?;
    tauri::async_runtime::spawn_blocking(move || provider.preview(ext))
        .await
        .map_err(|e| e.to_string())?
}

/// Re-discovers the extensions dir and force-reloads wasm bytes. Wired to the
/// Settings tab button and the `portunus --reload-extensions` CLI flag - the
/// extension author's iteration loop.
#[tauri::command]
pub fn rescan_extensions(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    config: tauri::State<'_, ConfigState>,
    frecency: tauri::State<'_, FrecencyState>,
    kv_state: tauri::State<'_, ExtensionKvState>,
) {
    let registry = registry.inner().clone();
    let enabled = crate::util::lock(&config).extensions.enabled.clone();
    let kv_store = kv_state.0.clone();
    let frecency = frecency.inner().clone();
    std::thread::spawn(move || {
        use tauri::Emitter;
        let notify_app = app.clone();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = notify_app.emit("search-invalidated", ());
        });
        sync(&registry, &enabled, &kv_store, &frecency, true, Some(notify));
        let _ = app.emit("search-invalidated", ());
    });
}

//! WASM extension system: discovery, lifecycle, and Tauri commands.
//!
//! Extensions live in `$XDG_DATA_HOME/portunus/extensions/<name>/` as
//! `manifest.toml` + `extension.wasm`. Discovery is passive - a dropped-in
//! extension shows up disabled in Settings and only runs once the user has
//! reviewed its permissions and enabled it.

pub mod hostfns;
pub mod install;
pub mod kv;
pub mod logs;
pub mod manifest;
pub mod query;
pub mod secrets;
pub mod trigger;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use portunus_ext_sdk::{ActivateEffect, ExtensionResult, PreviewContent};
use tauri::Manager;

use crate::config::ExtensionsConfig;
use crate::providers::wasm::WasmProvider;
use crate::{ConfigState, FrecencyState, Registry};
use kv::ExtensionKv;

/// Shared KV store handle, managed as Tauri state at startup.
pub struct ExtensionKvState(pub Arc<ExtensionKv>);

pub fn extensions_dir() -> PathBuf {
    crate::paths::data_dir().join("extensions")
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
            // Dot-prefixed dirs are install machinery (.staging-*/.old-*),
            // never extensions.
            if name.starts_with('.') {
                return None;
            }
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
    extensions_cfg: &ExtensionsConfig,
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
    // Present = any directory entry, including a momentarily-broken dev
    // symlink (mid-rebuild) - purging a dev extension's kv because cargo was
    // between writes would be data loss.
    let mut disk_names: std::collections::HashSet<String> =
        on_disk.iter().map(|d| d.name.clone()).collect();
    if let Ok(entries) = std::fs::read_dir(extensions_dir()) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if !name.starts_with('.') {
                    disk_names.insert(name.to_string());
                }
            }
        }
    }
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
        let want = disk_names.contains(name.as_str()) && entry_enabled(extensions_cfg, name);
        if !want {
            crate::util::write(registry).set_extension(name, None);
        }
    }

    // Load (or force-rebuild) enabled extensions. Compile outside the lock.
    for d in on_disk {
        if !entry_enabled(extensions_cfg, &d.name) {
            continue;
        }
        if !force && loaded.contains(&d.name) {
            continue;
        }
        load_one(registry, d, extensions_cfg, kv_store, notify_cb.clone());
    }
}

fn entry_enabled(cfg: &ExtensionsConfig, name: &str) -> bool {
    cfg.get(name).map(|e| e.enabled).unwrap_or(false)
}

/// Resolves the effective settings snapshot for one extension: schema
/// defaults overlaid with the (validated) user values from config.
fn resolved_settings(
    m: &manifest::ExtensionManifest,
    cfg: &ExtensionsConfig,
) -> HashMap<String, serde_json::Value> {
    let user: serde_json::Map<String, serde_json::Value> = cfg
        .get(&m.name)
        .and_then(|e| serde_json::to_value(&e.settings).ok())
        .and_then(|v| match v {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    manifest::resolve_settings(&m.settings_schema, &user)
}

/// `resolved_settings` plus keyring-stored secret values overlaid. Blocking
/// dbus per secret spec - load-path only, never the keystroke path (and never
/// the frontend: `ExtensionInfo` uses the secret-free `resolved_settings`).
fn resolved_settings_with_secrets(
    m: &manifest::ExtensionManifest,
    cfg: &ExtensionsConfig,
) -> HashMap<String, serde_json::Value> {
    let mut out = resolved_settings(m, cfg);
    for spec in m.settings_schema.iter().filter(|s| s.kind == "secret") {
        match secrets::get(&m.name, &spec.key) {
            Ok(Some(v)) => {
                out.insert(spec.key.clone(), serde_json::Value::String(v));
            }
            Ok(None) => {}
            Err(e) => logs::log(&m.name, logs::LogLevel::Error, &e),
        }
    }
    out
}

/// Compiles and registers one discovered extension (always rebuilds).
/// Compile happens outside the registry lock; only pointers swap under it.
fn load_one(
    registry: &Registry,
    d: &Discovered,
    extensions_cfg: &ExtensionsConfig,
    kv_store: &Arc<ExtensionKv>,
    notify_cb: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let Some((m, wasm_path)) = d.manifest.clone() else {
        return; // invalid manifest - surfaced via list_extensions
    };
    // Consent gate: a manifest whose permissions grew past the consented
    // snapshot must not load until the user re-approves in Settings.
    if let Err(e) = install::check_consent(&m) {
        logs::log(&d.name, logs::LogLevel::Error, &e);
        crate::util::write(registry).set_extension(&d.name, None);
        return;
    }
    let settings = resolved_settings_with_secrets(&m, extensions_cfg);
    match WasmProvider::load(m, wasm_path, kv_store.clone(), settings) {
        Ok(p) => {
            let p = Arc::new(p);
            crate::util::write(registry).set_extension(&d.name, Some(p.clone()));
            // Warm the cache off-thread; sync must never wait on network.
            if p.background_interval_secs().is_some() {
                std::thread::spawn(move || {
                    if p.refresh("load").is_ok() {
                        if let Some(ncb) = notify_cb {
                            ncb();
                        }
                    }
                });
            }
        }
        Err(e) => {
            logs::log(&d.name, logs::LogLevel::Error, &e);
            crate::util::write(registry).set_extension(&d.name, None);
        }
    }
}

/// Targeted reload of a single extension - the cheap path for settings
/// changes and dev-mode iteration. Rebuilds (or unloads) exactly one
/// extension; everything else keeps its warm instances.
pub fn sync_one(
    registry: &Registry,
    name: &str,
    extensions_cfg: &ExtensionsConfig,
    kv_store: &Arc<ExtensionKv>,
    notify_cb: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    let dir = extensions_dir().join(name);
    let enabled = entry_enabled(extensions_cfg, name);
    if !dir.is_dir() || !enabled {
        crate::util::write(registry).set_extension(name, None);
        return;
    }
    let d = match manifest::load(&dir) {
        Ok(m) => Discovered { name: name.to_string(), manifest: Some(m), error: None },
        Err(e) => {
            logs::log(name, logs::LogLevel::Error, &e);
            crate::util::write(registry).set_extension(name, None);
            return;
        }
    };
    load_one(registry, &d, extensions_cfg, kv_store, notify_cb);
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
    /// Derived: the settings schema declares at least one `type = "secret"`.
    has_secrets: bool,
}

#[derive(serde::Serialize)]
pub struct ExtensionInfo {
    name: String,
    version: String,
    description: String,
    author: String,
    homepage: String,
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
    /// Trigger prefixes from `[trigger]`; empty = always-mode.
    triggers: Vec<String>,
    /// The result kind this extension emits (drives launcher group labels).
    kind: Option<String>,
    /// Declared `[[settings]]` schema, rendered by the Settings UI.
    settings_schema: Vec<manifest::SettingSpec>,
    /// Effective settings values (defaults overlaid with user config).
    settings_values: HashMap<String, serde_json::Value>,
    /// True when the manifest's permissions grew past the consented snapshot -
    /// the extension won't load until the user re-approves.
    needs_reconsent: bool,
    /// Install origin: "url", "file" or "dev". None = no record yet.
    origin: Option<String>,
    /// Origin URL for url-installed extensions (update source).
    origin_url: Option<String>,
    /// True when the extensions-dir entry is a symlink (`portunus ext dev`).
    dev: bool,
    /// Secret setting keys that currently have a stored keyring value.
    secrets_set: Vec<String>,
}

#[tauri::command]
pub fn list_extensions(
    config: tauri::State<'_, ConfigState>,
    registry: tauri::State<'_, Registry>,
) -> Vec<ExtensionInfo> {
    let extensions_cfg = crate::util::lock(&config).extensions.clone();
    let consents = install::load_consents();
    let reg = crate::util::read(&registry);
    discover()
        .into_iter()
        .map(|d| {
            let provider = reg.extension(&d.name);
            let enabled = entry_enabled(&extensions_cfg, &d.name);
            let m = d.manifest.as_ref().map(|(m, _)| m);
            let needs_reconsent = m.is_some_and(|m| {
                consents.get(&d.name).is_some_and(|rec| {
                    !matches!(rec.origin, install::Origin::Dev)
                        && rec.permissions.grew_to(
                            &install::ConsentPermissions::from_manifest(m),
                        )
                })
            });
            let error = provider
                .as_ref()
                .and_then(|p| p.last_error())
                .or(d.error)
                .or_else(|| {
                    needs_reconsent.then(|| {
                        "permissions changed since install - review and re-approve".to_string()
                    })
                });
            let consent = consents.get(&d.name);
            let dev = std::fs::symlink_metadata(extensions_dir().join(&d.name))
                .map(|meta| meta.is_symlink())
                .unwrap_or(false);
            ExtensionInfo {
                version: m.map(|m| m.version.clone()).unwrap_or_default(),
                description: m.map(|m| m.description.clone()).unwrap_or_default(),
                author: m.map(|m| m.author.clone()).unwrap_or_default(),
                homepage: m.map(|m| m.homepage.clone()).unwrap_or_default(),
                permissions: m.map(|m| PermissionsInfo {
                    network: m.permissions.network.clone(),
                    kv: m.permissions.kv,
                    clipboard: m.permissions.clipboard,
                    open_url: m.permissions.open_url,
                    has_secrets: m.settings_schema.iter().any(|s| s.kind == "secret"),
                }),
                enabled,
                loaded: provider.is_some(),
                benched: provider.as_ref().is_some_and(|p| p.is_benched()),
                background_interval_secs: m
                    .and_then(|m| m.background.as_ref().map(|b| b.interval_secs())),
                triggers: m
                    .and_then(|m| m.trigger.as_ref())
                    .map(|t| t.prefixes.clone())
                    .unwrap_or_default(),
                kind: m.map(|m| m.default_kind()),
                settings_schema: m.map(|m| m.settings_schema.clone()).unwrap_or_default(),
                settings_values: m
                    .map(|m| resolved_settings(m, &extensions_cfg))
                    .unwrap_or_default(),
                needs_reconsent,
                origin: consent.map(|rec| match rec.origin {
                    install::Origin::Url { .. } => "url".to_string(),
                    install::Origin::File => "file".to_string(),
                    install::Origin::Dev => "dev".to_string(),
                }),
                origin_url: consent.and_then(|rec| match &rec.origin {
                    install::Origin::Url { url } => Some(url.clone()),
                    _ => None,
                }),
                dev,
                error,
                // Presence only (a few dbus round-trips per Settings render) -
                // values never leave the keyring.
                secrets_set: m
                    .map(|m| {
                        m.settings_schema
                            .iter()
                            .filter(|s| s.kind == "secret" && secrets::exists(&d.name, &s.key))
                            .map(|s| s.key.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
                name: d.name,
            }
        })
        .collect()
}

/// Validates and persists an extension's settings values, then hot-reloads
/// just that extension so its `settings_get` snapshot picks them up.
#[tauri::command]
pub fn set_extension_settings(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    config: tauri::State<'_, ConfigState>,
    kv_state: tauri::State<'_, ExtensionKvState>,
    name: String,
    values: HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    let dir = extensions_dir().join(&name);
    let (m, _) = manifest::load(&dir)?;

    // Validate every incoming value against the schema before writing config.
    let mut table = toml::Table::new();
    for spec in &m.settings_schema {
        let Some(v) = values.get(&spec.key) else { continue };
        // Guarantee no secret ever lands in config.toml, even via a buggy
        // frontend - secrets travel only through extension_secret_set.
        if spec.kind == "secret" {
            return Err(format!(
                "setting \"{}\": secrets are set via extension_secret_set",
                spec.key
            ));
        }
        let coerced = manifest::coerce_setting_value(spec, v)
            .map_err(|e| format!("setting \"{}\": {e}", spec.key))?;
        let toml_v: toml::Value = serde_json::from_value::<toml::Value>(coerced)
            .map_err(|e| format!("setting \"{}\": {e}", spec.key))?;
        table.insert(spec.key.clone(), toml_v);
    }

    let extensions_cfg = {
        let mut cfg = crate::util::lock(&config);
        cfg.extensions.entry(name.clone()).or_default().settings = table;
        cfg.save()?;
        cfg.extensions.clone()
    };

    // Targeted reload off-thread (compiles wasm); pointer swap under lock.
    let registry = registry.inner().clone();
    let kv_store = kv_state.0.clone();
    std::thread::spawn(move || {
        use tauri::Emitter;
        sync_one(&registry, &name, &extensions_cfg, &kv_store, None);
        let _ = app.emit("search-invalidated", ());
        let _ = app.emit("extensions-reloaded", ());
    });
    Ok(())
}

/// Looks up a schema spec and requires it to be a secret - the secret
/// commands must never write arbitrary keyring entries.
fn require_secret_spec(name: &str, key: &str) -> Result<(), String> {
    let dir = extensions_dir().join(name);
    let (m, _) = manifest::load(&dir)?;
    m.settings_schema
        .iter()
        .any(|s| s.key == key && s.kind == "secret")
        .then_some(())
        .ok_or_else(|| format!("\"{key}\" is not a declared secret setting"))
}

/// Reloads one extension after a secret change so its `settings_get`
/// snapshot picks the new value up (same shape as set_extension_settings).
fn reload_after_secret_change(
    app: tauri::AppHandle,
    registry: &Registry,
    kv_state: &ExtensionKvState,
    config: &ConfigState,
    name: String,
) {
    let extensions_cfg = crate::util::lock(config).extensions.clone();
    let registry = registry.clone();
    let kv_store = kv_state.0.clone();
    std::thread::spawn(move || {
        use tauri::Emitter;
        sync_one(&registry, &name, &extensions_cfg, &kv_store, None);
        let _ = app.emit("search-invalidated", ());
        let _ = app.emit("extensions-reloaded", ());
    });
}

/// Stores one secret setting in the system keyring and hot-reloads the
/// extension. Values never touch config.toml or the log store.
#[tauri::command]
pub async fn extension_secret_set(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    config: tauri::State<'_, ConfigState>,
    kv_state: tauri::State<'_, ExtensionKvState>,
    name: String,
    key: String,
    value: String,
) -> Result<(), String> {
    require_secret_spec(&name, &key)?;
    let (n, k) = (name.clone(), key.clone());
    tauri::async_runtime::spawn_blocking(move || secrets::set(&n, &k, &value))
        .await
        .map_err(|e| e.to_string())??;
    install::record_secret_key(&name, &key);
    reload_after_secret_change(app, &registry, &kv_state, &config, name);
    Ok(())
}

/// Removes one secret setting from the keyring and hot-reloads the extension.
#[tauri::command]
pub async fn extension_secret_clear(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    config: tauri::State<'_, ConfigState>,
    kv_state: tauri::State<'_, ExtensionKvState>,
    name: String,
    key: String,
) -> Result<(), String> {
    require_secret_spec(&name, &key)?;
    let (n, k) = (name.clone(), key.clone());
    tauri::async_runtime::spawn_blocking(move || secrets::delete(&n, &k))
        .await
        .map_err(|e| e.to_string())??;
    reload_after_secret_change(app, &registry, &kv_state, &config, name);
    Ok(())
}

/// Whether a Secret Service daemon is reachable (drives the Settings UI's
/// enabled/disabled state for secret fields).
#[tauri::command]
pub async fn secrets_available() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(secrets::available)
        .await
        .map_err(|e| e.to_string())
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
    // Hide first: activation may block for seconds (network I/O inside the
    // extension) - the launcher must dismiss instantly on Enter, like
    // launch_app does. Effects that need user-visible output (ShowToast) go
    // through desktop notifications, which work with the window hidden.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let provider = provider_for_id(&crate::util::read(&registry), &id)?;
    let effects = provider.activate(ext, action)?;
    run_activate_effects(&app, &id, effects);
    if let Some(store) = frecency.as_ref() {
        store.record_launch(&id, "extension");
    }
    Ok(())
}

/// Executes the declarative effects an `activate` call returned. Effects run
/// only on explicit user activation - the keypress is the consent - so none
/// of them consult manifest permissions. Caps mirror the equivalent host fns.
fn run_activate_effects(app: &tauri::AppHandle, id: &str, effects: Vec<ActivateEffect>) {
    use tauri::Emitter;
    for effect in effects {
        match effect {
            ActivateEffect::CopyText { text } => {
                if let Err(e) = hostfns::write_clipboard_text(&text) {
                    eprintln!("[{id}] copy_text effect: {e}");
                }
            }
            ActivateEffect::OpenUrl { url } => {
                if let Err(e) = hostfns::open_http_url(&url) {
                    eprintln!("[{id}] open_url effect: {e}");
                }
            }
            ActivateEffect::ShowToast { message } => {
                let mut msg = message;
                crate::util::truncate_char_boundary(&mut msg, 512);
                // The launcher window is already hidden at this point, so an
                // in-window toast would be invisible - use a desktop
                // notification. Also emitted as an event for visible windows.
                let _ = app.emit("extension-toast", &msg);
                let _ = std::process::Command::new("notify-send")
                    .arg("--app-name=Portunus")
                    .arg("--expire-time=3000")
                    .arg("Portunus")
                    .arg(&msg)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            }
        }
    }
}

#[derive(serde::Serialize, Clone)]
struct PreviewChunk {
    request_id: u64,
    content: PreviewContent,
}

#[tauri::command]
pub async fn extension_preview(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    id: String,
    ext: ExtensionResult,
    request_id: u64,
) -> Result<Option<PreviewContent>, String> {
    // Resolve the provider synchronously (fast read-lock), then run the wasm
    // call on the blocking thread pool so rapid navigation never stalls the
    // Tauri async runtime while the extension does its network I/O.
    let provider = provider_for_id(&crate::util::read(&registry), &id)?;
    // A streaming preview may hold its budget for seconds - the previous
    // call must not delay this one behind the preview-instance mutex.
    provider.cancel_preview();
    tauri::async_runtime::spawn_blocking(move || {
        provider.preview(ext, move |content| {
            use tauri::Emitter;
            let _ = app.emit("extension-preview-chunk", PreviewChunk { request_id, content });
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Aborts the in-flight streaming preview of one extension (selection moved).
#[tauri::command]
pub fn extension_preview_cancel(
    registry: tauri::State<'_, Registry>,
    id: String,
) -> Result<(), String> {
    provider_for_id(&crate::util::read(&registry), &id)?.cancel_preview();
    Ok(())
}

/// Tail of one extension's log ring buffer (oldest first) - the Settings log
/// viewer polls this while the panel is open.
#[tauri::command]
pub fn get_extension_logs(name: String, limit: Option<usize>) -> Vec<logs::LogEntry> {
    logs::LOGS.tail(&name, limit.unwrap_or(100).min(200))
}

/// Empties one extension's log ring buffer (the viewer's Clear button).
#[tauri::command]
pub fn clear_extension_logs(name: String) {
    logs::LOGS.purge(&name);
}

/// Why extension storage is running non-persistently this session (on-disk
/// SQLite failed to open), or None when healthy. Drives the Settings banner.
#[tauri::command]
pub fn extension_storage_status(
    kv_state: tauri::State<'_, ExtensionKvState>,
) -> Option<String> {
    kv_state.0.degraded_reason().map(str::to_string)
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
    let extensions_cfg = crate::util::lock(&config).extensions.clone();
    let kv_store = kv_state.0.clone();
    let frecency = frecency.inner().clone();
    std::thread::spawn(move || {
        use tauri::Emitter;
        let notify_app = app.clone();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = notify_app.emit("search-invalidated", ());
        });
        sync(&registry, &extensions_cfg, &kv_store, &frecency, true, Some(notify));
        let _ = app.emit("search-invalidated", ());
        // Distinct signal so ExtensionPreview drops its cache only on a real
        // extension reload, not on every file-watcher search-invalidated.
        let _ = app.emit("extensions-reloaded", ());
    });
}

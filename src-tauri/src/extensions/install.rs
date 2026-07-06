//! Extension distribution: `.portext` packaging, two-phase install
//! (preview → confirm), the consent registry, update checks, and uninstall.
//!
//! A `.portext` is a plain zip of an extension directory with `manifest.toml`
//! at the archive root. Installation is two-phase so the user consents to the
//! *exact staged bytes*: preview downloads/extracts into a dot-prefixed
//! staging dir (invisible to discovery), validates, and reports name/version/
//! permissions/sha256; confirm atomically swaps the staged dir into place.
//! Nothing is re-fetched between the two calls.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::Digest;

use super::{extensions_dir, kv::ExtensionKv, manifest};
use crate::config::ExtensionEntry;
use crate::{ConfigState, FrecencyState, Registry};

/// Download / archive size cap (compressed).
const MAX_ARCHIVE_BYTES: u64 = 40 * 1024 * 1024;
/// Zip-bomb guards: entry count and total decompressed size.
const MAX_ENTRIES: usize = 1000;
const MAX_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

// ── consent registry ──────────────────────────────────────────────────────────

/// Where an installed extension came from - drives update checks and the
/// consent policy (dev-linked dirs auto-refresh their snapshot).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum Origin {
    Url { url: String },
    File,
    Dev,
}

/// Permission snapshot the user consented to. Compared field-wise against the
/// current manifest on every load - growth requires re-consent.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ConsentPermissions {
    pub network: Vec<String>,
    pub kv: bool,
    pub clipboard: bool,
    pub open_url: bool,
    /// May inject synthetic paste keystrokes into other applications.
    pub paste: bool,
    /// Derived from the settings schema: any `type = "secret"` setting means
    /// the extension stores secrets in the keyring - consent-relevant,
    /// especially paired with `network`.
    pub has_secrets: bool,
}

impl ConsentPermissions {
    pub fn from_manifest(m: &manifest::ExtensionManifest) -> Self {
        let mut network = m.permissions.network.clone();
        network.sort();
        Self {
            network,
            kv: m.permissions.kv,
            clipboard: m.permissions.clipboard,
            open_url: m.permissions.open_url,
            paste: m.permissions.paste,
            has_secrets: m.settings_schema.iter().any(|s| s.kind == "secret"),
        }
    }

    /// True when `other` (current manifest) asks for anything this snapshot
    /// doesn't already grant.
    pub fn grew_to(&self, other: &Self) -> bool {
        (!self.kv && other.kv)
            || (!self.clipboard && other.clipboard)
            || (!self.open_url && other.open_url)
            || (!self.paste && other.paste)
            || (!self.has_secrets && other.has_secrets)
            || other.network.iter().any(|h| !self.network.contains(h))
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConsentRecord {
    pub version: String,
    #[serde(default)]
    pub sha256: String,
    pub origin: Origin,
    #[serde(default)]
    pub permissions: ConsentPermissions,
    /// Unix seconds - avoids a datetime dependency.
    #[serde(default)]
    pub consented_at: u64,
    /// Secret setting keys ever written to the keyring for this extension.
    /// The Secret Service has no portable enumeration, so uninstall unions
    /// this with the (best-effort) manifest schema to purge entries.
    #[serde(default)]
    pub secret_keys: Vec<String>,
}

fn consents_path() -> PathBuf {
    // Lives next to the extension dirs but dot-prefixed files are not
    // directories, so discovery never trips over it.
    extensions_dir().join("consents.toml")
}

pub fn load_consents() -> HashMap<String, ConsentRecord> {
    std::fs::read_to_string(consents_path())
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_consents(consents: &HashMap<String, ConsentRecord>) -> Result<(), String> {
    let raw = toml::to_string_pretty(consents).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(extensions_dir()).map_err(|e| e.to_string())?;
    std::fs::write(consents_path(), raw).map_err(|e| e.to_string())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Records consent for `name` with the manifest's current permissions.
pub fn record_consent(
    name: &str,
    m: &manifest::ExtensionManifest,
    sha256: String,
    origin: Origin,
) -> Result<(), String> {
    let mut consents = load_consents();
    // Keyring bookkeeping survives re-consent/update - the stored secrets do.
    let secret_keys = consents
        .get(name)
        .map(|r| r.secret_keys.clone())
        .unwrap_or_default();
    consents.insert(
        name.to_string(),
        ConsentRecord {
            version: m.version.clone(),
            sha256,
            origin,
            permissions: ConsentPermissions::from_manifest(m),
            consented_at: now_unix(),
            secret_keys,
        },
    );
    save_consents(&consents)
}

/// Consent gate, called from the extension load path. Returns an error when
/// the manifest's permissions have grown past the consented snapshot.
///
/// - No record: the extension was hand-dropped; the Settings enable toggle is
///   the review step, so loading writes the initial snapshot (enable = consent).
/// - Dev origin: the user owns the code - the snapshot auto-refreshes.
pub fn check_consent(m: &manifest::ExtensionManifest) -> Result<(), String> {
    let mut consents = load_consents();
    let current = ConsentPermissions::from_manifest(m);
    match consents.get_mut(&m.name) {
        None => {
            let _ = record_consent(&m.name, m, String::new(), infer_origin(&m.name));
            Ok(())
        }
        Some(rec) => {
            if rec.permissions.grew_to(&current) {
                if matches!(rec.origin, Origin::Dev) {
                    rec.permissions = current;
                    rec.version = m.version.clone();
                    rec.consented_at = now_unix();
                    let _ = save_consents(&consents);
                    return Ok(());
                }
                return Err(
                    "permissions changed since install - review and re-enable in Settings"
                        .to_string(),
                );
            }
            Ok(())
        }
    }
}

/// Remembers that a secret was written for `name`/`key` so uninstall can
/// purge the keyring entry even if the manifest is unreadable by then.
pub fn record_secret_key(name: &str, key: &str) {
    let mut consents = load_consents();
    let Some(rec) = consents.get_mut(name) else { return };
    if rec.secret_keys.iter().any(|k| k == key) {
        return;
    }
    rec.secret_keys.push(key.to_string());
    let _ = save_consents(&consents);
}

/// Dev-linked extensions are symlinked dirs; everything else without a
/// record is a hand-dropped folder (treated as a local file install).
fn infer_origin(name: &str) -> Origin {
    let path = extensions_dir().join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_symlink() => Origin::Dev,
        _ => Origin::File,
    }
}

/// Re-snapshots consent for one extension (the Settings "allow" button after
/// a permissions-grew error).
#[tauri::command]
pub fn consent_extension_permissions(name: String) -> Result<(), String> {
    let dir = extensions_dir().join(&name);
    let (m, _) = manifest::load(&dir)?;
    // Re-snapshot preserving the recorded origin/sha (secret_keys are carried
    // forward by record_consent itself); fall back to inferred origin when the
    // record is missing.
    let (origin, sha) = load_consents()
        .get(&name)
        .map(|r| (r.origin.clone(), r.sha256.clone()))
        .unwrap_or_else(|| (infer_origin(&name), String::new()));
    record_consent(&name, &m, sha, origin)
}

// ── packaging ─────────────────────────────────────────────────────────────────

/// Development junk that never belongs in a runtime archive.
fn is_junk(name: &str) -> bool {
    matches!(name, "target" | "src" | ".cargo" | ".git" | ".gitignore" | "Cargo.toml" | "Cargo.lock")
        || name.ends_with(".portext")
        || name.starts_with('.')
}

/// Builds `<name>.portext` (in the current directory) from an extension dir
/// and returns its path + sha256. Includes manifest.toml, the wasm entry and
/// top-level asset files; excludes build/dev files.
pub fn pack(dir: &Path, m: &manifest::ExtensionManifest) -> Result<(PathBuf, String), String> {
    let out_path = PathBuf::from(format!("{}.portext", m.name));
    let file = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut wrote_manifest = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !path.is_file() || (is_junk(fname) && fname != "manifest.toml") {
            continue;
        }
        if fname == "manifest.toml" {
            wrote_manifest = true;
        }
        zip.start_file(fname, options).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut zip, &bytes).map_err(|e| e.to_string())?;
    }
    if !wrote_manifest {
        return Err("manifest.toml missing".to_string());
    }
    zip.finish().map_err(|e| e.to_string())?;

    let bytes = std::fs::read(&out_path).map_err(|e| e.to_string())?;
    Ok((out_path, hex_sha256(&bytes)))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = sha2::Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ── two-phase install ─────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct InstallPreview {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub homepage: String,
    pub permissions: ConsentPermissions,
    pub keywords: Vec<String>,
    pub sha256: String,
    pub size_bytes: u64,
    /// Set when an extension with this name is already installed.
    pub replaces: Option<ReplacesInfo>,
    /// Opaque handle for confirm/cancel - the staging dir suffix.
    pub staging_token: String,
}

#[derive(serde::Serialize)]
pub struct ReplacesInfo {
    pub old_version: String,
    /// Permissions the new version asks for that the old consent didn't grant.
    pub permissions_grew: bool,
}

fn staging_dir(token: &str) -> PathBuf {
    extensions_dir().join(format!(".staging-{token}"))
}

/// Removes leftover `.staging-*` / `.old-*` dirs (crashed installs). Called
/// once at startup.
pub fn sweep_stale_dirs() {
    let Ok(entries) = std::fs::read_dir(extensions_dir()) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if (name.starts_with(".staging-") || name.starts_with(".old-")) && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[derive(serde::Serialize, Clone)]
struct InstallProgress {
    fetched: u64,
    /// Content-Length when the server sent one; None = unknown total.
    total: Option<u64>,
}

fn fetch_archive(source: &str, app: Option<&tauri::AppHandle>) -> Result<Vec<u8>, String> {
    if let Some(rest) = source.strip_prefix("file://") {
        return std::fs::read(rest).map_err(|e| format!("{rest}: {e}"));
    }
    if source.starts_with("http://") {
        return Err("extension downloads require https".to_string());
    }
    if source.starts_with("https://") {
        use tauri::Emitter;
        let resp = ureq::get(source)
            .timeout(std::time::Duration::from_secs(60))
            .call()
            .map_err(|e| format!("download failed: {e}"))?;
        let total: Option<u64> = resp
            .header("Content-Length")
            .and_then(|s| s.parse().ok());
        // Read in chunks so the UI gets a progress bar; the cap still applies.
        let mut reader = resp.into_reader().take(MAX_ARCHIVE_BYTES + 1);
        let mut bytes = Vec::new();
        let mut buf = [0u8; 64 * 1024];
        let mut last_emit = std::time::Instant::now();
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("download failed: {e}"))?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            // Throttle events to ~10/s so a fast download can't flood IPC.
            if let Some(app) = app {
                if last_emit.elapsed() >= std::time::Duration::from_millis(100) {
                    last_emit = std::time::Instant::now();
                    let _ = app.emit(
                        "ext-install-progress",
                        InstallProgress { fetched: bytes.len() as u64, total },
                    );
                }
            }
        }
        if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
            return Err(format!("archive exceeds the {MAX_ARCHIVE_BYTES}-byte cap"));
        }
        if let Some(app) = app {
            let _ = app.emit(
                "ext-install-progress",
                InstallProgress { fetched: bytes.len() as u64, total },
            );
        }
        return Ok(bytes);
    }
    // Anything else is a local path.
    std::fs::read(source).map_err(|e| format!("{source}: {e}"))
}

/// Extracts a `.portext` zip into `dest` with hardening: enclosed names only
/// (no `..`/absolute), no symlink entries, entry-count and decompressed-size
/// caps, flat manifest required at the archive root.
fn extract_archive(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("bad archive: {e}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!("archive has more than {MAX_ENTRIES} entries"));
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("bad archive: {e}"))?;
        // Symlink entries could point future reads/writes outside the dir.
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err("archive contains a symlink entry".to_string());
        }
        let Some(rel) = entry.enclosed_name() else {
            return Err(format!("archive entry \"{}\" escapes the target dir", entry.name()));
        };
        total = total.saturating_add(entry.size());
        if total > MAX_DECOMPRESSED_BYTES {
            return Err(format!(
                "archive decompresses past the {MAX_DECOMPRESSED_BYTES}-byte cap"
            ));
        }
        let out_path = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
        // take() enforces the cap even if the zip header lied about size().
        let mut limited = entry.by_ref().take(MAX_DECOMPRESSED_BYTES);
        std::io::copy(&mut limited, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Phase 1: fetch, verify, stage and describe an extension archive. Blocking
/// I/O (network + disk) - runs on the blocking pool, never the async reactor.
#[tauri::command]
pub async fn preview_extension_install(
    app: tauri::AppHandle,
    source: String,
    expected_sha256: Option<String>,
) -> Result<InstallPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        preview_install_blocking(Some(&app), &source, expected_sha256)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn preview_install_blocking(
    app: Option<&tauri::AppHandle>,
    source: &str,
    expected_sha256: Option<String>,
) -> Result<InstallPreview, String> {
    let bytes = fetch_archive(source, app)?;
    let sha256 = hex_sha256(&bytes);
    if let Some(expected) = expected_sha256 {
        let expected = expected.trim().to_lowercase();
        if !expected.is_empty() && expected != sha256 {
            return Err(format!(
                "sha256 mismatch: expected {expected}, archive is {sha256}"
            ));
        }
    }

    let token = format!("{}-{}", std::process::id(), now_unix_nanos());
    let stage = staging_dir(&token);
    extract_archive(&bytes, &stage).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&stage);
    })?;

    // Validate exactly what was staged. manifest::load checks name-matches-dir
    // against the real dir name, which is the staging dir here - so parse the
    // manifest directly and run the same checks against a rename-target path.
    let (m, _) = manifest::load_for_install(&stage).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&stage);
    })?;

    let consents = load_consents();
    let replaces = installed_version(&m.name).map(|old_version| ReplacesInfo {
        old_version,
        permissions_grew: consents
            .get(&m.name)
            .map(|rec| {
                rec.permissions
                    .grew_to(&ConsentPermissions::from_manifest(&m))
            })
            .unwrap_or(false),
    });

    // Remember the source + hash for confirm (written into the staging dir so
    // the token is the only state the frontend must hold).
    let meta = StagingMeta { source: source.to_string(), sha256: sha256.clone() };
    let meta_raw = toml::to_string(&meta).map_err(|e| e.to_string())?;
    std::fs::write(stage.join(".staging-meta.toml"), meta_raw).map_err(|e| e.to_string())?;

    Ok(InstallPreview {
        name: m.name.clone(),
        version: m.version.clone(),
        description: m.description.clone(),
        author: m.author.clone(),
        homepage: m.homepage.clone(),
        permissions: ConsentPermissions::from_manifest(&m),
        keywords: m.commands.iter().flat_map(|c| c.keywords.clone()).collect(),
        sha256,
        size_bytes: bytes.len() as u64,
        replaces,
        staging_token: token,
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StagingMeta {
    source: String,
    sha256: String,
}

fn now_unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn installed_version(name: &str) -> Option<String> {
    let dir = extensions_dir().join(name);
    manifest::load(&dir).ok().map(|(m, _)| m.version)
}

/// Reject tokens that could traverse out of the extensions dir.
fn validate_token(token: &str) -> Result<(), String> {
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("invalid staging token".to_string());
    }
    Ok(())
}

/// Phase 2: atomically swap the staged dir into place, record consent, enable
/// the extension, and load it. Consent covers the staged bytes - nothing is
/// re-fetched here.
#[tauri::command]
pub fn confirm_extension_install(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    config: tauri::State<'_, ConfigState>,
    kv_state: tauri::State<'_, super::ExtensionKvState>,
    staging_token: String,
) -> Result<String, String> {
    validate_token(&staging_token)?;
    let stage = staging_dir(&staging_token);
    if !stage.is_dir() {
        return Err("staged install expired - start over".to_string());
    }
    let meta: StagingMeta = std::fs::read_to_string(stage.join(".staging-meta.toml"))
        .ok()
        .and_then(|raw| toml::from_str(&raw).ok())
        .ok_or_else(|| "staged install is corrupt - start over".to_string())?;
    let _ = std::fs::remove_file(stage.join(".staging-meta.toml"));
    let (m, _) = manifest::load_for_install(&stage)?;
    let name = m.name.clone();

    // Unload before the swap so no instance holds the old dir open.
    crate::util::write(&registry).set_extension(&name, None);

    let final_dir = extensions_dir().join(&name);
    let old_dir = extensions_dir().join(format!(".old-{name}"));
    let _ = std::fs::remove_dir_all(&old_dir);
    if final_dir.exists() {
        std::fs::rename(&final_dir, &old_dir)
            .map_err(|e| format!("could not move old version aside: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&stage, &final_dir) {
        // Roll back so the previous install keeps working.
        if old_dir.exists() {
            let _ = std::fs::rename(&old_dir, &final_dir);
        }
        return Err(format!("could not install: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old_dir);

    let origin = if meta.source.starts_with("https://") {
        Origin::Url { url: meta.source.clone() }
    } else {
        Origin::File
    };
    record_consent(&name, &m, meta.sha256, origin)?;

    // The install dialog's consent step is the review - land enabled.
    let extensions_cfg = {
        let mut cfg = crate::util::lock(&config);
        cfg.extensions.entry(name.clone()).or_insert_with(ExtensionEntry::default).enabled = true;
        cfg.save()?;
        cfg.extensions.clone()
    };

    let registry = registry.inner().clone();
    let kv_store = kv_state.0.clone();
    let load_name = name.clone();
    std::thread::spawn(move || {
        use tauri::Emitter;
        super::sync_one(&registry, &load_name, &extensions_cfg, &kv_store, None);
        let _ = app.emit("search-invalidated", ());
        let _ = app.emit("extensions-reloaded", ());
    });
    Ok(name)
}

#[tauri::command]
pub fn cancel_extension_install(staging_token: String) -> Result<(), String> {
    validate_token(&staging_token)?;
    let stage = staging_dir(&staging_token);
    if stage.is_dir() {
        std::fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── update check ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct UpdateCheck {
    pub current_version: String,
    /// Full preview of the fetched archive; confirm it to update, cancel to
    /// discard. None when the fetched version is not newer.
    pub preview: Option<InstallPreview>,
}

/// Re-fetches an extension from its recorded origin URL and stages it. This
/// is a full archive download (there is no version endpoint) - manual-button
/// territory, capped at 40 MB.
#[tauri::command]
pub async fn check_extension_update(name: String) -> Result<UpdateCheck, String> {
    let consents = load_consents();
    let rec = consents
        .get(&name)
        .ok_or_else(|| "no install record for this extension".to_string())?;
    let Origin::Url { url } = rec.origin.clone() else {
        return Err("installed from a local file - no update source".to_string());
    };
    let current_version =
        installed_version(&name).ok_or_else(|| "extension is not installed".to_string())?;

    let preview = tauri::async_runtime::spawn_blocking(move || {
        preview_install_blocking(None, &url, None)
    })
    .await
    .map_err(|e| e.to_string())??;

    if preview.name != name {
        let _ = cancel_extension_install(preview.staging_token);
        return Err("update source now serves a different extension".to_string());
    }
    if preview.version == current_version {
        let token = preview.staging_token;
        let _ = cancel_extension_install(token);
        return Ok(UpdateCheck { current_version, preview: None });
    }
    Ok(UpdateCheck { current_version, preview: Some(preview) })
}

// ── uninstall ─────────────────────────────────────────────────────────────────

/// Removes an extension entirely: provider, directory (or dev symlink), kv
/// data, frecency history, logs, config entry and consent record.
#[tauri::command]
pub fn uninstall_extension(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    config: tauri::State<'_, ConfigState>,
    kv_state: tauri::State<'_, super::ExtensionKvState>,
    frecency: tauri::State<'_, FrecencyState>,
    name: String,
) -> Result<(), String> {
    use tauri::Emitter;
    crate::util::write(&registry).set_extension(&name, None);

    let dir = extensions_dir().join(&name);
    // Purge keyring secrets: manifest schema keys (best effort - the dir may
    // already be broken) unioned with the keys recorded at write time.
    // Never blocks the uninstall on keyring errors.
    {
        let mut keys: std::collections::HashSet<String> = manifest::load(&dir)
            .map(|(m, _)| {
                m.settings_schema
                    .iter()
                    .filter(|s| s.kind == "secret")
                    .map(|s| s.key.clone())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(rec) = load_consents().get(&name) {
            keys.extend(rec.secret_keys.iter().cloned());
        }
        for key in keys {
            if let Err(e) = super::secrets::delete(&name, &key) {
                eprintln!("[ext:{name}] uninstall: {e}");
            }
        }
    }
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_symlink() => {
            std::fs::remove_file(&dir).map_err(|e| e.to_string())?; // dev link: never touch the target
        }
        Ok(_) => std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?,
        Err(_) => {}
    }

    let kv: &Arc<ExtensionKv> = &kv_state.0;
    kv.delete_extension(&name);
    if let Some(store) = frecency.as_ref() {
        store.delete_prefix(&format!("ext:{name}:"));
    }
    super::logs::LOGS.purge(&name);

    {
        let mut cfg = crate::util::lock(&config);
        cfg.extensions.remove(&name);
        cfg.save()?;
    }
    let mut consents = load_consents();
    if consents.remove(&name).is_some() {
        save_consents(&consents)?;
    }

    let _ = app.emit("search-invalidated", ());
    let _ = app.emit("extensions-reloaded", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        buf.into_inner()
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "portunus-install-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn extract_rejects_path_traversal() {
        let dest = tmp_dir("traversal");
        let bytes = zip_of(&[("../evil.txt", b"x")]);
        let err = extract_archive(&bytes, &dest).unwrap_err();
        assert!(err.contains("escapes"), "{err}");
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_rejects_symlink_entries() {
        let dest = tmp_dir("symlink");
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();
        zip.add_symlink("link", "/etc/passwd", options).unwrap();
        zip.finish().unwrap();
        let err = extract_archive(&buf.into_inner(), &dest).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_ok_for_flat_archive() {
        let dest = tmp_dir("flat");
        let bytes = zip_of(&[("manifest.toml", b"api = 2"), ("extension.wasm", b"\0asm")]);
        extract_archive(&bytes, &dest).unwrap();
        assert!(dest.join("manifest.toml").is_file());
        assert!(dest.join("extension.wasm").is_file());
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn consent_superset_detection() {
        let base = ConsentPermissions {
            network: vec!["a.com".into()],
            kv: true,
            clipboard: false,
            open_url: false,
            paste: false,
            has_secrets: false,
        };
        // Same or narrower: no growth.
        assert!(!base.grew_to(&base));
        assert!(!base.grew_to(&ConsentPermissions { network: vec![], kv: false, ..base.clone() }));
        // Any new grant: growth.
        assert!(base.grew_to(&ConsentPermissions { clipboard: true, ..base.clone() }));
        assert!(base.grew_to(&ConsentPermissions { has_secrets: true, ..base.clone() }));
        assert!(base.grew_to(&ConsentPermissions { paste: true, ..base.clone() }));
        assert!(base.grew_to(&ConsentPermissions {
            network: vec!["a.com".into(), "b.com".into()],
            ..base.clone()
        }));
    }

    #[test]
    fn token_validation() {
        assert!(validate_token("123-456").is_ok());
        assert!(validate_token("").is_err());
        assert!(validate_token("../x").is_err());
        assert!(validate_token("a/b").is_err());
    }
}

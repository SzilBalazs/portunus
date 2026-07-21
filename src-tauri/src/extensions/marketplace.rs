//! Official extension marketplace: fetches and caches the static `index.json`
//! published by the extensions repo, exposes it to the marketplace scope
//! provider, and installs packages from it in one shot.
//!
//! The index is the consent surface: the launcher preview shows the
//! permissions listed there, and `marketplace_install` refuses any staged
//! package that asks for more than its index entry declared. Fetching follows
//! the currency-cache pattern: refreshes happen off the search path, failures
//! keep serving the stale cache, and the cache persists across restarts.

use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{extensions_dir, install, manifest};
use crate::{ConfigState, Registry};

pub const DEFAULT_INDEX_URL: &str =
    "https://szilbalazs.github.io/portunus-extensions/index.json";
/// Index schema this build understands; bumped only on breaking changes.
const SUPPORTED_SCHEMA: u32 = 1;
/// Refetch when the cached index is older than this (scope entry / startup).
const STALE_SECS: u64 = 3600;
/// Size cap on the fetched index document.
const MAX_INDEX_BYTES: u64 = 2 * 1024 * 1024;
/// Length cap on an entry's inline icon data URI.
const MAX_ICON_BYTES: usize = 8 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

// ── index schema ──────────────────────────────────────────────────────────────

/// One extension in the published index. `permissions` mirrors the manifest's
/// consent snapshot so the launcher can show the full grant list before any
/// bytes are downloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    pub version: String,
    pub api: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub permissions: install::ConsentPermissions,
    pub download_url: String,
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub icon_data_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplaceIndex {
    pub schema: u32,
    #[serde(default)]
    pub extensions: Vec<IndexEntry>,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    /// Unix seconds of the last successful fetch (200 or 304).
    fetched_at: u64,
    #[serde(default)]
    etag: Option<String>,
    index: MarketplaceIndex,
}

fn cache_path() -> PathBuf {
    crate::paths::data_dir().join("marketplace-index.json")
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── store ─────────────────────────────────────────────────────────────────────

/// Process-wide index cache, shared by the marketplace provider, the refresh
/// commands, and the startup fetch thread.
pub struct Store {
    cache: RwLock<Option<CacheFile>>,
    /// Debounces concurrent refreshes (scope entry racing the startup fetch).
    fetching: AtomicBool,
    index_url: RwLock<String>,
}

pub fn store() -> &'static Arc<Store> {
    static STORE: OnceLock<Arc<Store>> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Store::load_from_disk()))
}

impl Store {
    pub fn empty() -> Self {
        Self {
            cache: RwLock::new(None),
            fetching: AtomicBool::new(false),
            index_url: RwLock::new(DEFAULT_INDEX_URL.to_string()),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_index(index: MarketplaceIndex) -> Self {
        let store = Self::empty();
        *crate::util::write(&store.cache) = Some(CacheFile {
            fetched_at: now_unix(),
            etag: None,
            index,
        });
        store
    }

    pub fn load_from_disk() -> Self {
        let store = Self::empty();
        if let Ok(bytes) = std::fs::read(cache_path()) {
            if let Ok(file) = serde_json::from_slice::<CacheFile>(&bytes) {
                if file.index.schema == SUPPORTED_SCHEMA {
                    *crate::util::write(&store.cache) = Some(file);
                }
            }
        }
        store
    }

    pub fn set_index_url(&self, url: &str) {
        let url = if url.trim().is_empty() { DEFAULT_INDEX_URL } else { url.trim() };
        *crate::util::write(&self.index_url) = url.to_string();
    }

    fn index_url(&self) -> String {
        crate::util::read(&self.index_url).clone()
    }

    /// True when a config override points at a non-official index. Relaxes the
    /// https-only rules (`file://` index + downloads) for local testing.
    fn custom_url(&self) -> bool {
        self.index_url() != DEFAULT_INDEX_URL
    }

    pub fn is_stale(&self) -> bool {
        match crate::util::read(&self.cache).as_ref() {
            None => true,
            Some(f) => now_unix().saturating_sub(f.fetched_at) >= STALE_SECS,
        }
    }

    pub fn entry(&self, name: &str) -> Option<IndexEntry> {
        crate::util::read(&self.cache)
            .as_ref()
            .and_then(|f| f.index.extensions.iter().find(|e| e.name == name).cloned())
    }

    /// Clones the whole entry list; the index is small (a marketplace of even
    /// hundreds of extensions is well under a megabyte).
    pub fn snapshot(&self) -> Option<Vec<IndexEntry>> {
        crate::util::read(&self.cache)
            .as_ref()
            .map(|f| f.index.extensions.clone())
    }

    /// Fetches the index if stale (or `force`), swaps it in and persists it.
    /// Blocking network I/O - never call on the search path. Returns whether
    /// the entry list actually changed. Failures keep the stale cache.
    pub fn refresh(&self, force: bool) -> Result<bool, String> {
        if !force && !self.is_stale() {
            return Ok(false);
        }
        if self.fetching.swap(true, Ordering::SeqCst) {
            return Ok(false); // another refresh is already running
        }
        let result = self.refresh_inner();
        self.fetching.store(false, Ordering::SeqCst);
        result
    }

    fn refresh_inner(&self) -> Result<bool, String> {
        let url = self.index_url();
        let custom = self.custom_url();
        let etag = crate::util::read(&self.cache)
            .as_ref()
            .and_then(|f| f.etag.clone());

        let fetched = fetch_index(&url, custom, etag.as_deref())?;
        let (index, new_etag) = match fetched {
            Fetched::NotModified => {
                // Same bytes as cached: just bump freshness.
                let mut guard = crate::util::write(&self.cache);
                if let Some(f) = guard.as_mut() {
                    f.fetched_at = now_unix();
                    persist(f);
                }
                return Ok(false);
            }
            Fetched::Index(index, etag) => (index, etag),
        };

        if index.schema != SUPPORTED_SCHEMA {
            return Err(format!(
                "index schema v{} not supported (this build understands v{SUPPORTED_SCHEMA})",
                index.schema
            ));
        }
        let mut index = index;
        index.extensions.retain(|e| match validate_entry(e, custom) {
            Ok(()) => true,
            Err(err) => {
                eprintln!("[marketplace] dropping index entry \"{}\": {err}", e.name);
                false
            }
        });

        let mut guard = crate::util::write(&self.cache);
        let changed = guard.as_ref().map(|f| f.index != index).unwrap_or(true);
        let file = CacheFile { fetched_at: now_unix(), etag: new_etag, index };
        persist(&file);
        *guard = Some(file);
        Ok(changed)
    }
}

enum Fetched {
    NotModified,
    Index(MarketplaceIndex, Option<String>),
}

fn fetch_index(url: &str, custom: bool, etag: Option<&str>) -> Result<Fetched, String> {
    if let Some(rest) = url.strip_prefix("file://") {
        if !custom {
            return Err("file:// index requires a custom [marketplace] index_url".to_string());
        }
        let bytes = std::fs::read(rest).map_err(|e| format!("{rest}: {e}"))?;
        let index = parse_index(&bytes)?;
        return Ok(Fetched::Index(index, None));
    }
    if !url.starts_with("https://") {
        return Err("marketplace index_url must be https:// (or file:// for testing)".to_string());
    }
    let mut req = ureq::get(url).timeout(FETCH_TIMEOUT);
    if let Some(etag) = etag {
        req = req.set("If-None-Match", etag);
    }
    let resp = req.call().map_err(|e| format!("index fetch failed: {e}"))?;
    if resp.status() == 304 {
        return Ok(Fetched::NotModified);
    }
    let new_etag = resp.header("ETag").map(|s| s.to_string());
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("index fetch failed: {e}"))?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(format!("index exceeds the {MAX_INDEX_BYTES}-byte cap"));
    }
    Ok(Fetched::Index(parse_index(&bytes)?, new_etag))
}

fn parse_index(bytes: &[u8]) -> Result<MarketplaceIndex, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("bad index: {e}"))
}

/// Per-entry sanity: anything that fails here is dropped from the cached index
/// (one malformed entry must not take the whole marketplace down).
fn validate_entry(e: &IndexEntry, custom_url: bool) -> Result<(), String> {
    if e.name.is_empty()
        || e.name.len() > 64
        || !e.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("invalid name".to_string());
    }
    if e.version.is_empty() {
        return Err("missing version".to_string());
    }
    if e.sha256.len() != 64 || !e.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("sha256 must be 64 hex chars".to_string());
    }
    let dl_ok = e.download_url.starts_with("https://")
        || (custom_url && e.download_url.starts_with("file://"));
    if !dl_ok {
        return Err("download_url must be https://".to_string());
    }
    if e.size_bytes > install::MAX_ARCHIVE_BYTES {
        return Err("size exceeds the archive cap".to_string());
    }
    if let Some(icon) = &e.icon_data_uri {
        if icon.len() > MAX_ICON_BYTES {
            return Err("icon data URI too large".to_string());
        }
        const MIMES: [&str; 3] = [
            "data:image/png;base64,",
            "data:image/jpeg;base64,",
            "data:image/webp;base64,",
        ];
        if !MIMES.iter().any(|p| icon.starts_with(p)) {
            return Err("icon must be a png/jpeg/webp data URI".to_string());
        }
    }
    Ok(())
}

fn persist(file: &CacheFile) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_vec(file) {
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

// ── versions & updates ────────────────────────────────────────────────────────

/// True when the index version should be offered as an update over the
/// installed one. Dot-separated numeric compare; when either side has a
/// non-numeric segment, any difference counts as newer (the index is
/// authoritative for what the marketplace currently ships).
pub fn version_newer(index_v: &str, installed_v: &str) -> bool {
    if index_v == installed_v {
        return false;
    }
    let parse = |v: &str| -> Option<Vec<u64>> {
        v.trim().split('.').map(|s| s.parse::<u64>().ok()).collect()
    };
    match (parse(index_v), parse(installed_v)) {
        (Some(a), Some(b)) => {
            let len = a.len().max(b.len());
            for i in 0..len {
                let x = a.get(i).copied().unwrap_or(0);
                let y = b.get(i).copied().unwrap_or(0);
                if x != y {
                    return x > y;
                }
            }
            false
        }
        _ => true,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub name: String,
    pub installed_version: String,
    pub index_version: String,
    /// The new version asks for permissions the consented snapshot lacks.
    pub permissions_grew: bool,
    /// The new version's full permission set (from the index entry), so the
    /// Settings card can diff it against the consented snapshot and gate a
    /// grown `spawn` allowlist behind the same acknowledgement the install
    /// dialog enforces - not just a passive badge.
    pub permissions: install::ConsentPermissions,
}

/// Updates for installed marketplace-origin extensions, from the cached index.
pub fn updates_available() -> Vec<UpdateInfo> {
    let Some(entries) = store().snapshot() else { return vec![] };
    let consents = install::load_consents();
    let mut out = Vec::new();
    for (name, rec) in &consents {
        if !matches!(rec.origin, install::Origin::Marketplace) {
            continue;
        }
        let Some(entry) = entries.iter().find(|e| &e.name == name) else { continue };
        if entry.api != manifest::SUPPORTED_API {
            continue; // needs a newer Portunus; not offered as an update
        }
        let installed_version = manifest::load(&extensions_dir().join(name))
            .map(|(m, _)| m.version)
            .unwrap_or_else(|_| rec.version.clone());
        if version_newer(&entry.version, &installed_version) {
            out.push(UpdateInfo {
                name: name.clone(),
                installed_version,
                index_version: entry.version.clone(),
                permissions_grew: rec.permissions.grew_to(&entry.permissions),
                permissions: entry.permissions.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ── Tauri commands ────────────────────────────────────────────────────────────

/// Refreshes the index if stale (or `force`). Emits `search-invalidated` +
/// `marketplace-index-updated` when the entry list changed.
#[tauri::command]
pub async fn marketplace_refresh(app: tauri::AppHandle, force: bool) -> Result<bool, String> {
    let changed = tauri::async_runtime::spawn_blocking(move || store().refresh(force))
        .await
        .map_err(|e| e.to_string())??;
    if changed {
        use tauri::Emitter;
        let _ = app.emit("search-invalidated", ());
        let _ = app.emit("marketplace-index-updated", ());
    }
    Ok(changed)
}

#[tauri::command]
pub fn marketplace_updates() -> Vec<UpdateInfo> {
    updates_available()
}

fn installing() -> &'static Mutex<HashSet<String>> {
    static INSTALLING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    INSTALLING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// One-shot install (or update) of `name` from the cached index. The launcher
/// preview already showed the index-listed permissions - that is the consent
/// step - so this downloads, verifies and installs without a staging
/// round-trip. The staged package must not ask for anything beyond its index
/// entry, and its bytes are pinned by the index sha256.
#[tauri::command]
pub async fn marketplace_install(
    app: tauri::AppHandle,
    registry: tauri::State<'_, Registry>,
    config: tauri::State<'_, ConfigState>,
    kv_state: tauri::State<'_, super::ExtensionKvState>,
    name: String,
) -> Result<String, String> {
    if !crate::util::lock(installing()).insert(name.clone()) {
        return Err(format!("{name} is already installing"));
    }
    let result = install_inner(
        app,
        registry.inner().clone(),
        config.inner().clone(),
        kv_state.0.clone(),
        name.clone(),
    )
    .await;
    crate::util::lock(installing()).remove(&name);
    result
}

async fn install_inner(
    app: tauri::AppHandle,
    registry: Registry,
    config: ConfigState,
    kv: Arc<super::kv::ExtensionKv>,
    name: String,
) -> Result<String, String> {
    let dir = extensions_dir().join(&name);
    if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_symlink()) {
        return Err(format!(
            "{name} is dev-linked - unlink it before installing from the marketplace"
        ));
    }

    // Stage from the cached index entry. A stale cache serves moved shas (a
    // republished package), pulled download URLs (a version bump) or a version
    // that disagrees with the package - all of which surface only here. On any
    // of those, force-refresh the index once and retry against the fresh entry
    // before giving up, so the user isn't dead-ended on a recoverable staleness.
    let mut refreshed = false;
    let (entry, preview) = loop {
        let entry = store()
            .entry(&name)
            .ok_or_else(|| format!("{name} is not in the marketplace index"))?;
        if entry.api != manifest::SUPPORTED_API {
            return Err(format!(
                "{name} requires extension API v{} - update Portunus first",
                entry.api
            ));
        }

        let fetch_app = app.clone();
        let url = entry.download_url.clone();
        let sha = entry.sha256.clone();
        let staged = tauri::async_runtime::spawn_blocking(move || {
            install::preview_install_blocking(Some(&fetch_app), &url, Some(sha))
        })
        .await
        .map_err(|e| e.to_string())?;

        match staged {
            // Index and package agree: proceed to the (fatal) consent checks.
            Ok(preview) if preview.version == entry.version => break (entry, preview),
            // Version drift is a stale-index symptom; heal and retry.
            Ok(preview) => {
                let _ = install::cancel_extension_install(preview.staging_token);
                if refreshed {
                    return Err("index and package are out of sync - try again shortly".to_string());
                }
            }
            // Download / sha failures: heal and retry, else surface the error.
            Err(e) => {
                if refreshed {
                    return Err(e);
                }
            }
        }

        refreshed = true;
        let changed = tauri::async_runtime::spawn_blocking(|| store().refresh(true))
            .await
            .map_err(|e| e.to_string())??;
        if changed {
            use tauri::Emitter;
            let _ = app.emit("search-invalidated", ());
            let _ = app.emit("marketplace-index-updated", ());
        }
    };

    // Consent-surface checks are fatal - a refresh can't make a wrong-name or
    // over-asking package safe to install. Aborts and cleans the staging dir.
    let fail = |token: String, msg: String| -> Result<String, String> {
        let _ = install::cancel_extension_install(token);
        Err(msg)
    };
    if preview.name != name {
        return fail(
            preview.staging_token,
            format!("package serves \"{}\", not \"{name}\"", preview.name),
        );
    }
    if entry.permissions.grew_to(&preview.permissions) {
        return fail(
            preview.staging_token,
            "package requests permissions not listed in the marketplace - install aborted"
                .to_string(),
        );
    }

    let token = preview.staging_token.clone();
    tauri::async_runtime::spawn_blocking(move || {
        install::confirm_install_core(
            app,
            &registry,
            &config,
            &kv,
            &token,
            Some(install::Origin::Marketplace),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, version: &str) -> IndexEntry {
        IndexEntry {
            name: name.to_string(),
            version: version.to_string(),
            api: manifest::SUPPORTED_API,
            description: String::new(),
            author: String::new(),
            homepage: String::new(),
            keywords: vec![],
            permissions: install::ConsentPermissions::default(),
            download_url: "https://example.org/pkg.portext".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 1024,
            icon_data_uri: None,
        }
    }

    #[test]
    fn version_compare() {
        assert!(version_newer("1.2.10", "1.2.9"));
        assert!(version_newer("0.10", "0.9"));
        assert!(version_newer("1.0.1", "1.0"));
        assert!(!version_newer("1.2.9", "1.2.10"));
        assert!(!version_newer("1.0", "1.0"));
        assert!(!version_newer("1.0", "1.0.0"));
        // Unparsable segments: any difference counts as newer.
        assert!(version_newer("1.0-beta", "1.0"));
        assert!(!version_newer("1.0-beta", "1.0-beta"));
    }

    #[test]
    fn entry_validation() {
        assert!(validate_entry(&entry("emoji", "1.0"), false).is_ok());
        assert!(validate_entry(&entry("../evil", "1.0"), false).is_err());
        assert!(validate_entry(&IndexEntry { sha256: "zz".into(), ..entry("a", "1.0") }, false).is_err());
        assert!(validate_entry(
            &IndexEntry { download_url: "http://x.org/p".into(), ..entry("a", "1.0") },
            false
        )
        .is_err());
        // file:// downloads only with a custom index url.
        let file_dl = IndexEntry { download_url: "file:///tmp/p".into(), ..entry("a", "1.0") };
        assert!(validate_entry(&file_dl, false).is_err());
        assert!(validate_entry(&file_dl, true).is_ok());
        let big_icon = IndexEntry {
            icon_data_uri: Some(format!("data:image/png;base64,{}", "A".repeat(MAX_ICON_BYTES))),
            ..entry("a", "1.0")
        };
        assert!(validate_entry(&big_icon, false).is_err());
        let bad_mime = IndexEntry {
            icon_data_uri: Some("data:image/svg+xml;base64,PHN2Zz4=".into()),
            ..entry("a", "1.0")
        };
        assert!(validate_entry(&bad_mime, false).is_err());
    }

    #[test]
    fn index_parse_roundtrip() {
        let idx = MarketplaceIndex { schema: 1, extensions: vec![entry("emoji", "0.2.0")] };
        let json = serde_json::to_vec(&idx).unwrap();
        let back = parse_index(&json).unwrap();
        assert_eq!(back, idx);
        // Unknown fields are ignored (additive schema evolution).
        let raw = r#"{"schema":1,"extensions":[],"generated_at":"2026-07-16"}"#;
        assert!(parse_index(raw.as_bytes()).is_ok());
    }

    #[test]
    fn store_lookup() {
        let s = Store::with_index(MarketplaceIndex {
            schema: 1,
            extensions: vec![entry("emoji", "0.2.0"), entry("gh", "1.1.0")],
        });
        assert!(!s.is_stale());
        assert_eq!(s.entry("gh").unwrap().version, "1.1.0");
        assert!(s.entry("nope").is_none());
        assert_eq!(s.snapshot().unwrap().len(), 2);
    }
}

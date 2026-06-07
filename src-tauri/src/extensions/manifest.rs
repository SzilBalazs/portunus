//! Extension manifest parsing and validation.
//!
//! Every extension directory must contain a `manifest.toml` declaring the wire
//! API major it targets, its identity, and the permissions it wants. Anything
//! that fails validation is surfaced to the Settings UI instead of loading.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Wire-contract major this host implements (mirrors portunus-ext-sdk).
pub const SUPPORTED_API: u32 = portunus_ext_sdk::API_VERSION;

/// Hard cap on `extension.wasm` size — anything bigger is rejected at load.
const MAX_WASM_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionManifest {
    /// Required wire API major; host refuses unknown values.
    pub api: u32,
    /// Must match the directory name; used for id/kv namespacing.
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// `SearchResult.kind` values this extension emits; defaults to `ext-<name>`.
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub limits: Limits,
    /// Present = the host schedules the extension's `refresh` export.
    pub background: Option<BackgroundConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackgroundConfig {
    pub refresh_interval_secs: u64,
}

impl BackgroundConfig {
    /// Interval clamped to host-enforced bounds (1 min – 1 day).
    pub fn interval_secs(&self) -> u64 {
        self.refresh_interval_secs.clamp(60, 86_400)
    }
}

fn default_entry() -> String {
    "extension.wasm".to_string()
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Hostnames the extension may reach via Extism's built-in HTTP.
    pub network: Vec<String>,
    pub kv: bool,
    pub clipboard: bool,
    /// May open http(s) URLs in the default browser.
    pub open_url: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub search_timeout_ms: u64,
    pub activate_timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            search_timeout_ms: 150,
            activate_timeout_ms: 2000,
        }
    }
}

impl Limits {
    /// Budgets clamped to host-enforced ranges; a manifest can lower but never
    /// blow past them (search sits on the keystroke path).
    pub fn clamped(&self) -> Self {
        Self {
            search_timeout_ms: self.search_timeout_ms.clamp(10, 500),
            activate_timeout_ms: self.activate_timeout_ms.clamp(10, 10_000),
        }
    }
}

impl ExtensionManifest {
    pub fn default_kind(&self) -> String {
        self.kinds
            .first()
            .cloned()
            .unwrap_or_else(|| format!("ext-{}", self.name))
    }
}

/// Loads and validates `<dir>/manifest.toml`, returning the manifest plus the
/// canonicalized, escape-checked path of the wasm entry file.
pub fn load(dir: &Path) -> Result<(ExtensionManifest, PathBuf), String> {
    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "extension directory has a non-UTF-8 name".to_string())?;

    let raw = std::fs::read_to_string(dir.join("manifest.toml"))
        .map_err(|e| format!("manifest.toml: {e}"))?;
    let manifest: ExtensionManifest =
        toml::from_str(&raw).map_err(|e| format!("manifest.toml: {e}"))?;

    if manifest.api != SUPPORTED_API {
        return Err(format!(
            "requires API v{}, this Portunus supports v{SUPPORTED_API}",
            manifest.api
        ));
    }
    if manifest.name != dir_name {
        return Err(format!(
            "manifest name \"{}\" must match directory name \"{dir_name}\"",
            manifest.name
        ));
    }
    // Name feeds the `ext:<name>:<id>` grammar and kv namespacing — keep it tame.
    if manifest.name.is_empty()
        || !manifest
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("name may only contain ASCII letters, digits, '-' and '_'".to_string());
    }
    if manifest.entry.contains('/') || manifest.entry.contains("..") {
        return Err("entry may not contain path separators".to_string());
    }
    // Extism's allowed_hosts accepts glob patterns — a wildcard would grant
    // unrestricted outbound HTTP while looking innocuous in the permission UI.
    for host in &manifest.permissions.network {
        if host.is_empty() || host.contains('*') || host.contains('?') {
            return Err(format!(
                "network permission \"{host}\": wildcards are not allowed — list exact hosts"
            ));
        }
    }
    // Kinds drive frontend grouping/icons; without the ext- prefix an
    // extension's results could visually impersonate native apps or files.
    for kind in &manifest.kinds {
        if !kind.starts_with("ext-") {
            return Err(format!("kind \"{kind}\" must start with \"ext-\""));
        }
    }

    let wasm_path = dir.join(&manifest.entry);
    let canon_dir = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve extension dir: {e}"))?;
    let canon_wasm = wasm_path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", manifest.entry))?;
    if !canon_wasm.starts_with(&canon_dir) {
        return Err("entry resolves outside the extension directory".to_string());
    }
    let size = std::fs::metadata(&canon_wasm)
        .map_err(|e| format!("{}: {e}", manifest.entry))?
        .len();
    if size > MAX_WASM_BYTES {
        return Err(format!(
            "{} is {size} bytes, exceeds the {MAX_WASM_BYTES}-byte cap",
            manifest.entry
        ));
    }

    Ok((manifest, canon_wasm))
}

//! Extension manifest parsing and validation.
//!
//! Every extension directory must contain a `manifest.toml` declaring the wire
//! API major it targets, its identity, and the permissions it wants. Anything
//! that fails validation is surfaced to the Settings UI instead of loading.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Wire-contract major this host implements (mirrors portunus-ext-sdk).
pub const SUPPORTED_API: u32 = portunus_ext_sdk::API_VERSION;

/// Hard cap on `extension.wasm` size - anything bigger is rejected at load.
const MAX_WASM_BYTES: u64 = 32 * 1024 * 1024;

/// Upper bound on a command's `min_query_len`. A larger value almost certainly
/// means the author never meant to gate that aggressively, so it is rejected at
/// parse time rather than silently clamped.
pub const MAX_MIN_QUERY_LEN: usize = 32;

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
    /// Informational project URL, shown in Settings.
    #[serde(default)]
    pub homepage: String,
    /// `SearchResult.kind` values this extension emits; defaults to `ext-<name>`.
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default = "default_entry")]
    pub entry: String,
    /// The extension's commands - searchable launcher entries, each with its
    /// own title, search keywords and behavior. At least one is required.
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub limits: Limits,
    /// Present = the host schedules the extension's `refresh` export.
    pub background: Option<BackgroundConfig>,
    /// User-configurable settings, rendered by the host Settings UI and read
    /// by the extension via the `settings_get` host function.
    #[serde(default, rename = "settings")]
    pub settings_schema: Vec<SettingSpec>,
}

/// One `[[commands]]` entry - a searchable launcher command owned by this
/// extension. `search`/`query`/`activate`/`preview` all receive the command
/// name on the wire so one wasm module serves every command.
#[derive(Debug, Clone, Deserialize)]
pub struct CommandSpec {
    /// Identifier on the wire and in `ext:<ext>:cmd:<name>` ids; `[a-z0-9_-]+`,
    /// unique within the extension.
    pub name: String,
    /// Root-search entry title ("Search Repositories").
    pub title: String,
    /// Optional context line shown on the entry row.
    #[serde(default)]
    pub description: String,
    /// "scope" (enter-able mode with a chip) or "action" (one-shot Enter).
    /// "inline" is accepted as a legacy alias for "scope".
    #[serde(default = "default_command_mode")]
    pub mode: String,
    /// Search synonyms folded into the command's fuzzy match alongside its
    /// title, so `keywords = ["emoji", "smiley"]` makes the entry findable by
    /// either. Free-text terms - they never gate or trigger execution.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Minimum (post-strip) query length before `search` is called.
    #[serde(default)]
    pub min_query_len: usize,
    /// Inline only: run on every keystroke like a built-in provider
    /// (discouraged; costs a wasm call per keystroke).
    #[serde(default)]
    pub always: bool,
    /// Short label for the active-mode chip; defaults to the title.
    #[serde(default)]
    pub chip: String,
    /// Input placeholder while the command's mode is active.
    #[serde(default)]
    pub placeholder: String,
    /// `SearchResult.kind` override for this command's results; defaults to
    /// the extension's default kind. Must be listed in `kinds`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Optional icon for the command's root-search entry: a bare filename of a
    /// bundled asset (a base64-encoded PNG, by convention a `.b64` file). The
    /// host reads it once at load and shows it in place of the generic command
    /// glyph. Absent = the generic glyph.
    #[serde(default)]
    pub icon: Option<String>,
}

fn default_command_mode() -> String {
    "scope".to_string()
}

impl CommandSpec {
    pub fn min_len(&self) -> usize {
        self.min_query_len.min(MAX_MIN_QUERY_LEN)
    }

    pub fn chip_label(&self) -> String {
        if self.chip.is_empty() { self.title.clone() } else { self.chip.clone() }
    }
}

/// One `[[settings]]` entry - a typed, user-editable option.
/// Serialized to the Settings UI as-is (part of `list_extensions`).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct SettingSpec {
    /// Identifier the extension reads via `settings_get`; `[a-z0-9_]+`.
    pub key: String,
    /// Type tag: "string" | "bool" | "number" | "select" | "secret".
    /// `secret` values live in the system keyring (never config.toml) and
    /// render as a masked input; declaring one is consent-relevant.
    #[serde(rename = "type")]
    pub kind: String,
    /// Label shown in the Settings UI.
    pub label: String,
    /// Optional helper text under the label.
    #[serde(default)]
    pub description: String,
    /// Default value; must match the declared type.
    #[serde(default)]
    pub default: Option<toml::Value>,
    /// select: allowed values.
    #[serde(default)]
    pub options: Vec<String>,
    /// number: optional bounds/step for the UI stepper.
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    /// string: optional input placeholder.
    #[serde(default)]
    pub placeholder: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackgroundConfig {
    pub refresh_interval_secs: u64,
}

impl BackgroundConfig {
    /// Interval clamped to host-enforced bounds (1 min - 1 day).
    pub fn interval_secs(&self) -> u64 {
        self.refresh_interval_secs.clamp(60, 86_400)
    }
}

fn default_entry() -> String {
    "extension.wasm".to_string()
}

const VALID_COMMAND_MODES: [&str; 3] = ["inline", "scope", "action"];

const VALID_SETTING_TYPES: [&str; 5] = ["string", "bool", "number", "select", "secret"];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Hostnames the extension may reach via Extism's built-in HTTP.
    pub network: Vec<String>,
    pub kv: bool,
    pub clipboard: bool,
    /// May open http(s) URLs in the default browser.
    pub open_url: bool,
    /// May use the `Paste` activate effect (clipboard write + synthetic
    /// Ctrl+V into the previously focused window).
    pub paste: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub search_timeout_ms: u64,
    pub activate_timeout_ms: u64,
    /// Budget for the optional async `query` export (off the keystroke path,
    /// dedicated instance) - generous by design, network I/O belongs here.
    pub query_timeout_ms: u64,
    /// Budget for the optional `preview` export. Streaming previews (LLM
    /// output, slow APIs) raise this; plain previews keep the tight default.
    pub preview_timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            search_timeout_ms: 150,
            activate_timeout_ms: 2000,
            query_timeout_ms: 10_000,
            preview_timeout_ms: 500,
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
            query_timeout_ms: self.query_timeout_ms.clamp(500, 60_000),
            preview_timeout_ms: self.preview_timeout_ms.clamp(100, 10_000),
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
    load_impl(dir, true)
}

/// Same validation as [`load`] minus the name-matches-directory check - for
/// staged installs, whose directory is a random `.staging-*` name. The
/// install path renames the dir to the manifest name before it goes live.
pub fn load_for_install(dir: &Path) -> Result<(ExtensionManifest, PathBuf), String> {
    load_impl(dir, false)
}

fn load_impl(dir: &Path, check_dir_name: bool) -> Result<(ExtensionManifest, PathBuf), String> {
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
            "requires API v{}, this Portunus supports v{SUPPORTED_API}{}",
            manifest.api,
            if manifest.api == 3 {
                " - migrate the [trigger] section to one or more [[commands]] entries"
            } else {
                ""
            }
        ));
    }
    if check_dir_name && manifest.name != dir_name {
        return Err(format!(
            "manifest name \"{}\" must match directory name \"{dir_name}\"",
            manifest.name
        ));
    }
    // Name feeds the `ext:<name>:<id>` grammar and kv namespacing - keep it tame.
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
    // Extism's allowed_hosts accepts glob patterns - a wildcard would grant
    // unrestricted outbound HTTP while looking innocuous in the permission UI.
    for host in &manifest.permissions.network {
        if host.is_empty() || host.contains('*') || host.contains('?') {
            return Err(format!(
                "network permission \"{host}\": wildcards are not allowed - list exact hosts"
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
    validate_commands(&manifest)?;
    validate_settings_schema(&manifest.settings_schema)?;

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

fn validate_commands(manifest: &ExtensionManifest) -> Result<(), String> {
    if manifest.commands.is_empty() {
        return Err(
            "at least one [[commands]] entry is required (api 3 manifests: migrate [trigger] to [[commands]])"
                .to_string(),
        );
    }
    let mut names = std::collections::HashSet::new();
    for cmd in &manifest.commands {
        if cmd.name.is_empty()
            || cmd.name.len() > 64
            || !cmd
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(format!(
                "command name \"{}\": lowercase ASCII letters, digits, '-', '_' only (max 64 chars)",
                cmd.name
            ));
        }
        if !names.insert(cmd.name.as_str()) {
            return Err(format!("duplicate command name \"{}\"", cmd.name));
        }
        if cmd.title.trim().is_empty() {
            return Err(format!("command \"{}\": title is required", cmd.name));
        }
        if !VALID_COMMAND_MODES.contains(&cmd.mode.as_str()) {
            return Err(format!(
                "command \"{}\": mode \"{}\" is not one of inline/scope/action",
                cmd.name, cmd.mode
            ));
        }
        if cmd.min_query_len > MAX_MIN_QUERY_LEN {
            return Err(format!(
                "command \"{}\": min_query_len = {} exceeds the max of {MAX_MIN_QUERY_LEN}",
                cmd.name, cmd.min_query_len
            ));
        }
        if cmd.always && cmd.mode == "action" {
            return Err(format!(
                "command \"{}\": always = true does not apply to action commands",
                cmd.name
            ));
        }
        if let Some(kind) = &cmd.kind {
            if !manifest.kinds.contains(kind) && *kind != format!("ext-{}", manifest.name) {
                return Err(format!(
                    "command \"{}\": kind \"{kind}\" is not listed in kinds",
                    cmd.name
                ));
            }
        }
        for k in &cmd.keywords {
            if k.trim().is_empty() || k.len() > 64 {
                return Err(format!(
                    "command \"{}\": keyword \"{k}\" must be non-empty and at most 64 chars",
                    cmd.name
                ));
            }
        }
    }
    Ok(())
}

fn validate_settings_schema(schema: &[SettingSpec]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for s in schema {
        if s.key.is_empty()
            || s.key.len() > 64
            || !s
                .key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!(
                "setting key \"{}\": lowercase ASCII letters, digits and '_' only (max 64 chars)",
                s.key
            ));
        }
        if !seen.insert(s.key.as_str()) {
            return Err(format!("duplicate setting key \"{}\"", s.key));
        }
        if !VALID_SETTING_TYPES.contains(&s.kind.as_str()) {
            return Err(format!(
                "setting \"{}\": type \"{}\" is not one of string/bool/number/select/secret",
                s.key, s.kind
            ));
        }
        if s.label.is_empty() {
            return Err(format!("setting \"{}\": label is required", s.key));
        }
        if s.kind == "select" {
            if s.options.is_empty() {
                return Err(format!("setting \"{}\": select needs options", s.key));
            }
        } else if !s.options.is_empty() {
            return Err(format!("setting \"{}\": options only apply to select", s.key));
        }
        if s.kind == "secret" {
            // A default secret in a public manifest is always a bug.
            if s.default.is_some() {
                return Err(format!(
                    "setting \"{}\": secret settings cannot declare a default",
                    s.key
                ));
            }
            if s.min.is_some() || s.max.is_some() || s.step.is_some() {
                return Err(format!(
                    "setting \"{}\": min/max/step do not apply to secret",
                    s.key
                ));
            }
        }
        if let Some(default) = &s.default {
            coerce_setting_value(s, &toml_to_json(default)).map_err(|e| {
                format!("setting \"{}\": default {e}", s.key)
            })?;
        }
    }
    Ok(())
}

fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
}

/// Validates/coerces one JSON value against a setting spec. Returns the
/// canonical value to store, or an error naming the mismatch.
pub fn coerce_setting_value(
    spec: &SettingSpec,
    value: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    match spec.kind.as_str() {
        "string" => value
            .as_str()
            .map(|s| serde_json::Value::String(s.to_string()))
            .ok_or_else(|| "must be a string".to_string()),
        // Secrets travel through the dedicated secret commands only; the
        // coercion here backs their validation (the config path never sees
        // secret values - resolve_settings skips them).
        "secret" => {
            let s = value.as_str().ok_or_else(|| "must be a string".to_string())?;
            if s.is_empty() {
                return Err("must not be empty".to_string());
            }
            Ok(serde_json::Value::String(s.to_string()))
        }
        "bool" => value
            .as_bool()
            .map(serde_json::Value::Bool)
            .ok_or_else(|| "must be a bool".to_string()),
        "number" => {
            let n = value.as_f64().ok_or_else(|| "must be a number".to_string())?;
            if !n.is_finite() {
                return Err("must be finite".to_string());
            }
            if let Some(min) = spec.min {
                if n < min {
                    return Err(format!("must be >= {min}"));
                }
            }
            if let Some(max) = spec.max {
                if n > max {
                    return Err(format!("must be <= {max}"));
                }
            }
            serde_json::Number::from_f64(n)
                .map(serde_json::Value::Number)
                .ok_or_else(|| "must be a number".to_string())
        }
        "select" => {
            let s = value.as_str().ok_or_else(|| "must be a string".to_string())?;
            if !spec.options.iter().any(|o| o == s) {
                return Err(format!("\"{s}\" is not one of the declared options"));
            }
            Ok(serde_json::Value::String(s.to_string()))
        }
        _ => Err("unknown setting type".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_clamp_to_host_ranges() {
        let l = Limits {
            search_timeout_ms: 5_000,
            activate_timeout_ms: 1,
            query_timeout_ms: 999_999,
            preview_timeout_ms: 1,
        }
        .clamped();
        assert_eq!(l.search_timeout_ms, 500);
        assert_eq!(l.activate_timeout_ms, 10);
        assert_eq!(l.query_timeout_ms, 60_000);
        assert_eq!(l.preview_timeout_ms, 100);
    }

    #[test]
    fn secret_setting_rejects_default() {
        let spec = SettingSpec {
            key: "api_key".into(),
            kind: "secret".into(),
            label: "API key".into(),
            description: String::new(),
            default: Some(toml::Value::String("x".into())),
            options: vec![],
            min: None,
            max: None,
            step: None,
            placeholder: String::new(),
        };
        assert!(validate_settings_schema(&[spec]).is_err());
    }

    #[test]
    fn resolve_settings_skips_secrets() {
        let spec = SettingSpec {
            key: "api_key".into(),
            kind: "secret".into(),
            label: "API key".into(),
            description: String::new(),
            default: None,
            options: vec![],
            min: None,
            max: None,
            step: None,
            placeholder: String::new(),
        };
        let mut user = serde_json::Map::new();
        // Even a (malicious/buggy) config-sourced value must not resolve.
        user.insert("api_key".into(), serde_json::Value::String("leaked".into()));
        let out = resolve_settings(&[spec], &user);
        assert!(!out.contains_key("api_key"));
    }

    #[test]
    fn limits_defaults() {
        let l = Limits::default().clamped();
        assert_eq!(l.search_timeout_ms, 150);
        assert_eq!(l.activate_timeout_ms, 2000);
        assert_eq!(l.query_timeout_ms, 10_000);
        assert_eq!(l.preview_timeout_ms, 500);
    }
}

/// Resolves the effective settings map for an extension: schema defaults
/// overlaid with (schema-valid) user values. Unknown keys and type-mismatched
/// values are dropped - the wasm side always sees schema-shaped data.
pub fn resolve_settings(
    schema: &[SettingSpec],
    user: &serde_json::Map<String, serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut out = std::collections::HashMap::new();
    for spec in schema {
        // Secrets never resolve from config - they live in the keyring and
        // are overlaid by the loader (`resolved_settings_with_secrets`).
        if spec.kind == "secret" {
            continue;
        }
        let user_val = user
            .get(&spec.key)
            .and_then(|v| coerce_setting_value(spec, v).ok());
        let val = user_val.or_else(|| {
            spec.default
                .as_ref()
                .and_then(|d| coerce_setting_value(spec, &toml_to_json(d)).ok())
        });
        if let Some(v) = val {
            out.insert(spec.key.clone(), v);
        }
    }
    out
}

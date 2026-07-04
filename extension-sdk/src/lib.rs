//! Wire contract between Portunus and its WASM extensions.
//!
//! Everything crossing the extension boundary is defined here and versioned by
//! [`API_VERSION`]. Extensions declare the API major they target in their
//! `manifest.toml` (`api = 2`); the host refuses to load unknown majors.
//!
//! Extension authors: enable the default `guest` feature and see the `guest`
//! module for host-function wrappers. The Portunus host depends on this crate
//! with `default-features = false` (wire types only).

use serde::{Deserialize, Serialize};

/// Wire-contract major version. Bumped only on breaking changes.
pub const API_VERSION: u32 = 2;

/// Input to the extension's exported `search` function.
///
/// With a `[trigger]` section in the manifest, `query` arrives with the
/// matched prefix already stripped (`"emoji smi"` → `"smi"`); the raw text and
/// the prefix that matched ride alongside. In always-mode `query == raw_query`
/// and `trigger` is `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchInput {
    pub query: String,
    #[serde(default)]
    pub raw_query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
}

/// Output of the extension's exported `search` function.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchOutput {
    #[serde(default)]
    pub results: Vec<ExtensionResult>,
}

/// One search result produced by an extension.
///
/// `id` is opaque to the host; it is namespaced to `ext:<name>:<id>` before
/// entering the launcher, and the full result is passed back verbatim on
/// activate/preview, so extensions never need to persist search state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionResult {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Relevance in `0.0..=100.0`, higher = better. The host maps this into
    /// its internal score space; out-of-range values are clamped.
    #[serde(default)]
    pub relevance: f32,
    /// Available actions; the first is the default on Enter, the rest are
    /// reachable via the launcher's action picker (Alt+Enter). Empty means
    /// `activate` is called with `action: None` on Enter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// Optional small icon shown next to the result in the launcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ResultIcon>,
    /// Optional small text chip shown right-aligned on the result row
    /// (e.g. "beta", "cached", a category). Clamped by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
}

/// One user-facing action on a result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Action {
    /// Opaque id passed back to `activate` when the user picks this action.
    pub id: String,
    /// Short imperative label shown in the launcher ("Copy emoji", "Open docs").
    pub label: String,
    /// Optional muted secondary text shown next to the label in the picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Small inline icon for a search result. Same mime allowlist as image
/// previews (png/jpeg/gif/webp); capped at 32 KB base64 by the host. An
/// invalid icon is dropped (the result keeps the default glyph) - it never
/// fails the search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultIcon {
    pub mime: String,
    pub data_base64: String,
}

/// Input to the extension's exported `activate` function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateInput {
    /// The result exactly as the extension returned it from `search`.
    pub result: ExtensionResult,
    /// Action id chosen by the user, or None for the default action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// Output of the extension's exported `activate` function.
///
/// Effects are executed by the host after the call returns, in order. They
/// run only on explicit user activation - the keypress is the consent - so
/// none of them require a manifest permission. An empty list is fine (the
/// extension did its work via host functions or has nothing to do).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivateOutput {
    #[serde(default)]
    pub effects: Vec<ActivateEffect>,
}

/// Declarative side effect requested by `activate`, executed host-side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivateEffect {
    /// Put text on the system clipboard. No permission needed.
    CopyText { text: String },
    /// Open a http(s) URL in the default browser. No permission needed.
    OpenUrl { url: String },
    /// Show a brief toast in the launcher ("Copied!", "Added to list").
    ShowToast { message: String },
}

/// Input to the extension's optional exported `preview` function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewInput {
    pub result: ExtensionResult,
}

/// Input to the extension's optional exported `refresh` function.
///
/// Declared via `[background] refresh_interval_secs` in the manifest. The host
/// calls `refresh` once when the extension loads and then on the interval -
/// off the keystroke path, on a dedicated instance - so extensions can keep
/// kv-cached HTTP data warm while `search` stays offline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshInput {
    /// "load" (extension just loaded) or "scheduled" (interval tick).
    pub reason: String,
}

/// Output of the extension's optional exported `refresh` function. Errors
/// travel as traps/`Err`, so there is nothing to report on success; after a
/// successful refresh the host re-runs any open launcher query automatically.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefreshOutput {}

/// Declarative preview content rendered by the host. Extensions never ship UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PreviewContent {
    /// GitHub-flavored markdown. Raw HTML is not rendered.
    Markdown { content: String },
    /// Key-value table.
    Metadata { items: Vec<MetadataItem> },
    /// Inline image, base64-encoded. Capped at 1 MB by the host.
    Image { mime: String, data_base64: String },
    /// Simple row list with optional tag badges and monospace titles.
    List { items: Vec<ListItem> },
    /// Arbitrary HTML rendered in a sandboxed iframe. No scripts execute; no
    /// external network requests are allowed (CSP: `default-src 'none';
    /// style-src 'unsafe-inline' data:; img-src data:`). The host injects
    /// theme CSS variables and a base reset. Capped at 128 KB. Pure CSS/HTML
    /// only - use for rich layouts (weather cards, file trees, charts) the
    /// declarative types can't express.
    Html { content: String },
    /// Sequence of named sections, each a two-column command/description table.
    /// First cell per row is styled as a command (monospace); remaining cells as
    /// description text. Perfect for cheat sheets, man pages, shortcut references.
    Sections { items: Vec<SectionItem> },
    /// Monospace code block. `lang` is informational (no syntax highlighting added
    /// yet - it is reserved for future use and passed through unchanged).
    Code { lang: String, content: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataItem {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListItem {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Small badge chip shown to the right of the title (e.g. "installed", "v2.0").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Render `title` in monospace font (useful for command names, paths, etc.).
    #[serde(default)]
    pub mono: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionItem {
    /// Optional heading rendered above the rows in small-caps style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    /// Each row is a slice of cells. The first cell is the "command" (monospace,
    /// full-colour); the rest are the description (muted). A single-cell row
    /// spans both columns.
    pub rows: Vec<Vec<String>>,
}

/// Guest-side helpers: safe wrappers around the Portunus host functions.
/// Available to extensions compiled for wasm (`guest` feature, on by default).
#[cfg(feature = "guest")]
pub mod guest {
    pub use extism_pdk;
    pub use extism_pdk::{plugin_fn, FnResult, Json};

    use extism_pdk::host_fn;

    #[host_fn]
    extern "ExtismHost" {
        fn kv_get(key: String) -> Json<Option<String>>;
        fn kv_set(key: String, value: String);
        fn kv_list(prefix: String) -> Json<Vec<String>>;
        fn kv_delete(key: String);
        fn clipboard_write(text: String);
        fn now_ms() -> u64;
        fn open_url(url: String);
        fn log_message(message: String);
        fn settings_get(key: String) -> Json<Option<serde_json::Value>>;
    }

    /// Read a value from this extension's key-value store.
    /// Requires `kv = true` in the manifest permissions.
    pub fn kv_read(key: &str) -> Result<Option<String>, extism_pdk::Error> {
        let Json(v) = unsafe { kv_get(key.to_string())? };
        Ok(v)
    }

    /// Write a value to this extension's key-value store (10 MB quota).
    /// Requires `kv = true` in the manifest permissions.
    pub fn kv_write(key: &str, value: &str) -> Result<(), extism_pdk::Error> {
        unsafe { kv_set(key.to_string(), value.to_string()) }
    }

    /// Put text on the system clipboard.
    /// Requires `clipboard = true` in the manifest permissions.
    pub fn clipboard(text: &str) -> Result<(), extism_pdk::Error> {
        unsafe { clipboard_write(text.to_string()) }
    }

    /// List keys in this extension's key-value store matching a prefix
    /// (at most 10 000 returned). Requires `kv = true`.
    pub fn kv_keys(prefix: &str) -> Result<Vec<String>, extism_pdk::Error> {
        let Json(keys) = unsafe { kv_list(prefix.to_string())? };
        Ok(keys)
    }

    /// Delete a key from this extension's key-value store. Requires `kv = true`.
    pub fn kv_remove(key: &str) -> Result<(), extism_pdk::Error> {
        unsafe { kv_delete(key.to_string()) }
    }

    /// Current wall-clock time in milliseconds since the Unix epoch.
    /// (`std::time` does not work on wasm32-unknown-unknown - use this for
    /// cache timestamps.) No permission required.
    pub fn now() -> Result<u64, extism_pdk::Error> {
        unsafe { now_ms() }
    }

    /// Open a http(s) URL in the user's default browser.
    /// Requires `open_url = true` in the manifest permissions.
    pub fn open(url: &str) -> Result<(), extism_pdk::Error> {
        unsafe { open_url(url.to_string()) }
    }

    /// Write a debug line to the Portunus log (stderr), prefixed with the
    /// extension name. Capped at 4 KB per message. No permission required.
    pub fn debug(message: &str) -> Result<(), extism_pdk::Error> {
        unsafe { log_message(message.to_string()) }
    }

    /// Read one of this extension's user-configured settings (declared via
    /// `[[settings]]` in the manifest, edited in the Portunus Settings UI).
    /// Returns the user's value or the manifest default; `None` only for keys
    /// absent from the schema. No permission required.
    pub fn setting(key: &str) -> Result<Option<serde_json::Value>, extism_pdk::Error> {
        let Json(v) = unsafe { settings_get(key.to_string())? };
        Ok(v)
    }

    /// Convenience: string setting, or `None` if unset/not a string.
    pub fn setting_str(key: &str) -> Result<Option<String>, extism_pdk::Error> {
        Ok(setting(key)?.and_then(|v| v.as_str().map(str::to_string)))
    }

    /// Convenience: bool setting, or `None` if unset/not a bool.
    pub fn setting_bool(key: &str) -> Result<Option<bool>, extism_pdk::Error> {
        Ok(setting(key)?.and_then(|v| v.as_bool()))
    }

    /// Convenience: numeric setting, or `None` if unset/not a number.
    pub fn setting_num(key: &str) -> Result<Option<f64>, extism_pdk::Error> {
        Ok(setting(key)?.and_then(|v| v.as_f64()))
    }
}

//! Wire contract between Portunus and its WASM extensions.
//!
//! Everything crossing the extension boundary is defined here and versioned by
//! [`API_VERSION`]. Extensions declare the API major they target in their
//! `manifest.toml` (`api = 1`); the host refuses to load unknown majors.
//!
//! Extension authors: enable the default `guest` feature and see the `guest`
//! module for host-function wrappers. The Portunus host depends on this crate
//! with `default-features = false` (wire types only).

use serde::{Deserialize, Serialize};

/// Wire-contract major version. Bumped only on breaking changes.
pub const API_VERSION: u32 = 1;

/// Input to the extension's exported `search` function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchInput {
    pub query: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionResult {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Relevance in `0.0..=100.0`, higher = better. The host maps this into
    /// its internal score space; out-of-range values are clamped.
    #[serde(default)]
    pub relevance: f32,
    /// Optional action ids; the first is the default on Enter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateOutput {
    pub ok: bool,
}

/// Input to the extension's optional exported `preview` function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewInput {
    pub result: ExtensionResult,
}

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
    /// Simple row list.
    List { items: Vec<ListItem> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataItem {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
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
        fn clipboard_write(text: String);
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
}

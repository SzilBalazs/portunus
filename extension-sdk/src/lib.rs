//! Wire contract between Portunus and its WASM extensions.
//!
//! Everything crossing the extension boundary is defined here and versioned by
//! [`API_VERSION`]. Extensions declare the API major they target in their
//! `manifest.toml` (`api = 4`); the host refuses to load unknown majors.
//!
//! Extension authors: enable the default `guest` feature and see the `guest`
//! module for host-function wrappers. The Portunus host depends on this crate
//! with `default-features = false` (wire types only).

use serde::{Deserialize, Serialize};

/// Wire-contract major version. Bumped only on breaking changes.
pub const API_VERSION: u32 = 5;

/// Input to the extension's exported `search` function.
///
/// `command` names which of the extension's `[[commands]]` is being invoked -
/// dispatch on it when you declare more than one. `query` is the whole typed
/// term (there is no prefix stripping); `raw_query` equals it and is retained
/// for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchInput {
    /// Name of the `[[commands]]` entry being invoked.
    #[serde(default)]
    pub command: String,
    pub query: String,
    #[serde(default)]
    pub raw_query: String,
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
    /// Hint that running this action opens a form (a `ShowForm` effect)
    /// rather than dismissing the launcher. The host keeps the window
    /// visible while `activate` runs instead of hiding it optimistically,
    /// so the form doesn't flash the window hidden-then-shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub opens_form: bool,
    /// Suggested keyboard shortcut in canonical chord form
    /// (`ctrl+alt+shift+<key>`, e.g. `"ctrl+q"`) - the user can run the
    /// action directly on a selected result without opening the action
    /// picker, and can override or clear it in Settings. The host drops
    /// invalid or reserved chords, and it is ignored on the first (Enter
    /// default) action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
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

/// Input to the extension's optional exported `query` function - the async
/// search tier.
///
/// Same shape as [`SearchInput`], but a separate type so the two tiers can
/// evolve independently. `query` runs on a dedicated instance under a generous
/// budget (`[limits] query_timeout_ms`, default 10 s) and may do blocking
/// network I/O; partial batches stream to the launcher via [`guest::emit`] and
/// the return value is the final batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryInput {
    /// Name of the `[[commands]]` entry being invoked.
    #[serde(default)]
    pub command: String,
    pub query: String,
    #[serde(default)]
    pub raw_query: String,
}

/// Output of the optional exported `query` function: the final result batch.
/// May be empty when everything was already pushed via [`guest::emit`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryOutput {
    #[serde(default)]
    pub results: Vec<ExtensionResult>,
}

/// Payload of the `emit_results` host function - one partial batch pushed
/// from inside `query`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmitBatch {
    #[serde(default)]
    pub results: Vec<ExtensionResult>,
}

/// Ack returned by the `emit_results`/`emit_preview` host functions.
/// `cancelled = true` means the host no longer wants output for this call
/// (the user typed a new query or moved the selection) - stop work and return.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmitAck {
    #[serde(default)]
    pub cancelled: bool,
}

/// Input to the extension's exported `activate` function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateInput {
    /// Name of the `[[commands]]` entry the result came from. For an Action
    /// command's direct invocation, the command name with a default result.
    #[serde(default)]
    pub command: String,
    /// The result exactly as the extension returned it from `search`.
    pub result: ExtensionResult,
    /// Action id chosen by the user, or None for the default action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Values collected from a [`ActivateEffect::ShowForm`] the extension
    /// returned earlier. Present only on the follow-up call the host makes
    /// when the user submits the form (`action` is then the form's
    /// `submit_action`). Keyed by [`FormField::key`]; values are strings for
    /// text-like fields, booleans for checkboxes, numbers for number fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_values: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Output of the extension's exported `activate` function.
///
/// Effects are executed by the host after the call returns, in order. They
/// run only on explicit user activation - the keypress is the consent - so
/// none of them require a manifest permission (except [`ActivateEffect::Paste`],
/// which injects synthetic keystrokes into another application). An empty
/// list is fine (the extension did its work via host functions or has
/// nothing to do).
///
/// The host caps the list at 16 effects and honors at most one `ShowForm`.
/// Window visibility after activation resolves as: `ShowForm` > `Hide` >
/// `KeepOpen` > default (hide).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivateOutput {
    #[serde(default)]
    pub effects: Vec<ActivateEffect>,
}

impl ActivateOutput {
    /// Output with a single effect.
    pub fn single(effect: ActivateEffect) -> Self {
        Self { effects: vec![effect] }
    }

    /// Convenience: copy text to the clipboard.
    pub fn copy(text: impl Into<String>) -> Self {
        Self::single(ActivateEffect::CopyText { text: text.into() })
    }

    /// Convenience: open a URL in the default browser.
    pub fn open(url: impl Into<String>) -> Self {
        Self::single(ActivateEffect::OpenUrl { url: url.into() })
    }

    /// Convenience: replace the launcher query text. Pass "" to clear it.
    pub fn set_query(query: impl Into<String>) -> Self {
        Self::single(ActivateEffect::SetQuery { query: query.into() })
    }

    /// Convenience: show a toast at the given level.
    pub fn toast(message: impl Into<String>, level: ToastLevel) -> Self {
        Self::single(ActivateEffect::ShowToast { message: message.into(), level })
    }

    /// Convenience: spawn an OS command (argv, never a shell).
    ///
    /// **DANGER: this bypasses the wasm sandbox.** `command` must be listed in
    /// the manifest's `spawn` permission allowlist or the host drops the effect;
    /// the user is shown a hard warning before such an extension is enabled. See
    /// [`ActivateEffect::SpawnProcess`].
    pub fn spawn(command: impl Into<String>, args: Vec<String>) -> Self {
        Self::single(ActivateEffect::SpawnProcess { command: command.into(), args })
    }

    /// Convenience: open a form. The host calls `activate` again with
    /// `action = submit_action` and `form_values` filled when the user submits.
    pub fn form(
        title: impl Into<String>,
        fields: Vec<FormField>,
        submit_action: impl Into<String>,
    ) -> Self {
        Self::single(ActivateEffect::ShowForm {
            title: title.into(),
            fields,
            submit_action: submit_action.into(),
            submit_label: None,
        })
    }

    /// Append another effect (builder-style).
    pub fn and(mut self, effect: ActivateEffect) -> Self {
        self.effects.push(effect);
        self
    }
}

/// Declarative side effect requested by `activate`, executed host-side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivateEffect {
    /// Put text on the system clipboard. No permission needed.
    CopyText { text: String },
    /// Open a http(s) URL in the default browser. No permission needed.
    OpenUrl { url: String },
    /// Show a brief toast ("Copied!", "Added to list"). Shown in the launcher
    /// when it stays open, as a desktop notification when it hides. `level`
    /// tints the toast and controls how long it lingers (errors stay longer).
    ShowToast {
        message: String,
        #[serde(default)]
        level: ToastLevel,
    },
    /// Open a modal form in the launcher. When the user submits, the host
    /// calls `activate` again on the same result with `action = submit_action`
    /// and `form_values` populated; cancelling (Esc) makes no call. A submit
    /// handler may itself return another `ShowForm` for multi-step flows.
    /// Implies the window stays open. No permission needed.
    ShowForm {
        /// Modal heading. Capped at 120 chars by the host.
        title: String,
        /// Input fields, rendered in order. Capped at 32 by the host.
        fields: Vec<FormField>,
        /// Action id the host passes back on submit.
        submit_action: String,
        /// Label for the submit button; defaults to "Submit".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        submit_label: Option<String>,
    },
    /// Hide the launcher window after activation (the default when no
    /// visibility effect is present). Explicit form for clarity.
    Hide {},
    /// Keep the launcher window open after activation (e.g. toggle-style
    /// actions where the user will act again).
    KeepOpen {},
    /// Re-run the current launcher query after activation - use after
    /// mutating whatever the results reflect (delete, toggle, mark-done).
    RefreshResults {},
    /// Replace the launcher query text (`""` clears it) and re-run the current
    /// scope's search - even when the text is unchanged, so a drill-down that
    /// resets an already-empty box still refreshes. Implies the window stays
    /// open; pair with `KeepOpen`. No permission needed.
    SetQuery { query: String },
    /// Put `text` on the clipboard and inject a paste chord (Ctrl+V) into the
    /// previously focused window. Requires `paste = true` in the manifest
    /// permissions. Clobbers the clipboard; falls back to a "Copied - press
    /// Ctrl+V" notification when injection is unavailable.
    Paste { text: String },
    /// Launch an OS command, detached and fire-and-forget (no stdout/exit is
    /// returned). Arguments are passed as argv - nothing is routed through a
    /// shell.
    ///
    /// **DANGER: this bypasses the wasm sandbox.** The spawned process runs with
    /// the user's full authority. `command` must appear verbatim in the
    /// manifest's `spawn` permission allowlist; otherwise the host drops the
    /// effect and logs an error. Because a spawn allowlist is sandbox-breaking,
    /// the user is shown an explicit warning (and must confirm) before an
    /// extension requesting it is enabled. Do not request this unless it is
    /// essential to what the extension does.
    SpawnProcess { command: String, args: Vec<String> },
}

/// Toast severity; controls tint and linger time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToastLevel {
    #[default]
    Info,
    Success,
    Error,
}

/// One input field of a [`ActivateEffect::ShowForm`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FormField {
    /// Key under which the value is returned in `form_values`.
    pub key: String,
    /// Visible field label.
    pub label: String,
    /// Field kind: `text`, `textarea`, `password`, `select`, `checkbox`,
    /// or `number`. Unknown kinds are rendered as `text`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Submit is blocked while a required field is empty.
    #[serde(default)]
    pub required: bool,
    /// Placeholder for text-like fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Initial value. String for text-like fields, bool for `checkbox`,
    /// number for `number`, option value for `select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Choices for `select` fields (capped at 64 by the host).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<FormOption>,
}

impl FormField {
    /// Shorthand for a field of the given kind.
    pub fn new(key: impl Into<String>, label: impl Into<String>, kind: &str) -> Self {
        Self { key: key.into(), label: label.into(), kind: kind.to_string(), ..Self::default() }
    }

    /// Mark the field required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set the placeholder.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }
}

/// One choice of a `select` form field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FormOption {
    /// Value returned in `form_values` when chosen.
    pub value: String,
    /// Visible label; defaults to `value` when empty.
    #[serde(default)]
    pub label: String,
}

/// Input to the extension's optional exported `preview` function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewInput {
    /// Name of the `[[commands]]` entry the result came from.
    #[serde(default)]
    pub command: String,
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

/// Envelope of the `bus_request` host function: how long to wait for the
/// companion's reply, plus the request payload it receives verbatim.
/// Requires `bus = true` in the manifest permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusRequest {
    /// Reply wait in milliseconds; the host caps it at 30 000.
    pub timeout_ms: u64,
    /// Arbitrary JSON handed to the companion.
    pub payload: serde_json::Value,
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
        fn emit_results(batch: Json<super::EmitBatch>) -> Json<super::EmitAck>;
        fn emit_preview(content: Json<super::PreviewContent>) -> Json<super::EmitAck>;
        fn bus_status() -> Json<bool>;
        fn bus_request(envelope: Json<super::BusRequest>) -> String;
        fn bus_notify(payload: String);
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

    /// Push a partial result batch from inside the exported `query` function.
    /// Returns `false` when the query was cancelled (new keystroke) - stop
    /// work and return early. Calling this outside `query` is an error.
    pub fn emit(results: Vec<super::ExtensionResult>) -> Result<bool, extism_pdk::Error> {
        let Json(ack) = unsafe { emit_results(Json(super::EmitBatch { results }))? };
        Ok(!ack.cancelled)
    }

    /// Whether a companion process is attached to this extension's message
    /// bus (`ext-attach:<name>` on the portunus socket). Returns `false`
    /// (never an error) when nothing is attached or the `bus` permission is
    /// missing - branch to a fallback path on it.
    pub fn bus_attached() -> Result<bool, extism_pdk::Error> {
        let Json(v) = unsafe { bus_status()? };
        Ok(v)
    }

    /// Send a request to the attached companion and block for its reply (up
    /// to `timeout_ms`, host-capped at 30 s). Errors on timeout, detachment,
    /// or cancellation of the surrounding `query`. Requires `bus = true`.
    pub fn bus_call(
        payload: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, extism_pdk::Error> {
        let raw = unsafe { bus_request(Json(super::BusRequest { timeout_ms, payload }))? };
        Ok(serde_json::from_str(&raw)?)
    }

    /// Send a fire-and-forget message to the attached companion.
    /// Requires `bus = true`.
    pub fn bus_send(payload: serde_json::Value) -> Result<(), extism_pdk::Error> {
        unsafe { bus_notify(serde_json::to_string(&payload)?) }
    }

    /// Replace the rendered preview from inside the exported `preview`
    /// function (re-send the FULL content each time - the host swaps it
    /// wholesale, which is the right model for token-by-token accumulation).
    /// Returns `false` when the preview was cancelled (selection moved) -
    /// stop work and return early. Calling this outside `preview` is an error.
    pub fn emit_preview_update(
        content: &super::PreviewContent,
    ) -> Result<bool, extism_pdk::Error> {
        let Json(ack) = unsafe { emit_preview(Json(content.clone()))? };
        Ok(!ack.cancelled)
    }
}

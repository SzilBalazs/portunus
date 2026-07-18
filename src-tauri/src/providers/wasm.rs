//! WASM extension provider: wraps one Extism plugin instance as a `Provider`.
//!
//! Isolation guarantees, in order of importance:
//! - a trapping/hanging extension never panics or stalls the host - every call
//!   is bounded by a cancel-handle watchdog and returns `Result`;
//! - after any failed call the instance is rebuilt from disk before reuse
//!   (post-trap module state is not trustworthy);
//! - three consecutive failures pause the extension behind an escalating
//!   cooldown (see `breaker`), with the error surfaced to the Settings UI.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use extism::{CancelHandle, CompiledPlugin, Manifest as WasmManifest, Plugin, PluginBuilder, UserData, Wasm};
use portunus_ext_sdk::{
    ActivateEffect, ActivateInput, ActivateOutput, ExtensionResult, PreviewContent, PreviewInput,
    QueryInput, QueryOutput, RefreshInput, SearchInput, SearchOutput,
};

use super::breaker::FailureBreaker;
use super::ranking::{Category, MatchTier, ScoreParts};
use super::{SearchResult, EXTENSION_BAND, SCORE_EXTENSION_TRIGGERED};
use crate::extensions::hostfns::{self, ExtensionCtx, PreviewEmitSlot, QueryEmitSlot};
use crate::extensions::kv::ExtensionKv;
use crate::extensions::manifest::{CommandSpec, ExtensionManifest, Limits};
use crate::extensions::trigger::{self, GatedCommand};
use crate::util;

/// 64 MB linear-memory cap, in 64 KiB wasm pages.
const MEMORY_MAX_PAGES: u32 = 1024;
/// Raw JSON returned by a call is rejected past this size before parsing.
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
/// Per-query result cap (host truncates before scoring).
const MAX_RESULTS: usize = 200;
/// Title/subtitle clamp - UI sanity, not a wire error.
const MAX_FIELD_BYTES: usize = 2048;
/// Base64 payload cap for image previews (~1 MB decoded).
const MAX_IMAGE_B64_BYTES: usize = 1_400_000;
/// Base64 payload cap per result icon - icons are tiny and re-sent on every
/// keystroke, so this is deliberately far below the preview cap.
const MAX_ICON_B64_BYTES: usize = 32_768;
/// Mainstream formats only - an arbitrary mime would hand extension-controlled
/// bytes to whatever obscure WebKitGTK codec matches it (codec bugs are a
/// sandbox-independent attack surface).
const ALLOWED_IMAGE_MIME: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
/// Consecutive failures before the extension is paused (see `FailureBreaker`).
/// Cap on activate's effect list - a sane flow needs a handful, not dozens.
const MAX_EFFECTS: usize = 16;
/// Caps on `ShowForm` payloads and submitted `form_values`.
const MAX_FORM_TITLE_BYTES: usize = 120;
const MAX_FORM_FIELDS: usize = 32;
const MAX_FORM_OPTIONS: usize = 64;
const MAX_FORM_VALUES: usize = 32;
const MAX_FORM_VALUE_BYTES: usize = 16 * 1024;

const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// Consecutive async-`query` failures before the async tier is disabled for
/// the session. Separate from the interactive breaker: a flaky network query
/// must never take down a working sync `search`.
const MAX_QUERY_FAILURES: u32 = 5;

/// Background `refresh` budget - generous, it runs on a dedicated instance
/// off the keystroke path.
const REFRESH_TIMEOUT_SECS: u64 = 30;

/// Failure accounting for one call. Interactive calls feed the breaker;
/// background refresh surfaces errors without benching; quiet calls (async
/// query) leave all accounting to the caller, which knows whether the failure
/// was a cancellation.
#[derive(Clone, Copy, PartialEq)]
enum FailurePolicy {
    Interactive,
    Background,
    Quiet,
}

/// A call's streaming emit slot, installed into the shared [`ExtensionCtx`]
/// by [`WasmProvider::call_with_budget`] for exactly the duration of the
/// guest call (under the instance lock - see there for why).
enum EmitSlot {
    Query(QueryEmitSlot),
    Preview(PreviewEmitSlot),
}

/// One instance slot: its own compiled plugin plus the lazily (re)built
/// instance. Each slot gets a *separate* `CompiledPlugin` because extism's
/// cancellation increments the wasmtime engine's epoch, and every in-flight
/// call on that engine traps ("timeout"). Instances sharing one compiled
/// plugin share its engine - cancelling a `query` would collaterally kill a
/// concurrently-running `preview` (and vice versa). One engine per slot makes
/// cancellation slot-scoped.
struct Slot {
    compiled: CompiledPlugin,
    /// None = needs re-instantiation before the next call.
    instance: Mutex<Option<Plugin>>,
}

impl Slot {
    fn new(compiled: CompiledPlugin) -> Self {
        Self { compiled, instance: Mutex::new(None) }
    }
}

pub struct WasmProvider {
    /// Registry id: `ext:<name>`.
    reg_id: String,
    name: String,
    kind: String,
    manifest: ExtensionManifest,
    #[allow(dead_code)]
    wasm_path: PathBuf,
    /// Per-command icon `data:` URIs, keyed by command name. Precomputed from
    /// bundled asset files at load so the per-keystroke `commands()` path does
    /// no file I/O.
    command_icons: std::collections::HashMap<String, String>,
    limits: Limits,
    /// Interactive slot (search/activate).
    instance: Slot,
    /// Dedicated preview slot - preview calls can block for hundreds of ms
    /// (network I/O in the extension), which must never delay a search call or
    /// stall rapid result navigation. None if the module has no `preview`.
    preview_instance: Option<Slot>,
    /// Dedicated background slot for `refresh` - a refresh may hold its
    /// instance for seconds, which must never block a keystroke's search.
    /// None if the module has no `refresh`.
    bg_instance: Option<Slot>,
    /// Dedicated slot for the async `query` export - a query may block for
    /// seconds on network I/O. None if the module has no `query`.
    query_instance: Option<Slot>,
    /// Shared host-function context - the provider installs the streaming
    /// emit slots here around `query`/`preview` calls.
    ctx_data: UserData<ExtensionCtx>,
    /// Whether the module exports `query` (detected once at load).
    has_query: bool,
    last_error: Mutex<Option<String>>,
    breaker: FailureBreaker,
    /// Cancel handle of the in-flight `query` call, for keystroke cancellation.
    query_cancel: Mutex<Option<CancelHandle>>,
    /// Consecutive async-query failures; >= MAX_QUERY_FAILURES disables the tier.
    query_fail_count: AtomicU32,
    query_disabled: AtomicBool,
    /// Cooperative cancel flag of the in-flight `preview` call (the emit slot
    /// checks it between emits).
    preview_cancel_flag: Mutex<Option<Arc<AtomicBool>>>,
    /// Epoch cancel handle of the in-flight `preview` call.
    preview_epoch: Mutex<Option<CancelHandle>>,
}

/// Precomputes each command's icon `data:` URI from its bundled asset file,
/// once at load (never on the per-keystroke `commands()` path). A missing,
/// oversized, or unsafe icon is logged and skipped - the command still lists,
/// just with the generic glyph.
fn precompute_command_icons(
    manifest: &ExtensionManifest,
    wasm_path: &Path,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Some(dir) = wasm_path.parent() else { return out };
    for c in &manifest.commands {
        let Some(icon) = &c.icon else { continue };
        let note = |msg: &str| {
            crate::extensions::logs::log(&manifest.name, crate::extensions::logs::LogLevel::Error, msg);
        };
        // Untrusted manifest value: bare filename only, no separators or `..`,
        // so the asset can never escape the extension's own directory.
        if icon.is_empty() || icon.contains('/') || icon.contains('\\') || icon.contains("..") {
            note(&format!("command \"{}\" icon \"{icon}\" must be a bare filename", c.name));
            continue;
        }
        let b64 = match std::fs::read_to_string(dir.join(icon)) {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                note(&format!("command \"{}\" icon \"{icon}\" unreadable: {e}", c.name));
                continue;
            }
        };
        if b64.len() > MAX_ICON_B64_BYTES {
            note(&format!("command \"{}\" icon exceeds the {MAX_ICON_B64_BYTES}-byte cap", c.name));
            continue;
        }
        // `.b64` bundled assets are base64-encoded PNG by convention.
        out.insert(c.name.clone(), format!("data:image/png;base64,{b64}"));
    }
    out
}

/// Builds a fresh instance from a slot's compiled plugin.
fn instantiate(compiled: &CompiledPlugin) -> Result<Plugin, String> {
    Plugin::new_from_compiled(compiled)
        .map_err(|e| format!("failed to instantiate extension: {e}"))
}

/// Current wall-clock in unix ms, for the failure breaker.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl WasmProvider {
    /// Loads a validated manifest + wasm file into a live instance.
    /// Compiles the module - call from a background thread, never under the
    /// registry write lock.
    pub fn load(
        mut manifest: ExtensionManifest,
        wasm_path: PathBuf,
        kv: Arc<ExtensionKv>,
        settings: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Self, String> {
        let limits = manifest.limits.clamped();

        // Manifest-suggested command chords get the same clamp as result-action
        // shortcuts, once at load, so the command catalog only ever carries
        // canonical chords.
        for c in &mut manifest.commands {
            if let Some(raw) = c.default_shortcut.take() {
                match crate::keybinds::canonical(&raw) {
                    Some(canon) => c.default_shortcut = Some(canon),
                    None => {
                        let mut shown = raw;
                        crate::util::truncate_char_boundary(&mut shown, 64);
                        crate::extensions::logs::log(
                            &manifest.name,
                            crate::extensions::logs::LogLevel::Error,
                            &format!(
                                "command \"{}\": default_shortcut \"{shown}\" is invalid or reserved - ignored",
                                c.name
                            ),
                        );
                    }
                }
            }
        }

        let mut wasm_manifest = WasmManifest::new([Wasm::file(&wasm_path)])
            .with_memory_max(MEMORY_MAX_PAGES);
        if !manifest.permissions.network.is_empty() {
            wasm_manifest =
                wasm_manifest.with_allowed_hosts(manifest.permissions.network.iter().cloned());
        }
        let ctx = ExtensionCtx::new(manifest.name.clone(), &manifest.permissions, kv, settings);
        let (functions, ctx_data) = hostfns::build(ctx);
        // One compile per slot the module actually needs (see [`Slot`] for
        // why sharing one engine across slots is unsound). All slots share
        // the same host-function context via the cloned `functions`.
        let compile = || {
            CompiledPlugin::new(
                PluginBuilder::new(wasm_manifest.clone())
                    .with_functions(functions.clone())
                    .with_wasi(false),
            )
            .map_err(|e| format!("failed to load extension: {e}"))
        };

        let interactive = Slot::new(compile()?);
        let first = instantiate(&interactive.compiled)?;
        let has_query = first.function_exists("query");
        let has_preview = first.function_exists("preview");
        let has_refresh = first.function_exists("refresh");
        *util::lock(&interactive.instance) = Some(first);

        let preview_instance = if has_preview {
            let slot = Slot::new(compile()?);
            *util::lock(&slot.instance) = Some(instantiate(&slot.compiled)?);
            Some(slot)
        } else {
            None
        };
        let query_instance = if has_query { Some(Slot::new(compile()?)) } else { None };
        let bg_instance = if has_refresh { Some(Slot::new(compile()?)) } else { None };

        let command_icons = precompute_command_icons(&manifest, &wasm_path);

        Ok(Self {
            reg_id: format!("ext:{}", manifest.name),
            name: manifest.name.clone(),
            kind: manifest.default_kind(),
            manifest,
            wasm_path,
            command_icons,
            limits,
            instance: interactive,
            preview_instance,
            bg_instance,
            query_instance,
            ctx_data,
            has_query,
            last_error: Mutex::new(None),
            breaker: FailureBreaker::new(),
            query_cancel: Mutex::new(None),
            query_fail_count: AtomicU32::new(0),
            query_disabled: AtomicBool::new(false),
            preview_cancel_flag: Mutex::new(None),
            preview_epoch: Mutex::new(None),
        })
    }

    /// Search wall-clock budget - the registry sizes its fan-out deadline off
    /// the largest budget among loaded extensions.
    pub fn search_budget_ms(&self) -> u64 {
        self.limits.search_timeout_ms
    }

    pub fn last_error(&self) -> Option<String> {
        util::lock(&self.last_error).clone()
    }

    pub fn is_benched(&self) -> bool {
        self.breaker.is_open(now_ms())
    }

    /// The `SearchResult.kind` this extension's results carry.
    pub fn result_kind(&self) -> &str {
        &self.kind
    }

    /// Interval of the optional `[background]` refresh schedule.
    pub fn background_interval_secs(&self) -> Option<u64> {
        self.manifest.background.as_ref().map(|b| b.interval_secs())
    }

    // (instantiate is a free function below - it only needs the slot's
    // compiled plugin, not the provider.)

    /// Runs one exported function with a JSON payload under a wall-clock
    /// budget. The instance is lazily rebuilt if the previous call failed.
    /// `policy` selects failure accounting: interactive calls count toward
    /// the auto-bench streak, background refresh surfaces errors without
    /// benching, quiet calls leave accounting entirely to the caller.
    /// `cancel_store` (when given) receives the call's cancel handle before
    /// execution so another thread can cancel it mid-flight.
    /// `slot` (when given) is the call's streaming emit slot. It must be
    /// installed/cleared here, inside the instance-locked critical section:
    /// overlapping calls serialize on the instance mutex, and an install
    /// outside it lets the finishing call's unconditional clear wipe the slot
    /// the queued call just set ("emit is only valid during query/preview").
    fn call_with_budget(
        &self,
        instance: &Slot,
        function: &str,
        input: String,
        budget: Duration,
        policy: FailurePolicy,
        cancel_store: Option<&Mutex<Option<CancelHandle>>>,
        slot: Option<EmitSlot>,
    ) -> Result<Option<String>, String> {
        let interactive = policy == FailurePolicy::Interactive;
        if interactive && self.breaker.is_open(now_ms()) {
            return Err("extension paused after repeated failures".to_string());
        }
        let mut guard = util::lock(&instance.instance);
        if guard.is_none() {
            match instantiate(&instance.compiled) {
                Ok(p) => *guard = Some(p),
                Err(e) => {
                    match policy {
                        FailurePolicy::Interactive => self.record_failure(&e),
                        FailurePolicy::Background => self.note_error(&e),
                        FailurePolicy::Quiet => {}
                    }
                    return Err(e);
                }
            }
        }
        let plugin = guard.as_mut().expect("instance populated above");
        if !plugin.function_exists(function) {
            return Ok(None);
        }

        // A call that waited on the instance lock may be stale by the time it
        // owns it (epoch cancellation only reaches calls already in flight).
        // Bail before running the guest at all - the caller swallows these.
        match &slot {
            Some(EmitSlot::Query(s)) if s.generation != s.current.load(Ordering::Relaxed) => {
                return Err(format!("{function}: cancelled before start"));
            }
            Some(EmitSlot::Preview(s)) if s.cancelled.load(Ordering::Relaxed) => {
                return Err(format!("{function}: cancelled before start"));
            }
            _ => {}
        }

        // Install the emit slot now that this call owns the instance.
        let slot_kind = slot.as_ref().map(|s| matches!(s, EmitSlot::Query(_)));
        if let Some(slot) = slot {
            self.with_ctx(|ctx| match slot {
                EmitSlot::Query(s) => ctx.query_emit = Some(s),
                EmitSlot::Preview(s) => ctx.preview_emit = Some(s),
            });
        }

        // Watchdog: cancel the in-flight call if it outlives its budget.
        // recv_timeout keeps the thread cheap - it exits the moment the call
        // finishes, and epoch interruption makes cancel() safe mid-execution.
        let cancel = plugin.cancel_handle();
        if let Some(store) = cancel_store {
            *util::lock(store) = Some(plugin.cancel_handle());
        }
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let watchdog = std::thread::spawn(move || {
            if done_rx.recv_timeout(budget).is_err() {
                let _ = cancel.cancel();
            }
        });
        let outcome = plugin.call::<&str, String>(function, &input);
        let _ = done_tx.send(());
        let _ = watchdog.join();
        if let Some(store) = cancel_store {
            *util::lock(store) = None;
        }
        // Clear the slot before the instance lock is released (all paths
        // below drop `guard`), so it can never wipe a successor's slot.
        if let Some(is_query) = slot_kind {
            self.with_ctx(|ctx| {
                if is_query {
                    ctx.query_emit = None;
                } else {
                    ctx.preview_emit = None;
                }
            });
        }

        match outcome {
            Ok(json) => {
                // The module ran cleanly - reset the interactive failure streak.
                if interactive {
                    self.breaker.on_success();
                }
                if json.len() > MAX_OUTPUT_BYTES {
                    // Author bug (too much data), not a runtime failure: the
                    // instance is healthy, so keep it and don't bench -
                    // surface the error in Settings and drop the response.
                    let e = format!(
                        "{function} returned {} bytes (cap {MAX_OUTPUT_BYTES}) - return less data per call",
                        json.len()
                    );
                    drop(guard);
                    self.note_error(&e);
                    return Err(e);
                }
                Ok(Some(json))
            }
            Err(e) => {
                let e = format!("{function}: {}", e.root_cause());
                // Post-trap instance state is unreliable - rebuild next call.
                *guard = None;
                drop(guard);
                match policy {
                    // Background refresh failures stay visible but never bench -
                    // a broken refresh must not take down working search.
                    FailurePolicy::Interactive => self.record_failure(&e),
                    FailurePolicy::Background => self.note_error(&e),
                    // Quiet: the caller distinguishes cancellation from real
                    // failure and does its own accounting.
                    FailurePolicy::Quiet => {}
                }
                Err(e)
            }
        }
    }

    /// Logs and surfaces an error in Settings without counting it toward the
    /// auto-bench failure streak.
    fn note_error(&self, error: &str) {
        crate::extensions::logs::log(
            &self.name,
            crate::extensions::logs::LogLevel::Error,
            error,
        );
        *util::lock(&self.last_error) = Some(error.to_string());
    }

    fn record_failure(&self, error: &str) {
        self.note_error(error);
        if let Some(cooldown_ms) = self.breaker.on_failure(now_ms(), MAX_CONSECUTIVE_FAILURES) {
            crate::extensions::logs::log(
                &self.name,
                crate::extensions::logs::LogLevel::Error,
                &format!(
                    "paused for {}s after repeated failures - retries automatically",
                    cooldown_ms / 1000
                ),
            );
        }
    }

    /// Maps one extension DTO into an internal result: namespaced id, fixed
    /// kind, host-private scoring, clamped fields, no exec. Root results rank
    /// on the user-configurable extension band; scoped (explicitly entered)
    /// commands keep a fixed intent-tier score.
    fn to_search_result(
        &self,
        mut dto: ExtensionResult,
        command: &str,
        intent: bool,
    ) -> SearchResult {
        let relevance = if dto.relevance.is_finite() { dto.relevance } else { 0.0 };
        let icon_data_uri = dto.icon.as_ref().and_then(|i| self.icon_data_uri(i));
        let relevance_bonus = relevance.clamp(0.0, 100.0) / 100.0 * EXTENSION_BAND;
        // Scoped results carry no root-band parts: inside a scope there is
        // nothing else to compete against, and the scope must keep working
        // when the extension is weight-0 hidden from root search.
        let (score, parts) = if intent {
            (SCORE_EXTENSION_TRIGGERED + relevance_bonus, None)
        } else {
            let mut p = ScoreParts::new(Category::Extension, MatchTier::Fuzzy, 0);
            p.ext_name = Some(self.name.clone());
            p.intra = relevance_bonus;
            (0.0, Some(p))
        };
        dto.badge = dto.badge.take().map(clamp_field);
        for a in &mut dto.actions {
            a.label = clamp_field(std::mem::take(&mut a.label));
            a.hint = a.hint.take().map(clamp_field);
            // Untrusted chord suggestion: parse + reserved-set + size clamp,
            // normalized to canonical form. Invalid → dropped with a note.
            if let Some(raw) = a.shortcut.take() {
                match crate::keybinds::canonical(&raw) {
                    Some(canon) => a.shortcut = Some(canon),
                    None => {
                        let mut shown = raw;
                        crate::util::truncate_char_boundary(&mut shown, 64);
                        self.note_error(&format!(
                            "action \"{}\": shortcut \"{shown}\" is invalid or reserved - dropped",
                            a.id
                        ));
                    }
                }
            }
        }
        // Feed the Settings keybinds catalog (post-clamp, so it only ever
        // holds host-validated chords).
        crate::extensions::seen_actions::record(&self.name, &dto.actions);
        SearchResult {
            id: format!("ext:{}:{}", self.name, dto.id),
            title: clamp_field(dto.title.clone()),
            subtitle: dto.subtitle.clone().map(clamp_field),
            kind: self.command_kind(command),
            score,
            icon_data_uri,
            // Round-tripped back to the extension on activate/preview.
            ext: Some(dto),
            ext_command: Some(command.to_string()),
            parts,
            ..Default::default()
        }
    }

    /// The result kind for one of this extension's commands (per-command
    /// override or the extension default).
    fn command_kind(&self, command: &str) -> String {
        self.command_spec(command)
            .and_then(|c| c.kind.clone())
            .unwrap_or_else(|| self.kind.clone())
    }

    fn command_spec(&self, command: &str) -> Option<&CommandSpec> {
        self.manifest.commands.iter().find(|c| c.name == command)
    }

    /// Validates a result icon and builds its `data:` URI, or returns None
    /// (with the error surfaced in Settings) - a bad icon never drops the
    /// result and never fails the search.
    fn icon_data_uri(&self, icon: &portunus_ext_sdk::ResultIcon) -> Option<String> {
        if !ALLOWED_IMAGE_MIME.contains(&icon.mime.as_str()) {
            self.note_error(&format!(
                "result icon mime \"{}\" not allowed (png/jpeg/gif/webp only)",
                icon.mime
            ));
            return None;
        }
        if icon.data_base64.len() > MAX_ICON_B64_BYTES {
            self.note_error(&format!("result icon exceeds the {MAX_ICON_B64_BYTES}-byte cap"));
            return None;
        }
        Some(format!("data:{};base64,{}", icon.mime, icon.data_base64))
    }

    /// Runs the extension's default (or named) action for a result and
    /// returns the declarative effects it requested, validated and
    /// normalized. The caller (command layer) executes them - this type
    /// stays Tauri-free.
    pub fn activate(
        &self,
        command: String,
        result: ExtensionResult,
        action: Option<String>,
        form_values: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<Vec<ActivateEffect>, String> {
        if let Some(values) = &form_values {
            if values.len() > MAX_FORM_VALUES {
                return Err(format!("form submit exceeds {MAX_FORM_VALUES} values"));
            }
            if values
                .values()
                .any(|v| v.as_str().is_some_and(|s| s.len() > MAX_FORM_VALUE_BYTES))
            {
                return Err(format!(
                    "form submit value exceeds {MAX_FORM_VALUE_BYTES} bytes"
                ));
            }
        }
        let input = ActivateInput { command, result, action, form_values };
        let json = serde_json::to_string(&input).map_err(|e| e.to_string())?;
        match self.call_with_budget(
            &self.instance,
            "activate",
            json,
            Duration::from_millis(self.limits.activate_timeout_ms),
            FailurePolicy::Interactive,
            None,
            None,
        )? {
            Some(raw) => {
                // An empty/legacy body means "done, no effects" - only a
                // present-but-malformed response is an author error.
                if raw.trim().is_empty() {
                    return Ok(Vec::new());
                }
                let out: ActivateOutput = serde_json::from_str(&raw)
                    .map_err(|e| format!("activate: invalid response: {e}"))?;
                Ok(self.normalize_effects(out.effects))
            }
            None => Err("extension has no actions".to_string()),
        }
    }

    /// Applies the activate-effect caps: at most [`MAX_EFFECTS`] effects, at
    /// most one `ShowForm` (first wins), form field/option caps, and the
    /// `paste` permission gate. Violations are dropped and surfaced in the
    /// extension's log/Settings - they never fail the activation.
    fn normalize_effects(&self, effects: Vec<ActivateEffect>) -> Vec<ActivateEffect> {
        if effects.len() > MAX_EFFECTS {
            self.note_error(&format!(
                "activate returned {} effects - only the first {MAX_EFFECTS} run",
                effects.len()
            ));
        }
        let mut form_seen = false;
        let mut out = Vec::new();
        for effect in effects.into_iter().take(MAX_EFFECTS) {
            match effect {
                ActivateEffect::ShowForm { mut title, mut fields, submit_action, submit_label } => {
                    if form_seen {
                        self.note_error("activate returned more than one show_form - extras dropped");
                        continue;
                    }
                    form_seen = true;
                    crate::util::truncate_char_boundary(&mut title, MAX_FORM_TITLE_BYTES);
                    if fields.len() > MAX_FORM_FIELDS {
                        self.note_error(&format!(
                            "show_form has more than {MAX_FORM_FIELDS} fields - extras dropped"
                        ));
                        fields.truncate(MAX_FORM_FIELDS);
                    }
                    for field in &mut fields {
                        crate::util::truncate_char_boundary(&mut field.key, MAX_FIELD_BYTES);
                        crate::util::truncate_char_boundary(&mut field.label, MAX_FIELD_BYTES);
                        if let Some(p) = &mut field.placeholder {
                            crate::util::truncate_char_boundary(p, MAX_FIELD_BYTES);
                        }
                        if field.options.len() > MAX_FORM_OPTIONS {
                            field.options.truncate(MAX_FORM_OPTIONS);
                        }
                    }
                    out.push(ActivateEffect::ShowForm { title, fields, submit_action, submit_label });
                }
                ActivateEffect::Paste { text } => {
                    if !self.manifest.permissions.paste {
                        self.note_error("paste effect requires `paste = true` in [permissions]");
                        continue;
                    }
                    out.push(ActivateEffect::Paste { text });
                }
                ActivateEffect::SpawnProcess { command, args } => {
                    // The command must be an exact member of the manifest's
                    // `spawn` allowlist - the sandbox-breaking permission the
                    // user consented to. This is the authoritative gate;
                    // `run_activate_effects` trusts what survives here.
                    if !self.manifest.permissions.spawn.iter().any(|c| c == &command) {
                        self.note_error(&format!(
                            "spawn_process effect for \"{command}\" denied - not in the `spawn` permission allowlist"
                        ));
                        continue;
                    }
                    out.push(ActivateEffect::SpawnProcess { command, args });
                }
                // Permission-free effects pass through untouched. Listed
                // explicitly (no `_ =>` catch-all) so adding a new effect that
                // needs a permission is a compile error here until it's gated -
                // the sandbox stays deny-by-default rather than deny-by-omission.
                effect @ (ActivateEffect::CopyText { .. }
                | ActivateEffect::OpenUrl { .. }
                | ActivateEffect::ShowToast { .. }
                | ActivateEffect::Hide {}
                | ActivateEffect::KeepOpen {}
                | ActivateEffect::RefreshResults {}
                | ActivateEffect::SetQuery { .. }) => out.push(effect),
            }
        }
        out
    }

    /// Runs the extension's optional `refresh` export on the dedicated
    /// background instance - never contends with interactive calls.
    pub fn refresh(&self, reason: &str) -> Result<(), String> {
        // No `refresh` export: scheduled but nothing to do.
        let Some(bg) = &self.bg_instance else { return Ok(()) };
        let input = serde_json::to_string(&RefreshInput { reason: reason.to_string() })
            .map_err(|e| e.to_string())?;
        self.call_with_budget(
            bg,
            "refresh",
            input,
            Duration::from_secs(REFRESH_TIMEOUT_SECS),
            FailurePolicy::Background,
            None,
            None,
        )
        // Missing export is a no-op: scheduled but nothing to do.
        .map(|_| ())
    }

    /// Fetches declarative preview content, or None if the extension doesn't
    /// export `preview`. `sink` receives streamed intermediate content (each
    /// emit replaces the previous render); the return value is the final
    /// content. A concurrent [`Self::cancel_preview`] aborts the call.
    pub fn preview(
        &self,
        command: String,
        result: ExtensionResult,
        sink: impl FnMut(PreviewContent) + Send + 'static,
    ) -> Result<Option<PreviewContent>, String> {
        let Some(preview_slot) = &self.preview_instance else { return Ok(None) };
        let input = PreviewInput { command, result };
        let json = serde_json::to_string(&input).map_err(|e| e.to_string())?;

        // Cooperative cancel flag: registered before the call so a
        // cancel_preview that fires while this call is still queued on the
        // instance lock already reaches it (newest registration wins).
        let cancelled = Arc::new(AtomicBool::new(false));
        *util::lock(&self.preview_cancel_flag) = Some(cancelled.clone());

        // Quiet policy: a cancelled preview (user moved the selection) is not
        // a failure and must never feed the bench streak - account manually.
        let outcome = self.call_with_budget(
            preview_slot,
            "preview",
            json,
            Duration::from_millis(self.limits.preview_timeout_ms),
            FailurePolicy::Quiet,
            Some(&self.preview_epoch),
            Some(EmitSlot::Preview(PreviewEmitSlot {
                cancelled: cancelled.clone(),
                sink: Box::new(sink),
            })),
        );
        // Deregister only our own flag - a successor may have replaced it.
        {
            let mut flag = util::lock(&self.preview_cancel_flag);
            if flag.as_ref().is_some_and(|f| Arc::ptr_eq(f, &cancelled)) {
                *flag = None;
            }
        }

        let raw = match outcome {
            Ok(Some(raw)) => raw,
            Ok(None) => return Ok(None),
            Err(e) => {
                if cancelled.load(Ordering::Relaxed) {
                    return Err("preview cancelled".to_string());
                }
                self.record_failure(&e);
                return Err(e);
            }
        };
        if cancelled.load(Ordering::Relaxed) {
            return Err("preview cancelled".to_string());
        }
        self.breaker.on_success();
        let content: PreviewContent =
            serde_json::from_str(&raw).map_err(|e| format!("preview: invalid response: {e}"))?;
        validate_preview_content(&content)?;
        Ok(Some(content))
    }

    /// Cancels the in-flight `preview` call, if any: flips the cooperative
    /// flag (emit slot tells the guest to stop) and fires the epoch cancel.
    pub fn cancel_preview(&self) {
        if let Some(flag) = util::lock(&self.preview_cancel_flag).as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
        if let Some(handle) = util::lock(&self.preview_epoch).as_ref() {
            let _ = handle.cancel();
        }
    }

    /// Whether the module exports the async `query` function.
    pub fn has_query(&self) -> bool {
        self.has_query
    }

    /// Async tier disabled after repeated query failures this session.
    pub fn query_disabled(&self) -> bool {
        self.query_disabled.load(Ordering::Relaxed)
    }

    /// Cancels the in-flight async `query` call, if any (epoch interruption;
    /// the emit slot's generation gate handles the cooperative side).
    pub fn cancel_query(&self) {
        if let Some(handle) = util::lock(&self.query_cancel).as_ref() {
            let _ = handle.cancel();
        }
    }

    /// Runs a scoped mutation of the shared host-function context.
    fn with_ctx(&self, f: impl FnOnce(&mut ExtensionCtx)) {
        if let Ok(ctx) = self.ctx_data.get() {
            let mut ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut ctx);
        }
    }

    /// Runs the async `query` export on its dedicated instance. Blocking -
    /// call from a worker thread, never the keystroke path.
    ///
    /// `generation`/`current` gate streaming emits: the moment `current`
    /// moves past `generation` (new keystroke), emits are dropped and the
    /// guest is told to stop. `sink` receives already-mapped partial batches;
    /// the returned Vec is the mapped final batch.
    ///
    /// Failure policy: a cancelled call is swallowed (Ok(empty)); genuine
    /// failures surface in Settings and count toward the query tier's own
    /// disable streak - never the interactive bench.
    pub fn run_query(
        self: &Arc<Self>,
        gc: GatedCommand,
        intent: bool,
        generation: u64,
        current: Arc<AtomicU64>,
        mut sink: impl FnMut(Vec<SearchResult>) + Send + 'static,
    ) -> Result<Vec<SearchResult>, String> {
        if self.query_disabled() {
            return Ok(Vec::new());
        }
        let Some(query_slot) = &self.query_instance else { return Ok(Vec::new()) };
        let command = gc.command;
        let input = serde_json::to_string(&QueryInput {
            command: command.clone(),
            query: gc.gated.query,
            raw_query: gc.gated.raw_query,
        })
        .map_err(|e| e.to_string())?;

        // The emit slot maps DTOs host-side before they reach the manager, so
        // streamed results pass the exact same clamps/validation as sync ones.
        let mapper = self.clone();
        let map_command = command.clone();
        let slot = QueryEmitSlot {
            generation,
            current: current.clone(),
            sink: Box::new(move |dtos: Vec<ExtensionResult>| {
                let mapped = dtos
                    .into_iter()
                    .map(|dto| mapper.to_search_result(dto, &map_command, intent))
                    .collect();
                sink(mapped);
            }),
            emitted: 0,
        };

        let outcome = self.call_with_budget(
            query_slot,
            "query",
            input,
            Duration::from_millis(self.limits.query_timeout_ms),
            FailurePolicy::Quiet,
            Some(&self.query_cancel),
            Some(EmitSlot::Query(slot)),
        );

        match outcome {
            Ok(Some(raw)) => {
                self.query_fail_count.store(0, Ordering::Relaxed);
                let out: QueryOutput = serde_json::from_str(&raw)
                    .map_err(|e| format!("query: invalid response: {e}"))?;
                Ok(out
                    .results
                    .into_iter()
                    .take(MAX_RESULTS)
                    .map(|dto| self.to_search_result(dto, &command, intent))
                    .collect())
            }
            Ok(None) => Ok(Vec::new()),
            Err(e) => {
                // Cancelled by a newer query: not a failure, swallow entirely.
                if generation != current.load(Ordering::Relaxed) {
                    return Ok(Vec::new());
                }
                self.note_error(&e);
                let fails = self.query_fail_count.fetch_add(1, Ordering::Relaxed) + 1;
                if fails >= MAX_QUERY_FAILURES {
                    self.query_disabled.store(true, Ordering::Relaxed);
                    crate::extensions::logs::log(
                        &self.name,
                        crate::extensions::logs::LogLevel::Error,
                        &format!(
                            "async query disabled for this session after {fails} consecutive failures (sync search keeps running)"
                        ),
                    );
                }
                Err(e)
            }
        }
    }
}

/// Validates declarative preview content against host caps. Shared by the
/// one-shot `preview` return path and the streaming `emit_preview` host fn so
/// streamed content can't bypass the caps.
pub fn validate_preview_content(content: &PreviewContent) -> Result<(), String> {
    if let PreviewContent::Image { mime, data_base64 } = content {
        if !ALLOWED_IMAGE_MIME.contains(&mime.as_str()) {
            return Err(format!(
                "preview: image mime \"{mime}\" not allowed (png/jpeg/gif/webp only)"
            ));
        }
        if data_base64.len() > MAX_IMAGE_B64_BYTES {
            return Err("preview: image exceeds the 1 MB cap".to_string());
        }
    }
    if let PreviewContent::Html { content } = content {
        if content.len() > 128 * 1024 {
            return Err("preview: html exceeds the 128 KB cap".to_string());
        }
    }
    Ok(())
}

fn clamp_field(mut s: String) -> String {
    crate::util::truncate_char_boundary(&mut s, MAX_FIELD_BYTES);
    s
}

impl WasmProvider {
    #[allow(dead_code)]
    pub fn id(&self) -> &str {
        &self.reg_id
    }

    /// The command descriptors this extension contributes to the catalog -
    /// one per `[[commands]]` manifest entry.
    pub fn commands(&self) -> Vec<crate::providers::CommandDescriptor> {
        use crate::providers::command::{CommandDescriptor, CommandRoute, CommandSource, ModeKind};
        self.manifest
            .commands
            .iter()
            .map(|c| CommandDescriptor {
                id: format!("ext:{}:cmd:{}", self.name, c.name),
                title: c.title.clone(),
                chip: c.chip_label(),
                subtitle: Some(if c.description.is_empty() {
                    self.name.clone()
                } else {
                    c.description.clone()
                }),
                source: CommandSource::Extension { name: self.name.clone() },
                mode_kind: match c.mode.as_str() {
                    "action" => ModeKind::Action,
                    // "inline" (legacy) and "scope" both open an enterable scope.
                    _ => ModeKind::Scope,
                },
                keywords: c.keywords.clone(),
                placeholder: (!c.placeholder.is_empty()).then(|| c.placeholder.clone()),
                min_query_len: c.min_len(),
                result_kind: c.kind.clone().unwrap_or_else(|| self.kind.clone()),
                glyph: None,
                icon_data_uri: self.command_icons.get(&c.name).cloned(),
                default_shortcut: c.default_shortcut.clone(),
                opens_form: c.opens_form,
                uncapped: false,
                route: CommandRoute::Extension {
                    name: self.name.clone(),
                    command: c.name.clone(),
                },
            })
            .collect()
    }

    /// Resolves which command (if any) a root-search keystroke invokes.
    /// None = don't run (the registry spawns no thread). Pure and cheap - it
    /// runs for every loaded extension on every keystroke.
    pub fn gate(&self, raw_query: &str) -> Option<GatedCommand> {
        trigger::gate(&self.manifest.commands, raw_query)
    }

    /// Gate for an entered command mode: the whole query is the command's
    /// input, min_query_len applies, empty = browse state.
    pub fn gate_scoped(&self, command: &str, query: &str) -> Option<GatedCommand> {
        trigger::gate_scoped(self.command_spec(command)?, query)
    }

    /// Runs the extension's `search` export for a gated command. `intent` is
    /// true when the user explicitly invoked the command (typed prefix or an
    /// entered mode) - always-mode invocations band lower.
    pub fn search_gated(&self, gc: GatedCommand, intent: bool) -> Vec<SearchResult> {
        let command = gc.command;
        let input = match serde_json::to_string(&SearchInput {
            command: command.clone(),
            query: gc.gated.query,
            raw_query: gc.gated.raw_query,
        }) {
            Ok(j) => j,
            Err(_) => return Vec::new(),
        };
        let raw = match self.call_with_budget(
            &self.instance,
            "search",
            input,
            Duration::from_millis(self.limits.search_timeout_ms),
            FailurePolicy::Interactive,
            None,
            None,
        ) {
            Ok(Some(raw)) => raw,
            // Missing search export or a failed call both mean "no results";
            // failures are already recorded for the Settings UI.
            Ok(None) | Err(_) => return Vec::new(),
        };
        let output: SearchOutput = match serde_json::from_str(&raw) {
            Ok(o) => o,
            Err(e) => {
                self.record_failure(&format!("search: invalid response: {e}"));
                return Vec::new();
            }
        };
        output
            .results
            .into_iter()
            .take(MAX_RESULTS)
            .map(|dto| self.to_search_result(dto, &command, intent))
            .collect()
    }
}

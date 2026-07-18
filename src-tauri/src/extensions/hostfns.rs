//! Host functions exposed to extensions.
//!
//! Every function checks the extension's manifest-declared permissions at call
//! time - an extension always gets the symbols, but calls without the matching
//! permission grant return errors. Bodies must never panic: a panic unwinding
//! across the wasmtime FFI boundary is undefined behavior, so everything here
//! is Result-based and locks recover from poisoning.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use extism::convert::Json;
use extism::{host_fn, Function, UserData, PTR};
use portunus_ext_sdk::{EmitAck, EmitBatch, ExtensionResult, PreviewContent};

use super::kv::ExtensionKv;
use super::manifest::Permissions;

/// Raw JSON cap per `emit_results`/`emit_preview` payload.
const MAX_EMIT_BYTES: usize = 512 * 1024;
/// Results one `query` call may emit in total, across all batches (mirrors
/// the sync-tier per-call cap).
const MAX_QUERY_RESULTS: usize = 200;

/// Streaming sink installed for the duration of one `query` call. Emits are
/// gated by the query generation: a stale generation means the user typed a
/// new query, and the extension is told to stop via `EmitAck::cancelled`.
pub struct QueryEmitSlot {
    /// Generation this call belongs to.
    pub generation: u64,
    /// Live counter owned by the query manager; bumped on every new search.
    pub current: Arc<AtomicU64>,
    /// Receives validated, still-live batches.
    pub sink: Box<dyn FnMut(Vec<ExtensionResult>) + Send>,
    /// Running result count, for the per-query cap.
    pub emitted: usize,
}

/// Streaming sink installed for the duration of one `preview` call. Each emit
/// REPLACES the rendered preview; cancellation flips the shared flag.
pub struct PreviewEmitSlot {
    pub cancelled: Arc<AtomicBool>,
    pub sink: Box<dyn FnMut(PreviewContent) + Send>,
}

/// Per-extension context shared with every host function via `UserData`.
pub struct ExtensionCtx {
    pub name: String,
    pub kv_enabled: bool,
    pub clipboard_enabled: bool,
    pub open_url_enabled: bool,
    pub bus_enabled: bool,
    pub kv: Arc<ExtensionKv>,
    /// Immutable snapshot of the extension's resolved settings (schema
    /// defaults overlaid with user values). A settings change reloads the
    /// extension, so the snapshot can never go stale mid-instance.
    pub settings: std::collections::HashMap<String, serde_json::Value>,
    /// Set by the host around a `query` call; None otherwise. One UserData is
    /// shared by all instances of an extension, but query and preview run on
    /// dedicated instances, so the two slots never fight over one field.
    pub query_emit: Option<QueryEmitSlot>,
    /// Set by the host around a `preview` call; None otherwise.
    pub preview_emit: Option<PreviewEmitSlot>,
}

impl ExtensionCtx {
    pub fn new(
        name: String,
        permissions: &Permissions,
        kv: Arc<ExtensionKv>,
        settings: std::collections::HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            name,
            kv_enabled: permissions.kv,
            clipboard_enabled: permissions.clipboard,
            open_url_enabled: permissions.open_url,
            bus_enabled: permissions.bus,
            kv,
            settings,
            query_emit: None,
            preview_emit: None,
        }
    }
}

host_fn!(kv_get(ctx: ExtensionCtx; key: String) -> Json<Option<String>> {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    if !ctx.kv_enabled {
        return Err(extism::Error::msg("kv permission not granted"));
    }
    Ok(Json(ctx.kv.get(&ctx.name, &key)))
});

host_fn!(kv_set(ctx: ExtensionCtx; key: String, value: String) {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    if !ctx.kv_enabled {
        return Err(extism::Error::msg("kv permission not granted"));
    }
    ctx.kv.set(&ctx.name, &key, &value).map_err(extism::Error::msg)
});

host_fn!(kv_list(ctx: ExtensionCtx; prefix: String) -> Json<Vec<String>> {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    if !ctx.kv_enabled {
        return Err(extism::Error::msg("kv permission not granted"));
    }
    Ok(Json(ctx.kv.list(&ctx.name, &prefix)))
});

host_fn!(kv_delete(ctx: ExtensionCtx; key: String) {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    if !ctx.kv_enabled {
        return Err(extism::Error::msg("kv permission not granted"));
    }
    ctx.kv.delete(&ctx.name, &key);
    Ok(())
});

// Wall clock for cache staleness - std::time doesn't work on
// wasm32-unknown-unknown. Not sensitive, so no permission gate.
host_fn!(now_ms(_ctx: ExtensionCtx;) -> u64 {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0))
});

host_fn!(open_url(ctx: ExtensionCtx; url: String) {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    if !ctx.open_url_enabled {
        return Err(extism::Error::msg("open_url permission not granted"));
    }
    open_http_url(&url).map_err(extism::Error::msg)
});

/// Opens a http(s) URL in the default browser. Shared by the `open_url` host
/// fn and the `OpenUrl` activate effect - both enforce the same caps.
pub fn open_http_url(url: &str) -> Result<(), String> {
    // Scheme allowlist: anything else (file:, javascript:, custom handlers)
    // would hand the extension an arbitrary-handler launch primitive.
    if url.len() > 2048 {
        return Err("url exceeds 2048 bytes".to_string());
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http(s) urls may be opened".to_string());
    }
    crate::util::spawn_detached("xdg-open", &[url])
        .map(|_| ())
        .map_err(|e| format!("xdg-open: {e}"))
}

/// Launches an OS command detached, argv-only (never a shell). Reached only via
/// the `SpawnProcess` activate effect, whose command the provider has already
/// checked against the manifest's `spawn` allowlist - so this is a plain
/// fire-and-forget spawn (same detached pattern as [`open_http_url`]), not a
/// permission boundary. **Bypasses the wasm sandbox by design.**
pub fn spawn_process(command: &str, args: &[String]) -> Result<(), String> {
    // Guard the argv the same way the URL path guards its input: bounded and
    // free of embedded control bytes, before it reaches the OS.
    if command.is_empty() {
        return Err("spawn: empty command".to_string());
    }
    if args.len() > 64 {
        return Err("spawn: too many arguments (max 64)".to_string());
    }
    for a in args {
        if a.len() > 4096 || a.contains('\0') {
            return Err("spawn: argument too long or contains NUL".to_string());
        }
    }
    // Bare command names resolve via the inherited $PATH (see the `spawn` note in
    // EXTENSIONS.md); the allowlist constrains the name, not which file wins the
    // PATH lookup. Detached, argv-only, fire-and-forget.
    crate::util::spawn_detached(command, args)
        .map(|_| ())
        .map_err(|e| format!("{command}: {e}"))
}

// ── Message bus (see extensions::bus) ─────────────────────────────────────────

// Whether a companion process is attached. Permission-gated like the other
// bus calls, but reports `false` instead of erroring so guests can branch to
// a fallback path without error handling.
host_fn!(bus_status(ctx: ExtensionCtx;) -> Json<bool> {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    Ok(Json(ctx.bus_enabled && crate::extensions::bus::BUS.attached(&ctx.name)))
});

// Request/response with the companion. `payload` is a BusRequest envelope
// (timeout + arbitrary JSON). The blocking wait runs OUTSIDE the ctx lock -
// one UserData is shared by every instance slot of the extension, and a
// 30 s wait under the lock would freeze the other slots' host calls.
host_fn!(bus_request(ctx: ExtensionCtx; payload: String) -> String {
    if payload.len() > super::bus::MAX_LINE_BYTES {
        return Err(extism::Error::msg(format!(
            "bus_request payload exceeds {} bytes", super::bus::MAX_LINE_BYTES
        )));
    }
    let req: portunus_ext_sdk::BusRequest = serde_json::from_str(&payload)
        .map_err(|e| extism::Error::msg(format!("bus_request: invalid envelope: {e}")))?;
    // Snapshot what the wait needs, then release the lock.
    let (name, cancel_gate) = {
        let ctx = ctx.get()?;
        let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
        if !ctx.bus_enabled {
            return Err(extism::Error::msg("bus permission not granted"));
        }
        // During `query`, cancellation is the emit slot's generation gate -
        // reuse it so a stale query stops waiting on a slow companion.
        let gate = ctx
            .query_emit
            .as_ref()
            .map(|s| (s.generation, s.current.clone()));
        (ctx.name.clone(), gate)
    };
    let cancelled = move || {
        cancel_gate
            .as_ref()
            .is_some_and(|(generation, current)| *generation != current.load(Ordering::Relaxed))
    };
    let reply = crate::extensions::bus::BUS
        .request(&name, req.payload, req.timeout_ms, cancelled)
        .map_err(extism::Error::msg)?;
    Ok(serde_json::to_string(&reply).map_err(|e| extism::Error::msg(e.to_string()))?)
});

// Fire-and-forget message to the companion.
host_fn!(bus_notify(ctx: ExtensionCtx; payload: String) {
    if payload.len() > super::bus::MAX_LINE_BYTES {
        return Err(extism::Error::msg(format!(
            "bus_notify payload exceeds {} bytes", super::bus::MAX_LINE_BYTES
        )));
    }
    let name = {
        let ctx = ctx.get()?;
        let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
        if !ctx.bus_enabled {
            return Err(extism::Error::msg("bus permission not granted"));
        }
        ctx.name.clone()
    };
    let value: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|e| extism::Error::msg(format!("bus_notify: invalid json: {e}")))?;
    crate::extensions::bus::BUS.notify(&name, value).map_err(extism::Error::msg)
});

host_fn!(log_message(ctx: ExtensionCtx; message: String) {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    let mut msg = message;
    crate::util::truncate_char_boundary(&mut msg, 4096);
    super::logs::log(&ctx.name, super::logs::LogLevel::Info, &msg);
    Ok(())
});

// Settings are extension-owned data the user typed into the host UI - not
// sensitive host state - so no permission gate.
host_fn!(settings_get(ctx: ExtensionCtx; key: String) -> Json<Option<serde_json::Value>> {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    Ok(Json(ctx.settings.get(&key).cloned()))
});

// Streaming: pushes one partial result batch from inside `query`. Takes the
// raw string (not Json<..>) so the payload size cap applies before parsing.
host_fn!(emit_results(ctx: ExtensionCtx; payload: String) -> Json<EmitAck> {
    let ctx = ctx.get()?;
    let mut ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    let name = ctx.name.clone();
    let Some(slot) = ctx.query_emit.as_mut() else {
        return Err(extism::Error::msg("emit_results is only valid during query"));
    };
    // Stale generation = the user typed a new query. Tell the guest to stop
    // and forward nothing - this kills post-cancel emits at the source.
    if slot.generation != slot.current.load(Ordering::Relaxed) {
        return Ok(Json(EmitAck { cancelled: true }));
    }
    if payload.len() > MAX_EMIT_BYTES {
        return Err(extism::Error::msg(format!(
            "emit_results payload is {} bytes (cap {MAX_EMIT_BYTES}) - emit smaller batches",
            payload.len()
        )));
    }
    let batch: EmitBatch = serde_json::from_str(&payload)
        .map_err(|e| extism::Error::msg(format!("emit_results: invalid batch: {e}")))?;
    let mut results = batch.results;
    let room = MAX_QUERY_RESULTS.saturating_sub(slot.emitted);
    if results.len() > room {
        results.truncate(room);
        super::logs::log(
            &name,
            super::logs::LogLevel::Error,
            &format!("query emitted more than {MAX_QUERY_RESULTS} results - excess dropped"),
        );
    }
    slot.emitted += results.len();
    if !results.is_empty() {
        (slot.sink)(results);
    }
    Ok(Json(EmitAck { cancelled: false }))
});

// Streaming: replaces the rendered preview from inside `preview`.
host_fn!(emit_preview(ctx: ExtensionCtx; payload: String) -> Json<EmitAck> {
    let ctx = ctx.get()?;
    let mut ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    let Some(slot) = ctx.preview_emit.as_mut() else {
        return Err(extism::Error::msg("emit_preview is only valid during preview"));
    };
    if slot.cancelled.load(Ordering::Relaxed) {
        return Ok(Json(EmitAck { cancelled: true }));
    }
    if payload.len() > MAX_EMIT_BYTES {
        return Err(extism::Error::msg(format!(
            "emit_preview payload is {} bytes (cap {MAX_EMIT_BYTES})",
            payload.len()
        )));
    }
    let content: PreviewContent = serde_json::from_str(&payload)
        .map_err(|e| extism::Error::msg(format!("emit_preview: invalid content: {e}")))?;
    crate::providers::wasm::validate_preview_content(&content)
        .map_err(extism::Error::msg)?;
    (slot.sink)(content);
    Ok(Json(EmitAck { cancelled: false }))
});

host_fn!(clipboard_write(ctx: ExtensionCtx; text: String) {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    if !ctx.clipboard_enabled {
        return Err(extism::Error::msg("clipboard permission not granted"));
    }
    write_clipboard_text(&text).map_err(extism::Error::msg)
});

/// Same wl-copy path the clipboard provider uses. Shared by the
/// `clipboard_write` host fn and the `CopyText` activate effect.
pub fn write_clipboard_text(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("wl-copy: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("wl-copy: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("wl-copy: {e}"))?;
    if !status.success() {
        return Err("wl-copy exited with an error".to_string());
    }
    Ok(())
}

/// Delay between hiding the launcher and injecting the paste chord - focus
/// must return to the previously focused surface first. Same heuristic as
/// the clipboard provider's smart paste; raise if pastes land in the void
/// on slower compositors.
const PASTE_FOCUS_DELAY_MS: u64 = 150;

/// Backs the `Paste` activate effect: clipboard write now, paste chord after
/// the launcher has hidden and focus returned. wtype speaks
/// zwp_virtual_keyboard_v1 (wlroots compositors); where it fails the text is
/// already on the clipboard, so degrade to a "press Ctrl+V" notification -
/// never a silent no-op.
pub fn paste_text(text: &str) -> Result<(), String> {
    write_clipboard_text(text)?;
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(PASTE_FOCUS_DELAY_MS));
        // Same chord the clipboard provider's smart paste uses.
        let ok = std::process::Command::new("wtype")
            .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = std::process::Command::new("notify-send")
                .arg("--app-name=Portunus")
                .arg("--expire-time=3000")
                .arg("Portunus")
                .arg("Copied to clipboard — press Ctrl+V to paste")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    });
    Ok(())
}

/// Builds the host-function imports for one extension. Returns the functions
/// plus the shared `UserData` handle - the provider keeps it to install the
/// streaming emit slots around `query`/`preview` calls.
pub fn build(ctx: ExtensionCtx) -> (Vec<Function>, UserData<ExtensionCtx>) {
    let data = UserData::new(ctx);
    let functions = vec![
        Function::new("kv_get", [PTR], [PTR], data.clone(), kv_get),
        Function::new("kv_set", [PTR, PTR], [], data.clone(), kv_set),
        Function::new("kv_list", [PTR], [PTR], data.clone(), kv_list),
        Function::new("kv_delete", [PTR], [], data.clone(), kv_delete),
        Function::new("now_ms", [], [PTR], data.clone(), now_ms),
        Function::new("open_url", [PTR], [], data.clone(), open_url),
        Function::new("log_message", [PTR], [], data.clone(), log_message),
        Function::new("settings_get", [PTR], [PTR], data.clone(), settings_get),
        Function::new("clipboard_write", [PTR], [], data.clone(), clipboard_write),
        Function::new("emit_results", [PTR], [PTR], data.clone(), emit_results),
        Function::new("emit_preview", [PTR], [PTR], data.clone(), emit_preview),
        Function::new("bus_status", [], [PTR], data.clone(), bus_status),
        Function::new("bus_request", [PTR], [PTR], data.clone(), bus_request),
        Function::new("bus_notify", [PTR], [], data.clone(), bus_notify),
    ];
    (functions, data)
}

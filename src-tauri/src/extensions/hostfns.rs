//! Host functions exposed to extensions.
//!
//! Every function checks the extension's manifest-declared permissions at call
//! time - an extension always gets the symbols, but calls without the matching
//! permission grant return errors. Bodies must never panic: a panic unwinding
//! across the wasmtime FFI boundary is undefined behavior, so everything here
//! is Result-based and locks recover from poisoning.

use std::sync::Arc;

use extism::convert::Json;
use extism::{host_fn, Function, UserData, PTR};

use super::kv::ExtensionKv;
use super::manifest::Permissions;

/// Per-extension context shared with every host function via `UserData`.
pub struct ExtensionCtx {
    pub name: String,
    pub kv_enabled: bool,
    pub clipboard_enabled: bool,
    pub open_url_enabled: bool,
    pub kv: Arc<ExtensionKv>,
    /// Immutable snapshot of the extension's resolved settings (schema
    /// defaults overlaid with user values). A settings change reloads the
    /// extension, so the snapshot can never go stale mid-instance.
    pub settings: std::collections::HashMap<String, serde_json::Value>,
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
            kv,
            settings,
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
    // Same detached-spawn pattern as launch_app.
    use std::os::unix::process::CommandExt;
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("xdg-open: {e}"))
}

host_fn!(log_message(ctx: ExtensionCtx; message: String) {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    let mut msg = message;
    if msg.len() > 4096 {
        let mut cut = 4096;
        while !msg.is_char_boundary(cut) {
            cut -= 1;
        }
        msg.truncate(cut);
    }
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

/// Builds the host-function imports for one extension instance.
pub fn build(ctx: ExtensionCtx) -> Vec<Function> {
    let data = UserData::new(ctx);
    vec![
        Function::new("kv_get", [PTR], [PTR], data.clone(), kv_get),
        Function::new("kv_set", [PTR, PTR], [], data.clone(), kv_set),
        Function::new("kv_list", [PTR], [PTR], data.clone(), kv_list),
        Function::new("kv_delete", [PTR], [], data.clone(), kv_delete),
        Function::new("now_ms", [], [PTR], data.clone(), now_ms),
        Function::new("open_url", [PTR], [], data.clone(), open_url),
        Function::new("log_message", [PTR], [], data.clone(), log_message),
        Function::new("settings_get", [PTR], [PTR], data.clone(), settings_get),
        Function::new("clipboard_write", [PTR], [], data, clipboard_write),
    ]
}

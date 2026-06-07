//! Host functions exposed to extensions.
//!
//! Every function checks the extension's manifest-declared permissions at call
//! time — an extension always gets the symbols, but calls without the matching
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
}

impl ExtensionCtx {
    pub fn new(name: String, permissions: &Permissions, kv: Arc<ExtensionKv>) -> Self {
        Self {
            name,
            kv_enabled: permissions.kv,
            clipboard_enabled: permissions.clipboard,
            open_url_enabled: permissions.open_url,
            kv,
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

// Wall clock for cache staleness — std::time doesn't work on
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
    // Scheme allowlist: anything else (file:, javascript:, custom handlers)
    // would hand the extension an arbitrary-handler launch primitive.
    if url.len() > 2048 {
        return Err(extism::Error::msg("url exceeds 2048 bytes"));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(extism::Error::msg("only http(s) urls may be opened"));
    }
    // Same detached-spawn pattern as launch_app.
    use std::os::unix::process::CommandExt;
    std::process::Command::new("xdg-open")
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| extism::Error::msg(format!("xdg-open: {e}")))
});

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
    eprintln!("[ext:{}] {msg}", ctx.name);
    Ok(())
});

host_fn!(clipboard_write(ctx: ExtensionCtx; text: String) {
    let ctx = ctx.get()?;
    let ctx = ctx.lock().unwrap_or_else(|e| e.into_inner());
    if !ctx.clipboard_enabled {
        return Err(extism::Error::msg("clipboard permission not granted"));
    }
    write_clipboard(&text)
});

/// Same wl-copy path the clipboard provider uses.
fn write_clipboard(text: &str) -> Result<(), extism::Error> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| extism::Error::msg(format!("wl-copy: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| extism::Error::msg(format!("wl-copy: {e}")))?;
    }
    let status = child
        .wait()
        .map_err(|e| extism::Error::msg(format!("wl-copy: {e}")))?;
    if !status.success() {
        return Err(extism::Error::msg("wl-copy exited with an error"));
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
        Function::new("clipboard_write", [PTR], [], data, clipboard_write),
    ]
}

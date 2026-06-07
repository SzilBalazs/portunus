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
    pub kv: Arc<ExtensionKv>,
}

impl ExtensionCtx {
    pub fn new(name: String, permissions: &Permissions, kv: Arc<ExtensionKv>) -> Self {
        Self {
            name,
            kv_enabled: permissions.kv,
            clipboard_enabled: permissions.clipboard,
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
        Function::new("clipboard_write", [PTR], [], data, clipboard_write),
    ]
}

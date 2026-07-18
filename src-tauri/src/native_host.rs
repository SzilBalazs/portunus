//! `portunus native-host` - browser native-messaging shim for the extension
//! message bus.
//!
//! A browser extension declaring a native-messaging host makes the browser
//! spawn this process and speak length-prefixed JSON over stdio (u32 LE frame
//! length + payload). This subcommand is a pure relay: stdio frames in ↔
//! newline-delimited JSON on the portunus socket's `ext-attach:<name>`
//! channel out (see `extensions::bus`). No logic, no parsing of payloads.
//!
//! Browser manifests have no `args` field - the browser invokes the manifest's
//! `path` with `[manifest-path, browser-ext-id]` - so `install` writes a tiny
//! wrapper script that execs `portunus native-host <name>` and points the
//! manifest at it.

use std::io::{BufRead, Read, Write};
use std::os::unix::net::UnixStream;

use crate::extensions::bus;

/// Entry point for `portunus native-host <args>`. Returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("install") => install(&args[1..]),
        Some(name) if bus::valid_name(name) => pump(name),
        _ => {
            eprintln!(
                "usage: portunus native-host <extension-name>\n       portunus native-host install <extension-name> --ff-ext-id <id@domain>"
            );
            2
        }
    }
}

/// Relays stdio native-messaging frames ↔ the socket's ext-attach channel
/// until either side closes.
fn pump(name: &str) -> i32 {
    let mut socket = match UnixStream::connect(crate::ipc::socket_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("portunus native-host: cannot reach a running portunus: {e}");
            return 1;
        }
    };
    if socket.write_all(format!("ext-attach:{name}\n").as_bytes()).is_err() {
        eprintln!("portunus native-host: attach failed");
        return 1;
    }

    // Browser → portunus. Runs on this (main) thread.
    let to_socket = {
        let mut socket = match socket.try_clone() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("portunus native-host: {e}");
                return 1;
            }
        };
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            loop {
                let Some(frame) = read_frame(&mut stdin) else { break };
                if socket.write_all(&frame).is_err() || socket.write_all(b"\n").is_err() {
                    break;
                }
            }
            // Stdin closed (browser gone) - close the socket so the other
            // pump direction unblocks too.
            let _ = socket.shutdown(std::net::Shutdown::Both);
        })
    };

    // Portunus → browser.
    let mut reader = std::io::BufReader::new(&mut socket);
    let mut stdout = std::io::stdout().lock();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let mut limited = Read::take(&mut reader, (bus::MAX_LINE_BYTES + 1) as u64);
        match limited.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) if buf.len() > bus::MAX_LINE_BYTES => break,
            Ok(_) => {}
            Err(_) => break,
        }
        while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            buf.pop();
        }
        if buf.is_empty() {
            continue;
        }
        if write_frame(&mut stdout, &buf).is_err() {
            break;
        }
    }
    // Socket closed - unblock the stdin thread by exiting; the browser closes
    // stdin when it reaps us, and the thread also exits on its own EOF.
    drop(stdout);
    let _ = socket.shutdown(std::net::Shutdown::Both);
    let _ = to_socket.join();
    0
}

/// Reads one native-messaging frame: u32 LE length + payload. None on EOF,
/// oversize, or a short read. The payload must not contain a raw newline
/// (it would corrupt the NDJSON channel; JSON serializers never emit one).
fn read_frame(r: &mut impl Read) -> Option<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    r.read_exact(&mut len_bytes).ok()?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len == 0 || len > bus::MAX_LINE_BYTES {
        return None;
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).ok()?;
    if payload.contains(&b'\n') {
        return None;
    }
    Some(payload)
}

/// Writes one native-messaging frame and flushes (the browser reads frames,
/// not a stream - an unflushed reply is an undelivered reply).
fn write_frame(w: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// `portunus native-host install <name> --ff-ext-id <id@domain>`: writes the
/// wrapper script + Firefox native-messaging manifest for one extension.
fn install(args: &[String]) -> i32 {
    let Some(name) = args.first().filter(|n| bus::valid_name(n)) else {
        eprintln!("usage: portunus native-host install <extension-name> --ff-ext-id <id@domain>");
        return 2;
    };
    let ff_ext_id = match args.iter().position(|a| a == "--ff-ext-id").and_then(|p| args.get(p + 1))
    {
        Some(id) if !id.is_empty() && id.len() <= 128 && !id.contains(['"', '\\', '\n']) => {
            id.clone()
        }
        _ => {
            eprintln!("portunus native-host install: --ff-ext-id <id@domain> is required (the browser extension's id)");
            return 2;
        }
    };
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("portunus native-host install: cannot resolve own executable path");
        return 1;
    };

    // Wrapper script: the browser invokes the manifest's `path` with fixed
    // args of its own, so the subcommand + name must be baked into a script.
    let script_dir = crate::paths::data_dir().join("native-host");
    if let Err(e) = std::fs::create_dir_all(&script_dir) {
        eprintln!("portunus native-host install: {}: {e}", script_dir.display());
        return 1;
    }
    let script_path = script_dir.join(format!("{name}.sh"));
    let script = format!("#!/bin/sh\nexec \"{}\" native-host {name}\n", exe.display());
    if let Err(e) = std::fs::write(&script_path, script) {
        eprintln!("portunus native-host install: {}: {e}", script_path.display());
        return 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755));
    }

    // Native-messaging host names allow [a-z0-9._] segments - map '-' to '_'.
    let host_name = format!("portunus_{}", name.replace('-', "_"));
    let manifest = serde_json::json!({
        "name": host_name,
        "description": format!("Portunus message-bus native host for the \"{name}\" extension"),
        "path": script_path,
        "type": "stdio",
        "allowed_extensions": [ff_ext_id],
    });
    let Some(home) = std::env::var_os("HOME") else {
        eprintln!("portunus native-host install: $HOME is not set");
        return 1;
    };
    let manifest_dir = std::path::Path::new(&home).join(".mozilla/native-messaging-hosts");
    if let Err(e) = std::fs::create_dir_all(&manifest_dir) {
        eprintln!("portunus native-host install: {}: {e}", manifest_dir.display());
        return 1;
    }
    let manifest_path = manifest_dir.join(format!("{host_name}.json"));
    match serde_json::to_string_pretty(&manifest)
        .map_err(|e| e.to_string())
        .and_then(|s| std::fs::write(&manifest_path, s + "\n").map_err(|e| e.to_string()))
    {
        Ok(()) => {
            println!("wrote {}", script_path.display());
            println!("wrote {}", manifest_path.display());
            println!("native host \"{host_name}\" ready - reload the browser extension to connect");
            0
        }
        Err(e) => {
            eprintln!("portunus native-host install: {}: {e}", manifest_path.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let payload = r#"{"id":1,"payload":{"op":"search","q":"café"}}"#.as_bytes();
        let mut framed = Vec::new();
        write_frame(&mut framed, payload).expect("write");
        let mut cursor = std::io::Cursor::new(framed);
        let out = read_frame(&mut cursor).expect("read");
        assert_eq!(out, payload);
        // EOF after the single frame.
        assert!(read_frame(&mut cursor).is_none());
    }

    #[test]
    fn frame_rejects_oversize_and_newline() {
        let mut oversize = Vec::new();
        oversize.extend_from_slice(&((bus::MAX_LINE_BYTES as u32) + 1).to_le_bytes());
        oversize.extend(std::iter::repeat_n(b'x', 8));
        assert!(read_frame(&mut std::io::Cursor::new(oversize)).is_none());

        let mut newline = Vec::new();
        write_frame(&mut newline, b"{\"a\":\n1}").expect("write");
        assert!(read_frame(&mut std::io::Cursor::new(newline)).is_none());
    }

    #[test]
    fn frame_rejects_truncated() {
        let mut framed = Vec::new();
        write_frame(&mut framed, b"{\"id\":1}").expect("write");
        framed.truncate(framed.len() - 3);
        assert!(read_frame(&mut std::io::Cursor::new(framed)).is_none());
    }
}

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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::extensions::bus;

/// Delay between socket-connect attempts while portunus is not yet up.
const CONNECT_RETRY: Duration = Duration::from_secs(1);
/// Cap on stdin frames buffered before the socket connects. The browser sends
/// nothing until we message it, so this only bounds a misbehaving peer.
const MAX_PREBUFFER: usize = 64;

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

/// The live socket write half plus frames buffered while disconnected. Shared
/// between the persistent stdin thread (producer) and the main connect loop
/// (which swaps the writer on each (re)connect).
#[derive(Default)]
struct Conn {
    /// Write half of the current socket, or `None` while disconnected.
    writer: Option<UnixStream>,
    /// Frames read from the browser before/between connections. Normally empty
    /// (the browser only replies after we message it); bounded regardless.
    prebuf: Vec<Vec<u8>>,
}

/// Relays stdio native-messaging frames ↔ the socket's ext-attach channel.
///
/// The browser spawns this process; it may start before portunus is up (login
/// race) or outlive a portunus restart. So the connect is retried instead of
/// giving up: staying alive keeps the browser's native port open, which keeps
/// an MV3 event page from being suspended, so the bridge re-establishes within
/// [`CONNECT_RETRY`] of portunus appearing. The only clean exit is the browser
/// closing stdin (`browser_gone`).
fn pump(name: &str) -> i32 {
    let browser_gone = Arc::new(AtomicBool::new(false));
    let conn = Arc::new(std::sync::Mutex::new(Conn::default()));

    // Browser → portunus: one reader thread for the whole process lifetime.
    // Writes straight to the live socket; buffers if disconnected. On stdin EOF
    // the browser is gone - flag it and close any live socket so the main
    // loop's blocking read unblocks and the process exits.
    {
        let browser_gone = browser_gone.clone();
        let conn = conn.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            while let Some(frame) = read_frame(&mut stdin) {
                let mut c = conn.lock().unwrap();
                if let Some(w) = c.writer.as_mut() {
                    let _ = w.write_all(&frame).and_then(|_| w.write_all(b"\n"));
                } else if c.prebuf.len() < MAX_PREBUFFER {
                    c.prebuf.push(frame);
                } else {
                    eprintln!(
                        "portunus native-host: dropping stdin frame (over {MAX_PREBUFFER} buffered before connect)"
                    );
                }
            }
            browser_gone.store(true, Ordering::Relaxed);
            if let Some(w) = conn.lock().unwrap().writer.take() {
                let _ = w.shutdown(std::net::Shutdown::Both);
            }
        });
    }

    loop {
        if browser_gone.load(Ordering::Relaxed) {
            return 0;
        }
        let socket = match UnixStream::connect(crate::ipc::socket_path()) {
            Ok(s) => s,
            Err(_) => {
                // Portunus not up yet. Wait and retry unless the browser left.
                std::thread::sleep(CONNECT_RETRY);
                continue;
            }
        };
        // Publish the writer + flush anything buffered while disconnected.
        {
            let mut w = match socket.try_clone() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("portunus native-host: {e}");
                    std::thread::sleep(CONNECT_RETRY);
                    continue;
                }
            };
            if w.write_all(format!("ext-attach:{name}\n").as_bytes()).is_err() {
                std::thread::sleep(CONNECT_RETRY);
                continue;
            }
            let mut c = conn.lock().unwrap();
            for frame in c.prebuf.drain(..) {
                let _ = w.write_all(&frame).and_then(|_| w.write_all(b"\n"));
            }
            c.writer = Some(w);
        }

        // Portunus → browser, on this thread, until the socket closes. A
        // portunus-side close (EOF) drops us back to the reconnect loop; a
        // restarted portunus rebinds and we re-attach.
        socket_to_stdout(&socket);
        conn.lock().unwrap().writer = None;
    }
}

/// Reads socket NDJSON lines and writes them to the browser as native-messaging
/// frames until the socket closes or a stdout write fails.
fn socket_to_stdout(socket: &UnixStream) {
    let mut reader = std::io::BufReader::new(socket);
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

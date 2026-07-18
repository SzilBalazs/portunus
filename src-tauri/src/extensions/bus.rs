//! Extension message bus: a persistent channel between one extension and one
//! external companion process (a browser extension's native-messaging host,
//! an editor plugin, ...).
//!
//! A companion connects to the portunus socket and sends `ext-attach:<name>`;
//! the connection then stays open as a newline-delimited JSON duplex channel
//! (see [`attach`]). The guest talks to it through the `bus_*` host functions
//! (gated by the `bus` manifest permission): request/response with host-side
//! correlation ids, plus fire-and-forget notifies. The companion may only
//! *reply* to requests - unsolicited inbound lines are dropped.
//!
//! Trust model: the socket is user-only (0700 `$XDG_RUNTIME_DIR`), so an
//! attaching process already runs with the user's authority - the same trust
//! boundary as the existing `--reload-*` socket verbs. Companion input is
//! still clamped (line length, pending-request count) before any of it is
//! handed to the wasm guest.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::util;

/// Cap per NDJSON line, both directions. A companion exceeding it is
/// disconnected (a broken framer would otherwise buffer unboundedly).
pub const MAX_LINE_BYTES: usize = 256 * 1024;
/// Concurrent in-flight `bus_request`s per extension.
const MAX_PENDING: usize = 16;
/// Hard cap on one `bus_request` wait, whatever the guest asked for.
pub const MAX_REQUEST_TIMEOUT_MS: u64 = 30_000;
/// Poll granularity of the request wait loop - each tick re-checks the
/// caller's cancellation gate so a dead companion never holds a cancelled
/// query for the full timeout.
const WAIT_TICK: Duration = Duration::from_millis(50);

pub static BUS: LazyLock<Bus> = LazyLock::new(Bus::default);

/// One live companion connection.
struct Channel {
    writer: Mutex<UnixStream>,
    pending: Mutex<HashMap<u64, mpsc::Sender<String>>>,
    next_id: AtomicU64,
    closed: AtomicBool,
}

impl Channel {
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        // Both halves: the reader loop wakes on EOF, blocked writers error out.
        let _ = util::lock(&self.writer).shutdown(std::net::Shutdown::Both);
    }

    fn write_line(&self, line: &str) -> Result<(), String> {
        if self.closed.load(Ordering::Relaxed) {
            return Err("companion disconnected".to_string());
        }
        let mut w = util::lock(&self.writer);
        w.write_all(line.as_bytes())
            .and_then(|_| w.write_all(b"\n"))
            .map_err(|e| format!("bus write: {e}"))
    }
}

/// Wire envelope. Requests carry an `id` the companion must echo in its
/// reply; notifies have no `id` and expect no reply.
#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    payload: serde_json::Value,
}

#[derive(Default)]
pub struct Bus {
    channels: Mutex<HashMap<String, Arc<Channel>>>,
}

impl Bus {
    /// Whether a companion is currently attached for `name`.
    pub fn attached(&self, name: &str) -> bool {
        util::lock(&self.channels)
            .get(name)
            .is_some_and(|c| !c.closed.load(Ordering::Relaxed))
    }

    /// Sends a fire-and-forget message to the companion.
    pub fn notify(&self, name: &str, payload: serde_json::Value) -> Result<(), String> {
        let ch = self.channel(name)?;
        let line = serde_json::to_string(&Envelope { id: None, payload })
            .map_err(|e| e.to_string())?;
        if line.len() > MAX_LINE_BYTES {
            return Err(format!("bus message exceeds {MAX_LINE_BYTES} bytes"));
        }
        ch.write_line(&line)
    }

    /// Sends a request and blocks (in [`WAIT_TICK`] slices) for the reply.
    /// `cancelled` is re-checked every tick so a caller with a cancellation
    /// gate (the async query tier) unblocks promptly. Returns the reply
    /// payload, or an error on timeout/disconnect/cancellation.
    pub fn request(
        &self,
        name: &str,
        payload: serde_json::Value,
        timeout_ms: u64,
        cancelled: impl Fn() -> bool,
    ) -> Result<serde_json::Value, String> {
        let ch = self.channel(name)?;
        let (tx, rx) = mpsc::channel::<String>();
        let id = ch.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut pending = util::lock(&ch.pending);
            if pending.len() >= MAX_PENDING {
                return Err(format!("more than {MAX_PENDING} concurrent bus requests"));
            }
            pending.insert(id, tx);
        }
        // Always deregister, whatever path exits the wait below.
        let _cleanup = Cleanup(|| {
            util::lock(&ch.pending).remove(&id);
        });

        let line = serde_json::to_string(&Envelope { id: Some(id), payload })
            .map_err(|e| e.to_string())?;
        if line.len() > MAX_LINE_BYTES {
            return Err(format!("bus message exceeds {MAX_LINE_BYTES} bytes"));
        }
        ch.write_line(&line)?;

        let deadline =
            Instant::now() + Duration::from_millis(timeout_ms.min(MAX_REQUEST_TIMEOUT_MS));
        loop {
            match rx.recv_timeout(WAIT_TICK) {
                Ok(raw) => {
                    let env: Envelope = serde_json::from_str(&raw)
                        .map_err(|e| format!("bus reply: invalid json: {e}"))?;
                    return Ok(env.payload);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if cancelled() {
                        return Err("bus request cancelled".to_string());
                    }
                    if ch.closed.load(Ordering::Relaxed) {
                        return Err("companion disconnected".to_string());
                    }
                    if Instant::now() >= deadline {
                        return Err("bus request timed out".to_string());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("companion disconnected".to_string());
                }
            }
        }
    }

    fn channel(&self, name: &str) -> Result<Arc<Channel>, String> {
        util::lock(&self.channels)
            .get(name)
            .filter(|c| !c.closed.load(Ordering::Relaxed))
            .cloned()
            .ok_or_else(|| "no companion attached".to_string())
    }

    /// Registers a fresh channel for `name`, closing any previous attachment
    /// (newest wins - a restarted companion must not be locked out by its
    /// dead predecessor). Returns the channel for the reader loop.
    fn register(&self, name: &str, writer: UnixStream) -> Arc<Channel> {
        let ch = Arc::new(Channel {
            writer: Mutex::new(writer),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        });
        if let Some(old) = util::lock(&self.channels).insert(name.to_string(), ch.clone()) {
            old.close();
        }
        ch
    }

    /// Removes `ch` if it is still the current attachment for `name` (a
    /// replacement may already have taken the slot).
    fn deregister(&self, name: &str, ch: &Arc<Channel>) {
        let mut map = util::lock(&self.channels);
        if map.get(name).is_some_and(|cur| Arc::ptr_eq(cur, ch)) {
            map.remove(name);
        }
    }
}

/// Valid bus/extension name: same grammar the manifest enforces, so an
/// attach for a not-yet-installed extension is representable but junk isn't.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Takes over an `ext-attach:<name>` socket connection: registers the channel
/// and runs the reader loop until EOF/error. Call from the per-connection
/// thread; `reader` must wrap the same stream as `writer` (it may hold bytes
/// buffered past the attach line).
pub fn attach(
    name: &str,
    mut reader: std::io::BufReader<UnixStream>,
    writer: UnixStream,
) {
    let ch = BUS.register(name, writer);
    super::logs::log(name, super::logs::LogLevel::Info, "bus: companion attached");

    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        // Bounded read: a line past the cap disconnects the companion instead
        // of buffering without limit.
        let mut limited = std::io::Read::take(&mut reader, (MAX_LINE_BYTES + 1) as u64);
        match limited.read_until(b'\n', &mut buf) {
            Ok(0) => break, // EOF
            Ok(_) if buf.len() > MAX_LINE_BYTES => {
                super::logs::log(
                    name,
                    super::logs::LogLevel::Error,
                    &format!("bus: companion sent a line over {MAX_LINE_BYTES} bytes - disconnected"),
                );
                break;
            }
            Ok(_) => {}
            Err(e) => {
                if !ch.closed.load(Ordering::Relaxed) {
                    super::logs::log(
                        name,
                        super::logs::LogLevel::Error,
                        &format!("bus: read error: {e}"),
                    );
                }
                break;
            }
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Route replies to their waiting request; drop everything else (the
        // companion may only reply - it never initiates).
        let id = serde_json::from_str::<Envelope>(line).ok().and_then(|e| e.id);
        let Some(id) = id else {
            super::logs::log(
                name,
                super::logs::LogLevel::Error,
                "bus: dropped unsolicited/invalid line from companion",
            );
            continue;
        };
        if let Some(tx) = util::lock(&ch.pending).remove(&id) {
            let _ = tx.send(line.to_string());
        }
        // Unknown id: the request already timed out/cancelled - drop silently.
    }

    ch.close();
    BUS.deregister(name, &ch);
    super::logs::log(name, super::logs::LogLevel::Info, "bus: companion detached");
}

/// Runs a closure on drop - request-path cleanup that must survive early
/// returns.
struct Cleanup<F: FnMut()>(F);
impl<F: FnMut()> Drop for Cleanup<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipe() -> (UnixStream, UnixStream) {
        UnixStream::pair().expect("socketpair")
    }

    /// Spawns an attach loop for `name` over an in-process socket pair and
    /// returns the companion's end.
    fn attach_pair(name: &str) -> UnixStream {
        let (host_end, companion_end) = pipe();
        let reader = std::io::BufReader::new(host_end.try_clone().expect("clone"));
        let name = name.to_string();
        let thread_name = name.clone();
        std::thread::spawn(move || attach(&thread_name, reader, host_end));
        // Wait until registered.
        for _ in 0..100 {
            if BUS.attached(&name) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        companion_end
    }

    #[test]
    fn valid_name_grammar() {
        assert!(valid_name("ytm"));
        assert!(valid_name("my-ext_2"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(&"x".repeat(65)));
    }

    #[test]
    fn request_reply_roundtrip() {
        let companion = attach_pair("bus-test-rt");
        // Companion: echo every request back with the same id.
        let echo = companion.try_clone().expect("clone");
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(echo);
            let mut line = String::new();
            let mut w = companion.try_clone().expect("clone");
            while {
                line.clear();
                reader.read_line(&mut line).map(|n| n > 0).unwrap_or(false)
            } {
                let env: Envelope = serde_json::from_str(line.trim()).expect("valid envelope");
                let reply = serde_json::to_string(&Envelope {
                    id: env.id,
                    payload: serde_json::json!({"echo": env.payload}),
                })
                .expect("serialize");
                let _ = writeln!(w, "{reply}");
            }
        });
        let out = BUS
            .request("bus-test-rt", serde_json::json!({"q": 1}), 2000, || false)
            .expect("reply");
        assert_eq!(out, serde_json::json!({"echo": {"q": 1}}));
    }

    #[test]
    fn request_times_out_without_reply() {
        let _companion = attach_pair("bus-test-timeout");
        let err = BUS
            .request("bus-test-timeout", serde_json::json!({}), 120, || false)
            .expect_err("must time out");
        assert!(err.contains("timed out"), "{err}");
    }

    #[test]
    fn request_cancels_promptly() {
        let _companion = attach_pair("bus-test-cancel");
        let start = Instant::now();
        let err = BUS
            .request("bus-test-cancel", serde_json::json!({}), 10_000, || true)
            .expect_err("must cancel");
        assert!(err.contains("cancelled"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn no_companion_errors() {
        assert!(!BUS.attached("bus-test-none"));
        let err = BUS
            .request("bus-test-none", serde_json::json!({}), 100, || false)
            .expect_err("no channel");
        assert!(err.contains("no companion"), "{err}");
        assert!(BUS.notify("bus-test-none", serde_json::json!({})).is_err());
    }

    #[test]
    fn newest_attachment_wins() {
        let first = attach_pair("bus-test-replace");
        let _second = attach_pair("bus-test-replace");
        // First connection is closed by the replacement: reads hit EOF.
        let mut reader = std::io::BufReader::new(first);
        let mut line = String::new();
        // Give the replacement a moment to close the old channel.
        std::thread::sleep(Duration::from_millis(50));
        let n = reader.read_line(&mut line).unwrap_or(0);
        assert_eq!(n, 0, "old companion should see EOF");
        assert!(BUS.attached("bus-test-replace"));
    }

    #[test]
    fn oversize_line_disconnects() {
        let mut companion = attach_pair("bus-test-oversize");
        let huge = "x".repeat(MAX_LINE_BYTES + 10);
        let _ = writeln!(companion, "{huge}");
        for _ in 0..100 {
            if !BUS.attached("bus-test-oversize") {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("oversize line should disconnect the companion");
    }
}

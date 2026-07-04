//! Per-extension log capture: a small in-memory ring buffer per extension,
//! feeding the log viewer in Settings.
//!
//! Lives as Tauri-managed state independent of provider instances, so logs
//! survive reloads and load failures are visible even when no provider ever
//! came up. stderr remains the firehose; this is the last-N window.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Max entries retained per extension.
const RING_CAP: usize = 200;
/// Hard cap on message bytes stored per entry (stderr already clamps at 4 KB).
const MAX_MESSAGE_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LogEntry {
    /// Wall-clock ms since the Unix epoch.
    pub ts_ms: u64,
    pub level: LogLevel,
    pub message: String,
}

/// Process-global log store. Like stderr, extension logging is ambient
/// diagnostics - a global avoids threading a handle through every load path
/// (hostfns, provider errors, sync failures, CLI-triggered reloads).
pub static LOGS: std::sync::LazyLock<ExtensionLogs> =
    std::sync::LazyLock::new(ExtensionLogs::default);

/// Convenience: log to both stderr and the ring buffer.
pub fn log(extension: &str, level: LogLevel, message: &str) {
    eprintln!("[ext:{extension}] {message}");
    LOGS.push(extension, level, message);
}

#[derive(Clone, Default)]
pub struct ExtensionLogs(Arc<Mutex<HashMap<String, VecDeque<LogEntry>>>>);

impl ExtensionLogs {

    pub fn push(&self, extension: &str, level: LogLevel, message: &str) {
        let mut message = message.to_string();
        if message.len() > MAX_MESSAGE_BYTES {
            let mut cut = MAX_MESSAGE_BYTES;
            while !message.is_char_boundary(cut) {
                cut -= 1;
            }
            message.truncate(cut);
        }
        let entry = LogEntry {
            ts_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            level,
            message,
        };
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let ring = map.entry(extension.to_string()).or_default();
        if ring.len() >= RING_CAP {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    /// Last `limit` entries, oldest first.
    pub fn tail(&self, extension: &str, limit: usize) -> Vec<LogEntry> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(extension)
            .map(|ring| ring.iter().rev().take(limit).rev().cloned().collect())
            .unwrap_or_default()
    }

    /// Drops all entries for one extension (uninstall).
    pub fn purge(&self, extension: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(extension);
    }
}

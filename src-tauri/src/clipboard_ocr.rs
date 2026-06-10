//! Cache of OCR'd text for clipboard images, keyed by cliphist entry id.
//!
//! OCR is expensive, so each copied image is OCR'd once and the result is stored
//! here; subsequent clipboard searches read the cache instead of re-running
//! Tesseract. Mirrors the `frecency.rs` SQLite-store shape. Clipboard history is
//! small (≤ `max_entries`, default 250), so a plain text column + substring
//! match on the frontend suffices - no FTS needed.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, Result};

use crate::util;

pub struct ClipboardOcrStore {
    conn: Mutex<Connection>,
}

fn db_path() -> PathBuf {
    let data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{home}/.local/share")
    });
    PathBuf::from(data_home).join("portunus").join("clipboard_ocr.db")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64
}

impl ClipboardOcrStore {
    pub fn open() -> Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = crate::util::open_sqlite_resilient(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS clipboard_ocr (
                 id         TEXT PRIMARY KEY,
                 byte_size  INTEGER NOT NULL DEFAULT 0,
                 text       TEXT NOT NULL,
                 indexed_at INTEGER NOT NULL
             );",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Cached `(byte_size, text)` for an entry, or `None` if never OCR'd. An
    /// empty `text` is a valid negative-cache hit (image had no detectable text).
    pub fn get(&self, id: &str) -> Option<(u64, String)> {
        let conn = util::lock(&self.conn);
        conn.query_row(
            "SELECT byte_size, text FROM clipboard_ocr WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
        )
        .ok()
    }

    pub fn upsert(&self, id: &str, byte_size: u64, text: &str) {
        let conn = util::lock(&self.conn);
        let _ = conn.execute(
            "INSERT INTO clipboard_ocr (id, byte_size, text, indexed_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET byte_size = ?2, text = ?3, indexed_at = ?4",
            params![id, byte_size as i64, text, now_secs()],
        );
    }

    /// Drops rows for entries no longer present in cliphist, so the cache doesn't
    /// outgrow the history. Called at the end of an index pass with the live ids.
    pub fn prune(&self, live_ids: &HashSet<String>) {
        let conn = util::lock(&self.conn);
        let stale: Vec<String> = {
            let mut stmt = match conn.prepare("SELECT id FROM clipboard_ocr") {
                Ok(s) => s,
                Err(_) => return,
            };
            let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
                Ok(r) => r,
                Err(_) => return,
            };
            rows.flatten().filter(|id| !live_ids.contains(id)).collect()
        };
        for id in stale {
            let _ = conn.execute("DELETE FROM clipboard_ocr WHERE id = ?1", params![id]);
        }
    }
}

//! Per-extension key-value storage backing the `kv_get`/`kv_set` host functions.
//!
//! One SQLite database shared by all extensions; rows are namespaced by the
//! extension name so extensions can never read each other's data. Each
//! extension gets a byte quota so a buggy/malicious module can't fill the disk.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::util;

/// Total bytes (keys + values) one extension may store.
const QUOTA_BYTES: i64 = 10 * 1024 * 1024;

pub struct ExtensionKv {
    conn: Mutex<Connection>,
}

fn db_path() -> PathBuf {
    let data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{home}/.local/share")
    });
    PathBuf::from(data_home)
        .join("portunus")
        .join("extension_kv.sqlite")
}

impl ExtensionKv {
    pub fn open() -> rusqlite::Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = util::open_sqlite_resilient(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS kv (
                 extension TEXT NOT NULL,
                 key       TEXT NOT NULL,
                 value     TEXT NOT NULL,
                 PRIMARY KEY (extension, key)
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Last-resort fallback when the on-disk store can't be opened: keeps the
    /// host functions working for the session, data is simply not persisted.
    pub fn open_in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("in-memory sqlite cannot fail");
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS kv (
                 extension TEXT NOT NULL,
                 key       TEXT NOT NULL,
                 value     TEXT NOT NULL,
                 PRIMARY KEY (extension, key)
             );",
        );
        Self {
            conn: Mutex::new(conn),
        }
    }

    pub fn get(&self, extension: &str, key: &str) -> Option<String> {
        let conn = util::lock(&self.conn);
        conn.query_row(
            "SELECT value FROM kv WHERE extension = ?1 AND key = ?2",
            params![extension, key],
            |row| row.get(0),
        )
        .ok()
    }

    pub fn set(&self, extension: &str, key: &str, value: &str) -> Result<(), String> {
        let conn = util::lock(&self.conn);
        // Quota check: current usage minus the row being replaced, plus the new row.
        let used: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(key) + LENGTH(value)), 0) FROM kv
                 WHERE extension = ?1 AND key != ?2",
                params![extension, key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let incoming = (key.len() + value.len()) as i64;
        if used + incoming > QUOTA_BYTES {
            return Err(format!(
                "kv quota exceeded ({QUOTA_BYTES} bytes per extension)"
            ));
        }
        conn.execute(
            "INSERT INTO kv (extension, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(extension, key) DO UPDATE SET value = ?3",
            params![extension, key, value],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Keys matching a prefix in one extension's namespace, capped — feeds the
    /// `kv_list` host function (cache enumeration/eviction).
    pub fn list(&self, extension: &str, prefix: &str) -> Vec<String> {
        const MAX_KEYS: usize = 10_000;
        // Escape LIKE metacharacters in the prefix (same hazard as
        // frecency::delete_prefix: '_' is a single-char wildcard).
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let conn = util::lock(&self.conn);
        let mut keys = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT key FROM kv WHERE extension = ?1 AND key LIKE ?2 || '%' ESCAPE '\\'
             ORDER BY key LIMIT ?3",
        ) {
            if let Ok(rows) = stmt.query_map(params![extension, escaped, MAX_KEYS as i64], |row| {
                row.get::<_, String>(0)
            }) {
                keys.extend(rows.flatten());
            }
        }
        keys
    }

    /// Deletes one key from an extension's namespace.
    pub fn delete(&self, extension: &str, key: &str) {
        let conn = util::lock(&self.conn);
        let _ = conn.execute(
            "DELETE FROM kv WHERE extension = ?1 AND key = ?2",
            params![extension, key],
        );
    }

    /// Names of every extension with stored data — used to find orphans after
    /// an extension directory disappears.
    pub fn extension_names(&self) -> Vec<String> {
        let conn = util::lock(&self.conn);
        let mut names = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT extension FROM kv") {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                names.extend(rows.flatten());
            }
        }
        names
    }

    /// Drops all rows for an uninstalled extension (kv may hold secrets — the
    /// user deleting the extension expects its data gone too).
    pub fn delete_extension(&self, extension: &str) {
        let conn = util::lock(&self.conn);
        let _ = conn.execute("DELETE FROM kv WHERE extension = ?1", params![extension]);
    }
}

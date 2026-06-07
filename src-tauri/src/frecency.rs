use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, Result};

use crate::util;

pub struct FrecencyStore {
    conn: Mutex<Connection>,
    half_life_days: f32,
    // In-memory mirror of the (id → score) table, so a search can apply frecency
    // bonuses without hitting SQLite on every keystroke. Kept in sync by
    // record_launch and rebuilt from disk on open.
    cache: RwLock<HashMap<String, f32>>,
}

fn db_path() -> PathBuf {
    let data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{home}/.local/share")
    });
    PathBuf::from(data_home).join("portunus").join("frecency.db")
}

impl FrecencyStore {
    pub fn open(half_life_days: f32) -> Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = crate::util::open_sqlite_resilient(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS frecency (
                 id            TEXT PRIMARY KEY,
                 kind          TEXT NOT NULL,
                 score         REAL NOT NULL DEFAULT 0.0,
                 last_launched INTEGER NOT NULL
             );",
        )?;
        let store = Self {
            conn: Mutex::new(conn),
            half_life_days,
            cache: RwLock::new(HashMap::new()),
        };
        store.reload_cache();
        Ok(store)
    }

    /// Rebuilds the in-memory score cache from the database.
    fn reload_cache(&self) {
        let conn = util::lock(&self.conn);
        let mut map = HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id, score FROM frecency") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)? as f32))
            }) {
                map.extend(rows.flatten());
            }
        }
        *util::write(&self.cache) = map;
    }

    pub fn record_launch(&self, id: &str, kind: &str) {
        if !matches!(kind, "app" | "file" | "folder" | "extension") {
            return;
        }
        // Normalize recent:<path> → file:<path> so both providers share one frecency score.
        let normalized = if let Some(path) = id.strip_prefix("recent:") {
            format!("file:{path}")
        } else {
            id.to_string()
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;

        let conn = util::lock(&self.conn);

        let existing = conn
            .query_row(
                "SELECT score, last_launched FROM frecency WHERE id = ?1",
                params![normalized],
                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok();

        let new_score = match existing {
            Some((old_score, last_launched)) => {
                let elapsed_days = (now - last_launched).max(0) as f64 / 86400.0;
                old_score * 2f64.powf(-elapsed_days / self.half_life_days as f64) + 1.0
            }
            None => 1.0,
        };

        let written = conn.execute(
            "INSERT INTO frecency (id, kind, score, last_launched) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET score = ?3, last_launched = ?4",
            params![normalized, kind, new_score, now],
        );
        drop(conn);
        // Mirror the new score into the cache so the next search sees it without
        // a DB round-trip. Only on a successful write, to avoid drift from disk.
        if written.is_ok() {
            util::write(&self.cache).insert(normalized, new_score as f32);
        }
    }

    pub fn all_scores(&self) -> HashMap<String, f32> {
        util::read(&self.cache).clone()
    }

    /// Removes every record whose id starts with `prefix` — used when an
    /// extension is uninstalled (`ext:<name>:`), so its history doesn't
    /// outlive it.
    pub fn delete_prefix(&self, prefix: &str) {
        // Escape LIKE metacharacters: extension names may contain '_', which
        // LIKE would treat as a single-char wildcard and match neighbors
        // (deleting `ext:my_ext:%` must not hit `ext:my-ext:...`).
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let conn = util::lock(&self.conn);
        let _ = conn.execute(
            "DELETE FROM frecency WHERE id LIKE ?1 || '%' ESCAPE '\\'",
            params![escaped],
        );
        drop(conn);
        util::write(&self.cache).retain(|k, _| !k.starts_with(prefix));
    }

    /// Extension names that have frecency history (`ext:<name>:...` ids).
    /// Part of the uninstall-orphan census: an extension that never wrote kv
    /// data is otherwise invisible after its directory is deleted.
    pub fn extension_names(&self) -> Vec<String> {
        let cache = util::read(&self.cache);
        let mut names: Vec<String> = cache
            .keys()
            .filter_map(|id| id.strip_prefix("ext:"))
            .filter_map(|rest| rest.split(':').next())
            .map(str::to_string)
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

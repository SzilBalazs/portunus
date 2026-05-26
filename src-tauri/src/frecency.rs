use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, Result};

pub struct FrecencyStore {
    conn: Mutex<Connection>,
    half_life_days: f32,
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
        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS frecency (
                 id            TEXT PRIMARY KEY,
                 kind          TEXT NOT NULL,
                 score         REAL NOT NULL DEFAULT 0.0,
                 last_launched INTEGER NOT NULL
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            half_life_days,
        })
    }

    pub fn record_launch(&self, id: &str, kind: &str) {
        if !matches!(kind, "app" | "file" | "folder") {
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

        let conn = self.conn.lock().unwrap();

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

        let _ = conn.execute(
            "INSERT INTO frecency (id, kind, score, last_launched) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET score = ?3, last_launched = ?4",
            params![normalized, kind, new_score, now],
        );
    }

    pub fn all_scores(&self) -> HashMap<String, f32> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT id, score FROM frecency") {
            Ok(s) => s,
            Err(_) => return HashMap::new(),
        };
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)? as f32))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }
}

//! Small shared helpers.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use rusqlite::Connection;

/// Locks a `Mutex`, recovering rather than panicking if a previous holder
/// panicked and poisoned it. Our locks guard caches/indexes where the worst a
/// stale-after-panic read can do is return slightly-off data — far better than
/// cascading a single background panic into a crash on every later access.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Read-locks an `RwLock`, recovering from poisoning. See [`lock`].
pub fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|e| e.into_inner())
}

/// Write-locks an `RwLock`, recovering from poisoning. See [`lock`].
pub fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|e| e.into_inner())
}

/// Returns true if `bin` is found as an executable file on any PATH entry.
/// Used both to gate providers at startup and to report dependency status
/// to the Settings UI via `check_dependencies`.
pub fn binary_in_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
    })
}

/// Opens a SQLite database, recreating it from scratch if it fails a `quick_check`
/// integrity probe. A corrupt DB (e.g. from a power loss mid-write) otherwise opens
/// fine but then yields cryptic per-query failures — for our caches (frecency,
/// content index) the right recovery is simply to discard and rebuild, so callers
/// get a usable connection instead of silent degradation. The `-wal`/`-shm` sidecars
/// are removed alongside the main file so the recreated DB starts clean.
pub fn open_sqlite_resilient(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    let healthy = conn
        .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
        .map(|s| s == "ok")
        .unwrap_or(false);
    if healthy {
        return Ok(conn);
    }
    eprintln!(
        "[db] integrity check failed for {} — recreating from scratch",
        path.display()
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
    for sidecar in ["-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(sidecar);
        let _ = std::fs::remove_file(std::path::PathBuf::from(p));
    }
    Connection::open(path)
}

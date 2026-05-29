use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rayon::prelude::*;
use rusqlite::{params, Connection};

use crate::config::ContentConfig;
use crate::util;

#[cfg(feature = "ocr")]
use std::cell::RefCell;

#[cfg(feature = "ocr")]
static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

// ── database ──────────────────────────────────────────────────────────────────

struct StoredMeta {
    mtime: i64,
    size: u64,
}

struct FileUpdate {
    path: String,
    text: String,
    mtime: i64,
    size: u64,
}

pub struct ContentIndex {
    db: Arc<Mutex<Connection>>,
}

impl ContentIndex {
    pub fn open() -> rusqlite::Result<Self> {
        let data_home = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            format!("{home}/.local/share")
        });
        let dir = std::path::PathBuf::from(data_home).join("portunus");
        std::fs::create_dir_all(&dir).ok();
        let db_path = dir.join("content_index.db");

        let conn = crate::util::open_sqlite_resilient(&db_path)?;

        // Detect old schema (has `hash` column) and drop if found — triggers one-time reindex.
        let has_hash: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('file_meta') WHERE name='hash'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if has_hash {
            conn.execute_batch("DROP TABLE IF EXISTS file_meta; DROP TABLE IF EXISTS content_fts;")?;
        }

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS file_meta (
                 path       TEXT    PRIMARY KEY,
                 mtime      INTEGER NOT NULL DEFAULT 0,
                 size       INTEGER NOT NULL DEFAULT 0,
                 indexed_at INTEGER NOT NULL
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS content_fts USING fts5(
                 path UNINDEXED,
                 text,
                 tokenize='porter unicode61'
             );",
        )?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    fn all_meta(&self) -> rusqlite::Result<HashMap<String, StoredMeta>> {
        let db = util::lock(&self.db);
        let mut stmt = db.prepare("SELECT path, mtime, size FROM file_meta")?;
        let map = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    StoredMeta {
                        mtime: row.get::<_, i64>(1)?,
                        size: row.get::<_, u64>(2)?,
                    },
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
    }

    fn upsert_batch(&self, updates: &[FileUpdate]) -> rusqlite::Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let mut db = util::lock(&self.db);
        let tx = db.transaction()?;
        for u in updates {
            tx.execute("DELETE FROM content_fts WHERE path = ?", [u.path.as_str()])?;
            tx.execute(
                "INSERT INTO content_fts(path, text) VALUES (?, ?)",
                params![u.path, u.text],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO file_meta(path, mtime, size, indexed_at) VALUES (?, ?, ?, ?)",
                params![u.path, u.mtime, u.size, now],
            )?;
        }
        tx.commit()
    }

    pub fn remove_stale(&self, live_paths: &HashSet<String>) -> rusqlite::Result<usize> {
        let mut db = util::lock(&self.db);
        let stale: Vec<String> = {
            let mut stmt = db.prepare("SELECT path FROM file_meta")?;
            let paths: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            paths
        }
        .into_iter()
        .filter(|p| !live_paths.contains(p))
        .collect();
        let count = stale.len();
        let tx = db.transaction()?;
        for path in &stale {
            tx.execute("DELETE FROM content_fts WHERE path = ?", [path.as_str()])?;
            tx.execute("DELETE FROM file_meta WHERE path = ?", [path.as_str()])?;
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn search(&self, fts_query: &str, limit: usize) -> rusqlite::Result<Vec<(String, f64, String, i64, u64)>> {
        let db = util::lock(&self.db);
        // \x02 = STX, \x03 = ETX — used as highlight start/end markers in the snippet.
        let mut stmt = db.prepare(
            "SELECT content_fts.path, rank, snippet(content_fts, 1, '\x02', '\x03', '…', 20), \
             COALESCE(m.mtime, 0), COALESCE(m.size, 0) \
             FROM content_fts LEFT JOIN file_meta m ON m.path = content_fts.path \
             WHERE content_fts.text MATCH ? ORDER BY rank LIMIT ?",
        )?;
        let results = stmt
            .query_map(params![fts_query, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, u64>(4)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    pub fn remove_path(&self, path: &str) -> rusqlite::Result<()> {
        let db = util::lock(&self.db);
        db.execute("DELETE FROM content_fts WHERE path = ?", [path])?;
        db.execute("DELETE FROM file_meta WHERE path = ?", [path])?;
        Ok(())
    }

    /// Removes all entries whose path starts with `dir_path/`. Used when a directory is
    /// deleted or moved out of the watched tree and no individual file events are fired.
    pub fn remove_prefix(&self, dir_path: &str) -> rusqlite::Result<usize> {
        let pattern = format!("{dir_path}/%");
        let mut db = util::lock(&self.db);
        let tx = db.transaction()?;
        let removed = tx.execute("DELETE FROM content_fts WHERE path LIKE ?", [&pattern])?;
        tx.execute("DELETE FROM file_meta WHERE path LIKE ?", [&pattern])?;
        tx.commit()?;
        Ok(removed)
    }

    fn get_meta(&self, path: &str) -> rusqlite::Result<Option<StoredMeta>> {
        let db = util::lock(&self.db);
        match db.query_row(
            "SELECT mtime, size FROM file_meta WHERE path = ?",
            [path],
            |r| Ok(StoredMeta { mtime: r.get(0)?, size: r.get(1)? }),
        ) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        let db = util::lock(&self.db);
        db.execute_batch(
            "DELETE FROM content_fts;
             DELETE FROM file_meta;",
        )
    }

    pub fn is_empty(&self) -> bool {
        let db = util::lock(&self.db);
        db.query_row("SELECT COUNT(*) FROM file_meta", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) == 0
    }
}

// ── text extraction ───────────────────────────────────────────────────────────

fn pdftotext(path: &str) -> Result<String, String> {
    let out = std::process::Command::new("pdftotext")
        .args([path, "-"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

#[cfg(feature = "ocr")]
thread_local! {
    static LEPTESS: RefCell<Option<leptess::LepTess>> = RefCell::new(None);
}

#[cfg(feature = "ocr")]
fn ocr_file(path: &str, lang: &str) -> Result<String, String> {
    LEPTESS.with(|cell| {
        let mut api_opt = cell.borrow_mut();
        if api_opt.is_none() {
            *api_opt = leptess::LepTess::new(None, lang).ok();
        }
        let api = api_opt.as_mut().ok_or("tesseract unavailable")?;
        api.set_image(path).map_err(|e| format!("{e:?}"))?;
        api.get_utf8_text().map_err(|e| format!("{e:?}"))
    })
}

fn extract_pdf(path: &str, ocr_fallback: bool, lang: &str) -> Result<String, String> {
    let text = pdftotext(path)?;
    #[cfg(not(feature = "ocr"))]
    {
        let _ = (ocr_fallback, lang);
        return Ok(text);
    }
    #[cfg(feature = "ocr")]
    {
        if text.trim().len() >= 50 || !ocr_fallback {
            return Ok(text);
        }

        // No meaningful text layer — render pages to images via pdftoppm and OCR each
        let tmp_dir = std::env::temp_dir().join(format!(
            "portunus_pdftoppm_{}_{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
        let prefix = tmp_dir.join("page");

        let result = std::process::Command::new("pdftoppm")
            .args(["-tiff", "-r", "150", path, prefix.to_str().unwrap_or("page")])
            .output();

        let ocr_text = match result {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::remove_dir_all(&tmp_dir).ok();
                return Ok(text);
            }
            Err(e) => {
                std::fs::remove_dir_all(&tmp_dir).ok();
                return Err(e.to_string());
            }
            Ok(out) if !out.status.success() => {
                std::fs::remove_dir_all(&tmp_dir).ok();
                return Ok(text);
            }
            Ok(_) => {
                let mut combined = String::new();
                let mut page_files: Vec<_> = std::fs::read_dir(&tmp_dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path().extension().and_then(|x| x.to_str()) == Some("tif")
                    })
                    .collect();
                page_files.sort_by_key(|e| e.file_name());
                for entry in page_files {
                    if let Some(p) = entry.path().to_str() {
                        match ocr_file(p, lang) {
                            Ok(t) => combined.push_str(&t),
                            Err(e) => eprintln!("[content] ocr page failed {p}: {e}"),
                        }
                    }
                }
                combined
            }
        };

        std::fs::remove_dir_all(&tmp_dir).ok();
        Ok(ocr_text)
    }
}

// Internal dispatch: these define HOW text is extracted, not which extensions are indexed.
// The user-facing extension list lives in ContentConfig::extensions.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rst", "csv", "log", "toml", "yaml", "yml", "json", "xml", "sh", "bash",
    "zsh", "py", "rs", "js", "ts", "go", "c", "cpp", "h", "hpp", "rb", "java", "kt",
];

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tiff", "tif"];

fn extract_text(path: &str, cfg: &ContentConfig) -> Result<String, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "pdf" {
        extract_pdf(path, cfg.ocr_pdf_fallback, &cfg.ocr_language)
    } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        #[cfg(feature = "ocr")]
        if cfg.ocr_images {
            return ocr_file(path, &cfg.ocr_language);
        }
        Err("image OCR disabled".to_string())
    } else if TEXT_EXTENSIONS.contains(&ext.as_str()) {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    } else if crate::office::is_office_ext(&ext) {
        crate::office::extract_office_text(path)
    } else {
        Err(format!("unsupported extension: {ext}"))
    }
}

// ── background indexer ────────────────────────────────────────────────────────

fn collect_eligible(cfg: &ContentConfig) -> Vec<(std::path::PathBuf, u64, i64)> {
    let mut eligible = Vec::new();
    for dir_entry in &cfg.dirs {
        let dir = crate::config::Config::expand_path(&dir_entry.path);
        if !dir.is_dir() {
            continue;
        }
        let effective_exts = dir_entry.extensions.as_ref().unwrap_or(&cfg.extensions);
        for entry in walkdir::WalkDir::new(&dir)
            .max_depth(dir_entry.depth)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.depth() == 0 || entry.file_type().is_dir() {
                continue;
            }
            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !effective_exts.contains(&ext) {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size = meta.len();
            if size > cfg.max_file_bytes {
                continue;
            }
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            eligible.push((entry.path().to_path_buf(), size, mtime));
        }
    }
    eligible
}

pub fn run_content_indexer(
    index: Arc<ContentIndex>,
    cfg: &ContentConfig,
    on_progress: Option<Arc<dyn Fn(usize, usize) + Send + Sync>>,
) {
    let eligible = collect_eligible(cfg);
    let total = eligible.len();

    if total > 0 {
        if let Some(ref cb) = on_progress {
            cb(0, total);
        }
    }

    let live_paths: HashSet<String> = eligible
        .iter()
        .map(|(p, _, _)| p.to_string_lossy().into_owned())
        .collect();

    let stored = match index.all_meta() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[content] failed to load metadata: {e}");
            HashMap::new()
        }
    };

    let processed = Arc::new(AtomicUsize::new(0));
    let cfg_ref = cfg;

    let pool_result = rayon::ThreadPoolBuilder::new()
        .num_threads(cfg.threads)
        .build();

    let updates: Vec<FileUpdate> = match pool_result {
        Ok(pool) => pool.install(|| {
            collect_updates(&eligible, &stored, cfg_ref, &processed, total, &on_progress)
        }),
        Err(_) => collect_updates(&eligible, &stored, cfg_ref, &processed, total, &on_progress),
    };

    let db_indexed = updates.len();
    match index.upsert_batch(&updates) {
        Ok(()) => {}
        Err(e) => eprintln!("[content] upsert_batch failed: {e}"),
    }

    match index.remove_stale(&live_paths) {
        Ok(removed) => eprintln!(
            "[content] done: total={total} indexed={db_indexed} removed={removed}"
        ),
        Err(e) => eprintln!("[content] remove_stale error: {e}"),
    }

    // Signal 100% only after the DB writes are committed, so the frontend
    // progress bar doesn't disappear while upsert_batch is still running.
    if total > 0 {
        if let Some(ref cb) = on_progress {
            cb(total, total);
        }
    }
}

fn collect_updates(
    eligible: &[(std::path::PathBuf, u64, i64)],
    stored: &HashMap<String, StoredMeta>,
    cfg: &ContentConfig,
    processed: &Arc<AtomicUsize>,
    total: usize,
    on_progress: &Option<Arc<dyn Fn(usize, usize) + Send + Sync>>,
) -> Vec<FileUpdate> {
    eligible
        .par_iter()
        .filter_map(|(path_buf, size, mtime)| {
            let path_str = path_buf.to_string_lossy().into_owned();

            // Fast-path: mtime+size unchanged → skip with no file read
            if let Some(m) = stored.get(&path_str) {
                if m.mtime == *mtime && m.size == *size {
                    let n = processed.fetch_add(1, Ordering::Relaxed) + 1;
                    if n % 10 == 0 && n < total {
                        if let Some(ref cb) = on_progress {
                            cb(n, total);
                        }
                    }
                    return None;
                }
            }

            let text = match extract_text(&path_str, cfg) {
                Ok(t) if !t.trim().is_empty() => t,
                Ok(_) => {
                    let n = processed.fetch_add(1, Ordering::Relaxed) + 1;
                    if n % 10 == 0 && n < total {
                        if let Some(ref cb) = on_progress {
                            cb(n, total);
                        }
                    }
                    return None;
                }
                Err(e) => {
                    eprintln!("[content] extract failed {path_str}: {e}");
                    let n = processed.fetch_add(1, Ordering::Relaxed) + 1;
                    if n % 10 == 0 && n < total {
                        if let Some(ref cb) = on_progress {
                            cb(n, total);
                        }
                    }
                    return None;
                }
            };

            let n = processed.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 10 == 0 && n < total {
                if let Some(ref cb) = on_progress {
                    cb(n, total);
                }
            }

            Some(FileUpdate {
                path: path_str,
                text,
                mtime: *mtime,
                size: *size,
            })
        })
        .collect()
}

// ── per-event helpers ─────────────────────────────────────────────────────────

/// Called for every filesystem event path. Returns true if the index was modified.
pub fn process_event_path(index: &Arc<ContentIndex>, path: &Path, cfg: &ContentConfig, log: bool) -> bool {
    let path_str = path.to_string_lossy().into_owned();

    if !path.exists() {
        if log { eprintln!("[content] event: path gone, removing — {path_str}"); }
        // Remove the path itself plus any children (handles directory deletions/moves-out
        // where individual file events are not fired for the contents).
        let removed_children = index.remove_prefix(&path_str).unwrap_or(0) > 0;
        let removed_self = index.remove_path(&path_str).is_ok();
        if log { eprintln!("[content] event: removed_children={removed_children} removed_self={removed_self}"); }
        return removed_children || removed_self;
    }

    if path.is_dir() {
        if log { eprintln!("[content] event: directory appeared, walking — {path_str}"); }
        // Directory appeared (created or moved in): walk it and index all eligible files.
        // Individual file Create events are not fired for files inside a moved-in directory.
        // Honour the per-dir depth limit from config, just like the initial index does.
        let walk_depth = cfg.dirs.iter()
            .filter_map(|d| {
                let base = crate::config::Config::expand_path(&d.path);
                let rel = path.strip_prefix(&base).ok()?;
                Some(d.depth.saturating_sub(rel.components().count()))
            })
            .max()
            .unwrap_or(usize::MAX);
        let mut changed = false;
        for entry in walkdir::WalkDir::new(path)
            .max_depth(walk_depth)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.depth() == 0 || entry.file_type().is_dir() {
                continue;
            }
            if process_event_path(index, entry.path(), cfg, log) {
                changed = true;
            }
        }
        return changed;
    }

    if !is_event_path_eligible(path, cfg) {
        if log { eprintln!("[content] event: not eligible — {path_str}"); }
        return index.remove_path(&path_str).is_ok();
    }

    let Ok(meta) = std::fs::metadata(path) else {
        if log { eprintln!("[content] event: metadata failed — {path_str}"); }
        return false;
    };
    let size = meta.len();
    if size > cfg.max_file_bytes {
        if log { eprintln!("[content] event: file too large ({size}) — {path_str}"); }
        return index.remove_path(&path_str).is_ok();
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    if let Ok(Some(m)) = index.get_meta(&path_str) {
        if m.mtime == mtime && m.size == size {
            if log { eprintln!("[content] event: mtime+size unchanged, skipping — {path_str}"); }
            return false;
        }
    }

    match extract_text(&path_str, cfg) {
        Ok(text) if !text.trim().is_empty() => {
            match index.upsert_batch(&[FileUpdate { path: path_str.clone(), text, mtime, size }]) {
                Ok(()) => { eprintln!("[content] indexed {path_str}"); true }
                Err(e) => { eprintln!("[content] event upsert failed {path_str}: {e}"); false }
            }
        }
        Ok(_) => index.remove_path(&path_str).is_ok(),
        Err(e) => { eprintln!("[content] event extract failed {path_str}: {e}"); false }
    }
}

fn is_event_path_eligible(path: &Path, cfg: &ContentConfig) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    for dir_entry in &cfg.dirs {
        let dir = crate::config::Config::expand_path(&dir_entry.path);
        if let Ok(rel) = path.strip_prefix(&dir) {
            let effective_exts = dir_entry.extensions.as_ref().unwrap_or(&cfg.extensions);
            if !effective_exts.contains(&ext) {
                return false;
            }
            return rel.components().count() <= dir_entry.depth;
        }
    }
    false
}

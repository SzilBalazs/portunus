use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

/// Set while a full-scan reindex (full or incremental) is running. Used to
/// coalesce overlapping reindex triggers so only one drives the progress bar.
static REINDEX_ACTIVE: AtomicBool = AtomicBool::new(false);

/// RAII guard for the single-reindex-at-a-time invariant. `acquire` returns
/// `None` if a reindex is already running; the flag clears on drop, so it is
/// released even on early return or panic.
pub struct ReindexGuard(());

impl ReindexGuard {
    pub fn acquire() -> Option<ReindexGuard> {
        REINDEX_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| ReindexGuard(()))
    }
}

impl Drop for ReindexGuard {
    fn drop(&mut self) {
        REINDEX_ACTIVE.store(false, Ordering::Release);
    }
}

// ── database ──────────────────────────────────────────────────────────────────

struct StoredMeta {
    mtime: i64,
    size: u64,
}

struct FileUpdate {
    path: String,
    text: String,
    /// Per-page text for PDFs (split on form-feed). `None` for non-paged formats.
    /// Populated into `pdf_page_fts` so previews can open on the matched page.
    pages: Option<Vec<String>>,
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

        // Schema version, bumped whenever the on-disk layout changes. A mismatch drops
        // every table and recreates them, which triggers a one-time full reindex.
        //   v2: added `pdf_page_fts` (per-page PDF text, for match-page preview).
        const SCHEMA_VERSION: i64 = 2;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if version != SCHEMA_VERSION {
            conn.execute_batch(
                "DROP TABLE IF EXISTS file_meta;
                 DROP TABLE IF EXISTS content_fts;
                 DROP TABLE IF EXISTS pdf_page_fts;",
            )?;
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
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS pdf_page_fts USING fts5(
                 path UNINDEXED,
                 page UNINDEXED,
                 text,
                 tokenize='porter unicode61'
             );",
        )?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;

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
            tx.execute("DELETE FROM pdf_page_fts WHERE path = ?", [u.path.as_str()])?;
            // Empty `text` is a negative-cache tombstone: a file that extracted to
            // nothing (e.g. an OCR'd screenshot with no detectable text) or errored.
            // We still write its file_meta below so the mtime+size skip catches it
            // next run instead of re-extracting it every startup — but we skip the
            // content_fts/pdf_page_fts inserts so it never surfaces in search.
            if !u.text.trim().is_empty() {
                tx.execute(
                    "INSERT INTO content_fts(path, text) VALUES (?, ?)",
                    params![u.path, u.text],
                )?;
                if let Some(pages) = &u.pages {
                    for (i, page) in pages.iter().enumerate() {
                        if page.trim().is_empty() {
                            continue;
                        }
                        tx.execute(
                            "INSERT INTO pdf_page_fts(path, page, text) VALUES (?, ?, ?)",
                            params![u.path, i as i64, page],
                        )?;
                    }
                }
            }
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
            tx.execute("DELETE FROM pdf_page_fts WHERE path = ?", [path.as_str()])?;
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
        db.execute("DELETE FROM pdf_page_fts WHERE path = ?", [path])?;
        db.execute("DELETE FROM file_meta WHERE path = ?", [path])?;
        Ok(())
    }

    /// Page index (0-based) of the best-matching page of a PDF for `fts_query`,
    /// or `None` if the file has no indexed pages or no page matched.
    ///
    /// Ranks pages by how many *distinct* query terms they cover (so a multi-part
    /// query lands on the page holding the most of them), and breaks ties toward the
    /// earliest page — rather than FTS BM25, which favours shorter pages and would
    /// e.g. pick a repeated header on the last page over the first.
    pub fn best_page(&self, path: &str, fts_query: &str) -> Option<u32> {
        let terms: Vec<String> = fts_query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        if terms.is_empty() {
            return None;
        }
        // Single OR query so this stays one FTS lookup per file (same cost as before):
        // it returns every page carrying *any* term; we count distinct coverage in Rust.
        // Phrase-quote each term so FTS treats it literally (apostrophes etc.); doubled
        // quotes escape an embedded quote.
        let or_query = terms
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");

        let db = util::lock(&self.db);
        let mut stmt = db
            .prepare(
                "SELECT page, text FROM pdf_page_fts WHERE path = ? AND text MATCH ? \
                 ORDER BY page",
            )
            .ok()?;
        let rows = stmt
            .query_map(params![path, or_query], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .ok()?;

        // Pages arrive in ascending order; keep the first that strictly beats the best
        // coverage so far — that yields max distinct terms, earliest page on ties.
        let mut best: Option<(u32, u32)> = None; // (coverage, page)
        for (page, text) in rows.flatten() {
            let lower = text.to_lowercase();
            let cov = terms
                .iter()
                .filter(|t| {
                    lower
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|w| w.starts_with(t.as_str()))
                })
                .count() as u32;
            if best.is_none_or(|(bc, _)| cov > bc) {
                best = Some((cov, page as u32));
            }
        }
        best.map(|(_, page)| page)
    }

    /// Removes all entries whose path starts with `dir_path/`. Used when a directory is
    /// deleted or moved out of the watched tree and no individual file events are fired.
    pub fn remove_prefix(&self, dir_path: &str) -> rusqlite::Result<usize> {
        let pattern = format!("{dir_path}/%");
        let mut db = util::lock(&self.db);
        let tx = db.transaction()?;
        let removed = tx.execute("DELETE FROM content_fts WHERE path LIKE ?", [&pattern])?;
        tx.execute("DELETE FROM pdf_page_fts WHERE path LIKE ?", [&pattern])?;
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
             DELETE FROM pdf_page_fts;
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
                            // Separate pages with a form-feed so they split the same
                            // way pdftotext output does (used for per-page indexing).
                            Ok(t) => {
                                combined.push_str(&t);
                                combined.push('\u{000C}');
                            }
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

/// Splits extracted PDF text into per-page strings on the form-feed separators
/// emitted by pdftotext / the OCR fallback. Returns `None` for non-PDF paths,
/// which keeps `pdf_page_fts` PDF-only.
fn pdf_pages(path: &str, text: &str) -> Option<Vec<String>> {
    let is_pdf = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);
    is_pdf.then(|| text.split('\u{000C}').map(|s| s.to_string()).collect())
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

/// A pre-index time/size estimate for a single directory. Counts are exact (a
/// cheap metadata-only walk); the seconds range is a coarse heuristic — see
/// `estimate_dir`. Serialized to the frontend for the per-directory estimate row.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DirEstimate {
    pub total_files: usize,
    pub pdf_files: usize,
    pub image_files: usize,
    pub est_secs_min: u64,
    pub est_secs_max: u64,
}

/// Per-file fixed overhead + per-megabyte rate, in seconds, for one work bucket.
/// Cost is modelled as `count * fixed + total_mb * per_mb` because the dominant
/// term differs by type: a subprocess spawn (PDF) or OCR init is per-file, while
/// read/tokenise/extract scales with bytes. Calibrated coarsely against real
/// runs — see `estimate_dir`.
struct Cost {
    fixed: f64,
    per_mb: f64,
}

impl Cost {
    fn secs(&self, count: usize, mb: f64) -> f64 {
        count as f64 * self.fixed + mb * self.per_mb
    }
}

/// Walks a single directory (respecting `depth`, `extensions`, and the size cap)
/// and produces file counts plus a min/max indexing-time estimate. No text is
/// extracted, so this is fast even on large trees.
///
/// The seconds are a size-weighted heuristic — they set expectations, not promises.
/// Each bucket's cost is `files * fixed_overhead + megabytes * per_mb_rate`, so a
/// folder of small screenshots and one of large scans estimate very differently
/// even at the same file count. PDFs dominate the uncertainty: with a text layer a
/// PDF is a quick `pdftotext`; a scanned one routed through OCR is far slower and
/// scales with page imagery. The range brackets those: the minimum assumes every
/// PDF has a text layer; the maximum assumes every PDF is OCR'd (only when
/// `ocr_pdf_fallback` is on, otherwise a no-text PDF just fails fast).
///
/// `extensions` overrides the global `cfg.extensions` when `Some` (per-dir list).
pub fn estimate_dir(
    path: &str,
    depth: usize,
    extensions: Option<&Vec<String>>,
    cfg: &ContentConfig,
) -> DirEstimate {
    let dir = crate::config::Config::expand_path(path);
    if !dir.is_dir() {
        return DirEstimate::default();
    }
    let effective_exts = extensions.unwrap_or(&cfg.extensions);

    // (count, total_bytes) per bucket.
    let (mut fast, mut pdf, mut image, mut other) = ((0usize, 0u64), (0usize, 0u64), (0usize, 0u64), (0usize, 0u64));
    for entry in walkdir::WalkDir::new(&dir)
        .max_depth(depth)
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
        let size = match entry.metadata() {
            Ok(m) if m.len() <= cfg.max_file_bytes => m.len(),
            _ => continue,
        };
        let bucket = if ext == "pdf" {
            &mut pdf
        } else if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            &mut image
        } else if TEXT_EXTENSIONS.contains(&ext.as_str()) || crate::office::is_office_ext(&ext) {
            &mut fast
        } else {
            &mut other
        };
        bucket.0 += 1;
        bucket.1 += size;
    }

    let mb = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);

    // Coarse, size-weighted per-bucket costs (seconds), recalibrated against real
    // indexing runs. Text reads are disk-bound and cheap; `pdftotext` pays a
    // subprocess spawn plus byte cost; OCR (image, or scanned-PDF fallback) is the
    // expensive outlier and scales with image data.
    const FAST: Cost = Cost { fixed: 0.002, per_mb: 0.01 };       // read + tokenise + FTS insert
    const PDF_TEXT: Cost = Cost { fixed: 0.04, per_mb: 0.03 };    // pdftotext subprocess
    const PDF_OCR: Cost = Cost { fixed: 0.20, per_mb: 1.50 };     // pdftoppm + tesseract per page
    const PDF_SKIP: Cost = Cost { fixed: 0.05, per_mb: 0.0 };     // no text layer, OCR off → give up
    const IMG_OCR_MIN: Cost = Cost { fixed: 0.20, per_mb: 0.30 };
    const IMG_OCR_MAX: Cost = Cost { fixed: 0.40, per_mb: 0.80 };
    const OTHER: Cost = Cost { fixed: 0.002, per_mb: 0.0 };       // listed but unsupported → quick fail

    let img_cost_min = if cfg.ocr_images { IMG_OCR_MIN.secs(image.0, mb(image.1)) } else { 0.0 };
    let img_cost_max = if cfg.ocr_images { IMG_OCR_MAX.secs(image.0, mb(image.1)) } else { 0.0 };
    let pdf_cost_max = if cfg.ocr_pdf_fallback {
        PDF_OCR.secs(pdf.0, mb(pdf.1))
    } else {
        PDF_SKIP.secs(pdf.0, mb(pdf.1))
    };

    let parallelism = if cfg.threads == 0 {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2)
    } else {
        cfg.threads
    }
    .max(1) as f64;

    let base = FAST.secs(fast.0, mb(fast.1)) + OTHER.secs(other.0, mb(other.1));
    let total_min = base + PDF_TEXT.secs(pdf.0, mb(pdf.1)) + img_cost_min;
    let total_max = base + pdf_cost_max + img_cost_max;

    DirEstimate {
        total_files: fast.0 + pdf.0 + image.0 + other.0,
        pdf_files: pdf.0,
        image_files: image.0,
        est_secs_min: (total_min / parallelism).ceil() as u64,
        est_secs_max: (total_max / parallelism).ceil() as u64,
    }
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

            // Empty text on a successful extract (e.g. an image that OCR'd to
            // nothing) or an extract error both yield an empty-text FileUpdate —
            // a negative-cache tombstone. upsert_batch writes its file_meta but no
            // FTS row, so the mtime+size fast-path skips it next run instead of
            // re-running the (expensive) extraction every startup.
            let text = match extract_text(&path_str, cfg) {
                Ok(t) if !t.trim().is_empty() => t,
                Ok(_) => String::new(),
                Err(e) => {
                    eprintln!("[content] extract failed {path_str}: {e}");
                    String::new()
                }
            };

            let n = processed.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 10 == 0 && n < total {
                if let Some(ref cb) = on_progress {
                    cb(n, total);
                }
            }

            let pages = pdf_pages(&path_str, &text);

            Some(FileUpdate {
                path: path_str,
                text,
                pages,
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
            let pages = pdf_pages(&path_str, &text);
            match index.upsert_batch(&[FileUpdate { path: path_str.clone(), text, pages, mtime, size }]) {
                Ok(()) => { eprintln!("[content] indexed {path_str}"); true }
                Err(e) => { eprintln!("[content] event upsert failed {path_str}: {e}"); false }
            }
        }
        // Empty text or an extract error: write an empty-text tombstone so the
        // mtime+size skip catches this file next time instead of re-extracting it.
        // upsert_batch clears any prior content_fts rows but writes no new one, so
        // it drops out of search just as remove_path would.
        res => {
            if let Err(e) = &res {
                eprintln!("[content] event extract failed {path_str}: {e}");
            }
            index
                .upsert_batch(&[FileUpdate {
                    path: path_str.clone(),
                    text: String::new(),
                    pages: None,
                    mtime,
                    size,
                }])
                .is_ok()
        }
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

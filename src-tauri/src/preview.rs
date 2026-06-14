use crate::config::SharedConfig;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::Manager;

/// Reply for a render job: (jpeg bytes, total page count). A oneshot the command
/// `await`s directly, so no blocking-pool thread is parked for the render's duration.
type RenderReply = tokio::sync::oneshot::Sender<Result<(Vec<u8>, u32), String>>;
/// Reply for a rects job: normalized [x, y, w, h] boxes (top-left origin, 0..1).
type RectsReply = tokio::sync::oneshot::Sender<Result<Vec<[f32; 4]>, String>>;

/// Work for the single pdfium-bound worker thread. pdfium is not `Send` and is
/// bound once per thread, so both rasterizing and text-layer queries share this
/// one queue rather than each spinning up a `Pdfium` (see `pdfium_available`).
enum PdfJob {
    /// (path, 0-based page index, target pixel width, reply).
    Render(String, u32, u32, RenderReply),
    /// (path, 0-based page index, search terms, reply).
    Rects(String, u32, Vec<String>, RectsReply),
}

pub struct PdfWorkerHandle {
    tx: std::sync::mpsc::SyncSender<PdfJob>,
}

/// Default render width for the side preview. Quicklook requests larger widths
/// (high-DPI / zoom) so A4 pages stay sharp and readable.
const PDF_RENDER_WIDTH: u32 = 800;
/// Hard ceiling so a runaway zoom can't allocate a multi-hundred-MB bitmap.
const PDF_MAX_RENDER_WIDTH: u32 = 4000;

/// Binds to pdfium, preferring the bundled library (AppImage) and falling back
/// to the system library for a source build.
fn bind_pdfium() -> Result<pdfium_render::prelude::Pdfium, String> {
    use pdfium_render::prelude::*;
    let bindings = match crate::runtime_assets::pdfium_library() {
        Some(path) => Pdfium::bind_to_library(&path)
            .or_else(|_| Pdfium::bind_to_system_library()),
        None => Pdfium::bind_to_system_library(),
    }
    .map_err(|e| e.to_string())?;
    Ok(Pdfium::new(bindings))
}

/// Whether pdfium can be loaded. Used by `check_dependencies` to report
/// PDF-preview availability in Settings without waiting for the user to
/// actually preview a PDF (where the failure would otherwise first surface).
///
/// This only *binds* the library (loads its symbols); it must NOT construct a
/// `Pdfium`. `Pdfium::new` calls the global `FPDF_InitLibrary` and its `Drop`
/// calls `FPDF_DestroyLibrary`, which would tear down the library state out
/// from under the long-lived preview worker and deadlock pdfium.
pub fn pdfium_available() -> bool {
    use pdfium_render::prelude::*;
    if let Some(path) = crate::runtime_assets::pdfium_library() {
        if Pdfium::bind_to_library(&path).is_ok() {
            return true;
        }
    }
    Pdfium::bind_to_system_library().is_ok()
}

fn start_pdf_worker(shared: SharedConfig) -> PdfWorkerHandle {
    let (tx, rx) = std::sync::mpsc::sync_channel::<PdfJob>(4);
    std::thread::spawn(move || {
        use image::codecs::jpeg::JpegEncoder;
        use pdfium_render::prelude::*;
        // Read the flag fresh each message so the Debug toggle takes effect live.
        let log_pdf = || shared.read().unwrap().log_pdf;
        let pdfium = bind_pdfium().map_err(|e| {
            eprintln!("[pdf] failed to bind pdfium: {e}");
            e
        });
        // Prime pdfium's font subsystem (fontconfig enumeration on Linux) by
        // rendering a tiny embedded PDF once. That work otherwise lands on the
        // user's first real preview, making it noticeably slower than the rest.
        // Failures are non-fatal: a missed warmup just restores the old behavior.
        if let Ok(pdfium) = &pdfium {
            const WARMUP_PDF: &[u8] = include_bytes!("warmup.pdf");
            let warmed = (|| -> Result<(), PdfiumError> {
                let doc = pdfium.load_pdf_from_byte_slice(WARMUP_PDF, None)?;
                doc.pages()
                    .get(0)?
                    .render_with_config(&PdfRenderConfig::new().set_target_width(800))?;
                Ok(())
            })();
            if log_pdf() {
                match warmed {
                    Ok(()) => eprintln!("[pdf] warmup done"),
                    Err(e) => eprintln!("[pdf] warmup failed: {e}"),
                }
            }
        }
        while let Ok(job) = rx.recv() {
            let log = log_pdf();
            match job {
                PdfJob::Render(path, page_idx, width, reply) => {
                    let result = match &pdfium {
                        Err(msg) => Err(msg.clone()),
                        Ok(pdfium) => (|| {
                            if log {
                                eprintln!("[pdf] rendering: {path} (page {page_idx}, width {width})");
                            }
                            let doc = pdfium.load_pdf_from_file(&path, None).map_err(|e| {
                                let msg = e.to_string();
                                if log {
                                    eprintln!("[pdf] load_pdf_from_file failed: {msg}");
                                }
                                msg
                            })?;
                            let page_count = doc.pages().len();
                            if log {
                                eprintln!("[pdf] loaded, {page_count} page(s)");
                            }
                            // Clamp the requested page into range (pdfium uses u16 indices).
                            let idx = page_idx.min(page_count.saturating_sub(1) as u32) as u16;
                            let page = doc.pages().get(idx).map_err(|e| {
                                let msg = e.to_string();
                                if log {
                                    eprintln!("[pdf] get page {idx} failed: {msg}");
                                }
                                msg
                            })?;
                            let total = page_count as u32;
                            let bitmap = page
                                .render_with_config(
                                    &PdfRenderConfig::new().set_target_width(width as i32),
                                )
                                .map_err(|e| {
                                    let msg = e.to_string();
                                    if log {
                                        eprintln!("[pdf] render failed: {msg}");
                                    }
                                    msg
                                })?;
                            // Quality 90 (vs the encoder default of 75): text edges stay
                            // crisp when the page is enlarged for reading in Quicklook.
                            // Drop to 82 for big zoomed renders: at that resolution the
                            // quality loss is invisible but it markedly cuts encode time
                            // and the bytes shipped across IPC, so zoom-in feels faster.
                            let quality = if width >= 2000 { 82 } else { 90 };
                            let mut bytes = Vec::new();
                            bitmap
                                .as_image()
                                .into_rgb8()
                                .write_with_encoder(JpegEncoder::new_with_quality(
                                    std::io::Cursor::new(&mut bytes),
                                    quality,
                                ))
                                .map_err(|e| {
                                    let msg = e.to_string();
                                    if log {
                                        eprintln!("[pdf] jpeg encode failed: {msg}");
                                    }
                                    msg
                                })?;
                            if log {
                                eprintln!("[pdf] done, {} bytes", bytes.len());
                            }
                            Ok((bytes, total))
                        })(),
                    };
                    let _ = reply.send(result);
                }
                PdfJob::Rects(path, page_idx, terms, reply) => {
                    let result = match &pdfium {
                        Err(msg) => Err(msg.clone()),
                        Ok(pdfium) => page_match_rects(pdfium, &path, page_idx, &terms, log),
                    };
                    let _ = reply.send(result);
                }
            }
        }
    });
    PdfWorkerHandle { tx }
}

pub fn setup(app: &tauri::AppHandle, shared: SharedConfig) {
    app.manage(start_pdf_worker(shared));
    app.manage(start_image_ocr_worker());
}

// ── image OCR highlight worker ──────────────────────────────────────────────────

type ImageRectsReply = tokio::sync::oneshot::Sender<Result<Vec<[f32; 4]>, String>>;

struct ImageOcrJob {
    path: String,
    needles: Vec<String>,
    lang: String,
    /// Request generation; the worker skips a job superseded by a newer one.
    generation: u64,
    reply: ImageRectsReply,
}

/// Single-thread OCR worker for on-demand image highlight boxes. Tesseract is far
/// heavier than the PDF text-layer path, so it gets its own thread (off tokio's
/// blocking pool and off the indexer) and coalesces requests: only the newest
/// pending generation actually OCRs, so arrow-keying through image results doesn't
/// queue an OCR per selection (paired with the frontend's debounce).
pub struct ImageOcrHandle {
    tx: std::sync::mpsc::SyncSender<ImageOcrJob>,
    generation: Arc<AtomicU64>,
}

fn start_image_ocr_worker() -> ImageOcrHandle {
    let (tx, rx) = std::sync::mpsc::sync_channel::<ImageOcrJob>(8);
    let generation = Arc::new(AtomicU64::new(0));
    let worker_gen = Arc::clone(&generation);
    std::thread::spawn(move || {
        while let Ok(job) = rx.recv() {
            // Superseded by a newer request: reply empty (the awaiting frontend has
            // already moved on / cancelled) without paying for the OCR.
            if job.generation < worker_gen.load(Ordering::Acquire) {
                let _ = job.reply.send(Ok(Vec::new()));
                continue;
            }
            let result = crate::content_index::ocr_image_boxes(&job.path, &job.lang)
                .map(|words| match_word_boxes(&words, &job.needles));
            let _ = job.reply.send(result);
        }
    });
    ImageOcrHandle { tx, generation }
}

/// Boxes of words whose content key matches a query key — the same
/// `porter unicode61` keying the index used (see `content_match`). Shared by the
/// cached and on-demand paths so they highlight identically. `keys` are query keys
/// from `normalize_terms`.
fn match_word_boxes(words: &[crate::content_index::WordBox], keys: &[String]) -> Vec<[f32; 4]> {
    // Stored OCR words are verbatim Tesseract tokens and can carry attached
    // punctuation ("report.", "(note)"), which would never stem - so tokenize each
    // and match any contained token. Set membership avoids an O(words*keys) scan.
    let set: std::collections::HashSet<&str> = keys.iter().map(String::as_str).collect();
    words
        .iter()
        .filter(|(w, _)| {
            crate::content_match::tokenize(w)
                .iter()
                .any(|(_, t)| set.contains(crate::content_match::match_key(t).as_str()))
        })
        .map(|(_, rect)| *rect)
        .collect()
}

/// Normalized highlight rectangles for `terms` over an OCR'd image preview. Returns
/// empty unless `content.ocr_highlight` is on. When `content.ocr_highlight_cache` is
/// on and the image is indexed, boxes come straight from the DB (no OCR); otherwise
/// the image is OCR'd on demand via the serialized worker.
#[tauri::command]
pub async fn image_match_rects(
    path: String,
    terms: Vec<String>,
    config: tauri::State<'_, crate::ConfigState>,
    content: tauri::State<'_, crate::ContentState>,
    ocr: tauri::State<'_, ImageOcrHandle>,
) -> Result<Vec<[f32; 4]>, String> {
    let (enabled, cache, lang) = {
        let cfg = crate::util::lock(&config);
        (
            cfg.content.ocr_highlight,
            cfg.content.ocr_highlight_cache,
            cfg.content.ocr_language.clone(),
        )
    };
    if !enabled {
        return Ok(Vec::new());
    }
    let needles = normalize_terms(terms);
    if needles.is_empty() {
        return Ok(Vec::new());
    }

    // Cached fast path: boxes captured at index time, no per-preview OCR.
    if cache {
        // Clone the Arc out and drop the lock before any await.
        let idx = content
            .lock()
            .map_err(|e| e.to_string())?
            .as_ref()
            .map(Arc::clone);
        if let Some(idx) = idx {
            let p = path.clone();
            let cached =
                tauri::async_runtime::spawn_blocking(move || idx.cached_word_boxes(&p))
                    .await
                    .map_err(|e| e.to_string())?;
            // Some(boxes) => indexed; trust the cache (even if empty). None => not
            // indexed yet, so fall through to on-demand OCR below.
            if let Some(words) = cached {
                return Ok(match_word_boxes(&words, &needles));
            }
        }
    }

    // On-demand OCR via the serialized worker (cache off, or a cache miss).
    let generation = ocr.generation.fetch_add(1, Ordering::AcqRel) + 1;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel::<Result<Vec<[f32; 4]>, String>>();
    let tx = ocr.tx.clone();
    let job = ImageOcrJob { path, needles, lang, generation, reply: reply_tx };
    tauri::async_runtime::spawn_blocking(move || tx.send(job).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())??;
    reply_rx.await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn render_pdf_page(
    path: String,
    page: Option<u32>,
    width: Option<u32>,
    worker: tauri::State<'_, PdfWorkerHandle>,
) -> Result<tauri::ipc::Response, String> {
    let width = width
        .unwrap_or(PDF_RENDER_WIDTH)
        .clamp(PDF_RENDER_WIDTH, PDF_MAX_RENDER_WIDTH);
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel::<Result<(Vec<u8>, u32), String>>();
    let tx = worker.tx.clone();
    // Enqueue on a blocking thread, then `await` the oneshot reply below. Blocking `send`
    // (not `try_send`): when the single pdfium worker is busy - a slow high-zoom render
    // plus the adjacent-page prefetches - the bounded queue fills, and `try_send` would
    // drop the job, leaving a stale page on screen after the counter advanced. Waiting for
    // a slot keeps them in sync. The enqueue is brief; the slow render is awaited via the
    // oneshot, so no blocking-pool thread is held for its duration (unlike `recv`).
    tauri::async_runtime::spawn_blocking(move || {
        tx.send(PdfJob::Render(path, page.unwrap_or(0), width, reply_tx))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let (bytes, count) = reply_rx.await.map_err(|e| e.to_string())??;
    // Raw bytes across IPC: a JSON number[] would ~5x the JPEG payload and dominate
    // render time at high zoom. Prepend the page count as a u32 LE header that the
    // frontend slices back off (see getPdfUrl).
    let mut buf = Vec::with_capacity(4 + bytes.len());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&bytes);
    Ok(tauri::ipc::Response::new(buf))
}

/// Returns normalized highlight rectangles for `terms` on one page of a PDF that
/// has a real text layer. Each rect is `[x, y, w, h]` in 0..1, top-left origin,
/// so the frontend can place boxes over the rendered page at any width/zoom.
/// Empty terms, no text layer, or no matches all yield an empty list.
#[tauri::command]
pub async fn pdf_match_rects(
    path: String,
    page: u32,
    terms: Vec<String>,
    worker: tauri::State<'_, PdfWorkerHandle>,
) -> Result<Vec<[f32; 4]>, String> {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel::<Result<Vec<[f32; 4]>, String>>();
    let tx = worker.tx.clone();
    // Blocking `send` for the same reason as render_pdf_page: don't drop the job when the
    // worker queue is momentarily full; wait for a slot. Reply awaited via the oneshot.
    tauri::async_runtime::spawn_blocking(move || {
        tx.send(PdfJob::Rects(path, page, terms, reply_tx))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    reply_rx.await.map_err(|e| e.to_string())?
}

/// Lowercases search terms, drops 1-char noise, and deduplicates, mirroring the
/// frontend's `deriveContentTerms` and the content provider's tokenization, so
/// preview matching lands on the same terms the index matched. Dedup matters for
/// repeated-word queries (e.g. "the on the the on the"): without it each duplicate
/// needle re-scans the page and stamps the same boxes again.
fn normalize_terms<I: IntoIterator<Item = String>>(terms: I) -> Vec<String> {
    // Drop 1-char noise, then key each term the way the content index tokenized it
    // (`porter unicode61`) so highlight / box / section matching agrees with FTS.
    // `query_keys` folds, stems, dedups, and drops empties.
    crate::content_match::query_keys(terms.into_iter().filter(|t| t.chars().count() >= 2))
}

/// Largest vertical gap (in points) between consecutive chars of one match before
/// it is split into separate boxes - so a line-wrapped match yields one box per
/// line instead of a single bar spanning the gap.
const LINE_BREAK_POINTS: f32 = 2.0;

/// Locates `terms` in the page's text layer (word-prefix, case-insensitive, to
/// mirror the frontend's `buildTermRegex`) and returns normalized boxes. Runs on
/// the pdfium worker thread; `pdfium` is the thread's bound instance.
fn page_match_rects(
    pdfium: &pdfium_render::prelude::Pdfium,
    path: &str,
    page_idx: u32,
    terms: &[String],
    log: bool,
) -> Result<Vec<[f32; 4]>, String> {
    use pdfium_render::prelude::*;

    let needles = normalize_terms(terms.iter().cloned());
    if needles.is_empty() {
        return Ok(Vec::new());
    }

    let doc = pdfium.load_pdf_from_file(path, None).map_err(|e| {
        let msg = e.to_string();
        if log {
            eprintln!("[pdf] rects: load failed: {msg}");
        }
        msg
    })?;
    let page_count = doc.pages().len();
    let idx = page_idx.min(page_count.saturating_sub(1) as u32) as u16;
    let page = doc.pages().get(idx).map_err(|e| e.to_string())?;
    let page_w = page.width().value;
    let page_h = page.height().value;
    if page_w <= 0.0 || page_h <= 0.0 {
        return Ok(Vec::new());
    }

    let text = match page.text() {
        Ok(t) => t,
        Err(e) => {
            if log {
                eprintln!("[pdf] rects: no text layer: {e}");
            }
            return Ok(Vec::new());
        }
    };

    // Char string + parallel bounds. Skip chars without a value or bounds so the
    // two arrays stay aligned with `lower`.
    let mut lower = String::new();
    let mut bounds: Vec<PdfRect> = Vec::new();
    for ch in text.chars().iter() {
        let (Some(c), Ok(b)) = (ch.unicode_char(), ch.loose_bounds()) else {
            continue;
        };
        for lc in c.to_lowercase() {
            lower.push(lc);
            bounds.push(b);
        }
    }
    if lower.is_empty() {
        return Ok(Vec::new());
    }
    let chars: Vec<char> = lower.chars().collect();

    // Hard cap on boxes per page: a stopword query ("the") matches nearly every
    // word, and rendering hundreds of overlay divs per keystroke is the dominant
    // highlight cost. Beyond this many the highlight is visual noise anyway, so we
    // stop scanning - and log it rather than silently truncating.
    const MAX_RECTS: usize = 400;

    // Walk whole words; a word is highlighted when its content key matches a query
    // key - the same `porter unicode61` keying the index used. `chars`/`bounds` are
    // index-aligned, so a word's char span [start, end) maps straight to its bounds.
    // Diacritic chars continue the word so pdfium's split accents (`caf´e` =
    // ...U+00B4, e) aren't broken at the accent; match_key strips them when keying.
    let is_word = |c: char| c.is_alphanumeric() || crate::content_match::is_diacritic(c);
    // Membership set so each word is one hash lookup, not an O(needles) scan.
    let key_set: std::collections::HashSet<&str> = needles.iter().map(String::as_str).collect();
    let mut rects: Vec<[f32; 4]> = Vec::new();
    let mut capped = false;
    let mut i = 0;
    while i < chars.len() {
        if !is_word(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        if key_set.contains(crate::content_match::match_key(&word).as_str()) {
            if rects.len() >= MAX_RECTS {
                capped = true;
                break;
            }
            push_rects(&bounds[start..i], page_w, page_h, &mut rects);
        }
    }
    if log {
        eprintln!(
            "[pdf] rects: {} box(es) on page {idx}{}",
            rects.len(),
            if capped { " (capped)" } else { "" }
        );
    }
    Ok(rects)
}

/// Unions the char bounds of one match into boxes, splitting on vertical line
/// jumps, and pushes each as a normalized `[x, y, w, h]` (top-left origin).
fn push_rects(
    chars: &[pdfium_render::prelude::PdfRect],
    page_w: f32,
    page_h: f32,
    out: &mut Vec<[f32; 4]>,
) {
    let mut run_start = 0;
    for i in 0..chars.len() {
        let split = i > run_start
            && (chars[i].top().value - chars[run_start].top().value).abs() > LINE_BREAK_POINTS;
        if split {
            out.push(union_rect(&chars[run_start..i], page_w, page_h));
            run_start = i;
        }
    }
    if run_start < chars.len() {
        out.push(union_rect(&chars[run_start..], page_w, page_h));
    }
}

/// Bounding box of `chars` (pdfium points, bottom-left origin) as a normalized
/// top-left-origin `[x, y, w, h]`.
fn union_rect(
    chars: &[pdfium_render::prelude::PdfRect],
    page_w: f32,
    page_h: f32,
) -> [f32; 4] {
    let mut left = f32::MAX;
    let mut right = f32::MIN;
    let mut top = f32::MIN;
    let mut bottom = f32::MAX;
    for r in chars {
        left = left.min(r.left().value);
        right = right.max(r.right().value);
        top = top.max(r.top().value);
        bottom = bottom.min(r.bottom().value);
    }
    [
        (left / page_w).clamp(0.0, 1.0),
        ((page_h - top) / page_h).clamp(0.0, 1.0),
        ((right - left) / page_w).clamp(0.0, 1.0),
        ((top - bottom) / page_h).clamp(0.0, 1.0),
    ]
}

#[tauri::command]
pub fn read_office_preview(path: String) -> Result<String, String> {
    const MAX_LINES: usize = 300;
    const MAX_BYTES: usize = 32 * 2048;
    let text = crate::office::extract_office_markdown(&path)?;
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        if i >= MAX_LINES || out.len() + line.len() + 1 > MAX_BYTES {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

#[tauri::command]
pub fn read_spreadsheet_preview(path: String) -> Result<Vec<Vec<String>>, String> {
    crate::office::extract_spreadsheet_grid(&path)
}

const PREVIEW_MAX_LINES: usize = 300;
const PREVIEW_MAX_BYTES: usize = 32 * 2048;

/// Bit `i` is set when a word on the line keys to query key `i` (`key_idx` maps
/// query key -> bit, capped at 64). Keying mirrors the index (`porter unicode61`),
/// so `running` matches a `run` query but `category` does not match `cat`. `memo`
/// caches word -> bit across lines so repeated words aren't re-stemmed.
fn line_term_mask(
    line: &str,
    key_idx: &std::collections::HashMap<&str, usize>,
    memo: &mut std::collections::HashMap<String, Option<usize>>,
) -> u64 {
    let mut mask = 0u64;
    for (_, w) in crate::content_match::tokenize(line) {
        let bit = match memo.get(w) {
            Some(&cached) => cached,
            None => {
                let key = crate::content_match::match_key(w);
                let found = key_idx.get(key.as_str()).copied();
                memo.insert(w.to_string(), found);
                found
            }
        };
        if let Some(i) = bit {
            mask |= 1 << i;
        }
    }
    mask
}

/// Joins `lines[start..]` into a preview, bounded by line and byte caps.
fn clip_lines(lines: &[String], start: usize) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut total = 0usize;
    for line in lines.iter().skip(start).take(PREVIEW_MAX_LINES) {
        total += line.len() + 1;
        if total > PREVIEW_MAX_BYTES {
            break;
        }
        out.push(line.as_str());
    }
    out.join("\n")
}

#[tauri::command]
pub fn read_text_preview(path: String, terms: Option<Vec<String>>) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    // Upper bound on how far we scan for a match before giving up (content-indexed
    // files are size-capped by config, so this is a safety net for pathological files).
    const SCAN_LINES: usize = 50_000;

    let terms = normalize_terms(terms.unwrap_or_default());

    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);

    // No terms: keep the cheap streaming path - first window from the top.
    if terms.is_empty() {
        let mut lines: Vec<String> = Vec::new();
        let mut total = 0usize;
        for line in reader.lines().take(PREVIEW_MAX_LINES) {
            let line = line.map_err(|e| e.to_string())?;
            total += line.len() + 1;
            if total > PREVIEW_MAX_BYTES {
                break;
            }
            lines.push(line);
        }
        return Ok(lines.join("\n"));
    }

    // Terms present: read up to SCAN_LINES, then center the window on the earliest
    // section that covers the most *distinct* query terms - so a multi-part query
    // lands on the section holding all of them, not the first stray single match.
    const CLUSTER_LINES: usize = 10;
    let mut lines: Vec<String> = Vec::new();
    for line in reader.lines().take(SCAN_LINES) {
        lines.push(line.map_err(|e| e.to_string())?);
    }

    // Map each query key to its bit (cap 64), then mask each line; `memo` avoids
    // re-stemming repeated words across the scan.
    let key_idx: std::collections::HashMap<&str, usize> = terms
        .iter()
        .take(64)
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();
    let mut memo: std::collections::HashMap<String, Option<usize>> =
        std::collections::HashMap::new();
    let masks: Vec<u64> = lines
        .iter()
        .map(|l| line_term_mask(l, &key_idx, &mut memo))
        .collect();

    // Slide a CLUSTER_LINES window; track distinct terms covered. `counts[i]` is how
    // many lines in the window carry term `i`; `distinct` is how many terms have a
    // nonzero count. Strict `>` keeps the earliest window on ties.
    let mut counts = [0u16; 64];
    let mut distinct = 0i32;
    let mut best_distinct = 0i32;
    let mut best_start = 0usize;
    for end in 0..masks.len() {
        let mut m = masks[end];
        while m != 0 {
            let i = m.trailing_zeros() as usize;
            if counts[i] == 0 {
                distinct += 1;
            }
            counts[i] += 1;
            m &= m - 1;
        }
        if end >= CLUSTER_LINES {
            let mut out = masks[end - CLUSTER_LINES];
            while out != 0 {
                let i = out.trailing_zeros() as usize;
                counts[i] -= 1;
                if counts[i] == 0 {
                    distinct -= 1;
                }
                out &= out - 1;
            }
        }
        if distinct > best_distinct {
            best_distinct = distinct;
            best_start = end.saturating_sub(CLUSTER_LINES - 1);
        }
    }

    // No term found anywhere in the scan: fall back to the top of the file.
    let start = if best_distinct == 0 {
        0
    } else {
        (best_start + CLUSTER_LINES / 2).saturating_sub(PREVIEW_MAX_LINES / 2)
    };
    Ok(clip_lines(&lines, start))
}

#[tauri::command]
pub async fn render_image_preview(
    path: String,
    width: Option<u32>,
) -> Result<tauri::ipc::Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        use image::ImageFormat;
        // Default 800 for the side preview; Quicklook requests a larger width so an
        // enlarged image stays crisp. Clamped to keep memory bounded.
        let max_width = width.unwrap_or(800).clamp(200, 2400);
        let img = image::open(&path).map_err(|e| e.to_string())?;
        let img = if img.width() > max_width {
            img.thumbnail(max_width, u32::MAX)
        } else {
            img
        };
        let mut bytes = Vec::new();
        img.into_rgb8()
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        Ok::<Vec<u8>, String>(bytes)
    })
    .await
    .map_err(|e| e.to_string())??;
    // Raw bytes across IPC, not a JSON number[] (see render_pdf_page).
    Ok(tauri::ipc::Response::new(bytes))
}

#[derive(serde::Serialize)]
pub struct FolderEntry {
    name: String,
    is_dir: bool,
    size: Option<u64>,
}

#[tauri::command]
pub fn list_folder(path: String) -> Vec<FolderEntry> {
    const MAX: usize = 200;
    let mut entries: Vec<FolderEntry> = std::fs::read_dir(&path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .take(MAX)
        .map(|e| {
            let meta = e.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            FolderEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
                size: meta.and_then(|m| if m.is_file() { Some(m.len()) } else { None }),
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

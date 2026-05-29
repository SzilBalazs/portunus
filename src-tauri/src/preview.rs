use crate::config::SharedConfig;
use tauri::Manager;

/// (path, 0-based page index, reply channel). Reply is (jpeg bytes, total page count).
type PdfReply = std::sync::mpsc::Sender<Result<(Vec<u8>, u32), String>>;
type PdfRenderMsg = (String, u32, PdfReply);

pub struct PdfWorkerHandle {
    tx: std::sync::mpsc::SyncSender<PdfRenderMsg>,
}

const PDF_RENDER_WIDTH: u32 = 800;

/// Whether the pdfium system library can be loaded. Used by `check_dependencies`
/// to report PDF-preview availability in Settings without waiting for the user
/// to actually preview a PDF (where the failure would otherwise first surface).
pub fn pdfium_available() -> bool {
    use pdfium_render::prelude::*;
    Pdfium::bind_to_system_library().is_ok()
}

fn start_pdf_worker(shared: SharedConfig) -> PdfWorkerHandle {
    let (tx, rx) = std::sync::mpsc::sync_channel::<PdfRenderMsg>(4);
    std::thread::spawn(move || {
        use image::ImageFormat;
        use pdfium_render::prelude::*;
        // Read the flag fresh each message so the Debug toggle takes effect live.
        let log_pdf = || shared.read().unwrap().log_pdf;
        let pdfium = Pdfium::bind_to_system_library()
            .map(Pdfium::new)
            .map_err(|e| {
                eprintln!("[pdf] bind_to_system_library failed: {e}");
                e.to_string()
            });
        while let Ok((path, page_idx, reply)) = rx.recv() {
            let log = log_pdf();
            let result = match &pdfium {
                Err(msg) => Err(msg.clone()),
                Ok(pdfium) => (|| {
                    if log {
                        eprintln!("[pdf] rendering: {path} (page {page_idx})");
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
                            &PdfRenderConfig::new().set_target_width(PDF_RENDER_WIDTH as i32),
                        )
                        .map_err(|e| {
                            let msg = e.to_string();
                            if log {
                                eprintln!("[pdf] render failed: {msg}");
                            }
                            msg
                        })?;
                    let mut bytes = Vec::new();
                    bitmap
                        .as_image()
                        .into_rgb8()
                        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)
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
    });
    PdfWorkerHandle { tx }
}

pub fn setup(app: &tauri::AppHandle, shared: SharedConfig) {
    app.manage(start_pdf_worker(shared));
}

#[tauri::command]
pub async fn render_pdf_page(
    path: String,
    page: Option<u32>,
    worker: tauri::State<'_, PdfWorkerHandle>,
) -> Result<(Vec<u8>, u32), String> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Result<(Vec<u8>, u32), String>>();
    worker
        .tx
        .try_send((path, page.unwrap_or(0), reply_tx))
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        reply_rx.recv().unwrap_or_else(|e| Err(e.to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
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

/// Word-prefix, case-insensitive match of any term against a line. Mirrors the
/// porter-stemmer-ish matching used for highlighting (`run` matches `running`).
fn line_matches_terms(line: &str, terms: &[String]) -> bool {
    let lower = line.to_lowercase();
    terms.iter().any(|t| {
        lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w.starts_with(t.as_str()))
    })
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

    let terms: Vec<String> = terms
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 2)
        .collect();

    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);

    // No terms: keep the cheap streaming path — first window from the top.
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

    // Terms present: read up to SCAN_LINES, center the window on the first match
    // so the relevant section is visible even when it's deep in the file.
    let mut lines: Vec<String> = Vec::new();
    for line in reader.lines().take(SCAN_LINES) {
        lines.push(line.map_err(|e| e.to_string())?);
    }
    let start = lines
        .iter()
        .position(|l| line_matches_terms(l, &terms))
        .map(|i| i.saturating_sub(PREVIEW_MAX_LINES / 2))
        .unwrap_or(0);
    Ok(clip_lines(&lines, start))
}

#[tauri::command]
pub async fn render_image_preview(path: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use image::ImageFormat;
        const MAX_WIDTH: u32 = 800;
        let img = image::open(&path).map_err(|e| e.to_string())?;
        let img = if img.width() > MAX_WIDTH {
            img.thumbnail(MAX_WIDTH, u32::MAX)
        } else {
            img
        };
        let mut bytes = Vec::new();
        img.into_rgb8()
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    })
    .await
    .map_err(|e| e.to_string())?
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

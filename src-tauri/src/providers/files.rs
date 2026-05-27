use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use tauri::Manager;

use super::{Provider, SearchResult};
use crate::config::{FilesConfig, SharedConfig};

// ── PDF worker ────────────────────────────────────────────────────────────────

type PdfRenderMsg = (String, std::sync::mpsc::Sender<Result<Vec<u8>, String>>);

pub struct PdfWorkerHandle {
    tx: std::sync::mpsc::SyncSender<PdfRenderMsg>,
}

const PDF_RENDER_WIDTH: u32 = 800;

fn start_pdf_worker() -> PdfWorkerHandle {
    let (tx, rx) = std::sync::mpsc::sync_channel::<PdfRenderMsg>(4);
    std::thread::spawn(move || {
        use image::ImageFormat;
        use pdfium_render::prelude::*;
        let pdfium = Pdfium::bind_to_system_library()
            .map(Pdfium::new)
            .map_err(|e| {
                eprintln!("[pdf] bind_to_system_library failed: {e}");
                e.to_string()
            });
        while let Ok((path, reply)) = rx.recv() {
            let result = match &pdfium {
                Err(msg) => Err(msg.clone()),
                Ok(pdfium) => (|| {
                    eprintln!("[pdf] rendering: {path}");
                    let doc = pdfium.load_pdf_from_file(&path, None).map_err(|e| {
                        let msg = e.to_string();
                        eprintln!("[pdf] load_pdf_from_file failed: {msg}");
                        msg
                    })?;
                    eprintln!("[pdf] loaded, {} page(s)", doc.pages().len());
                    let page = doc.pages().get(0).map_err(|e| {
                        let msg = e.to_string();
                        eprintln!("[pdf] get page 0 failed: {msg}");
                        msg
                    })?;
                    let bitmap = page
                        .render_with_config(
                            &PdfRenderConfig::new().set_target_width(PDF_RENDER_WIDTH as i32),
                        )
                        .map_err(|e| {
                            let msg = e.to_string();
                            eprintln!("[pdf] render failed: {msg}");
                            msg
                        })?;
                    let mut bytes = Vec::new();
                    bitmap
                        .as_image()
                        .into_rgb8()
                        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)
                        .map_err(|e| {
                            let msg = e.to_string();
                            eprintln!("[pdf] jpeg encode failed: {msg}");
                            msg
                        })?;
                    eprintln!("[pdf] done, {} bytes", bytes.len());
                    Ok(bytes)
                })(),
            };
            let _ = reply.send(result);
        }
    });
    PdfWorkerHandle { tx }
}

pub fn setup(app: &tauri::AppHandle) {
    app.manage(start_pdf_worker());
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn render_pdf_page(
    path: String,
    worker: tauri::State<'_, PdfWorkerHandle>,
) -> Result<Vec<u8>, String> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Result<Vec<u8>, String>>();
    worker
        .tx
        .try_send((path, reply_tx))
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        reply_rx.recv().unwrap_or_else(|e| Err(e.to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn read_text_preview(path: String) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    const MAX_LINES: usize = 300;
    const MAX_BYTES: usize = 32 * 2048;
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = Vec::new();
    let mut total = 0usize;
    for line in BufReader::new(file).lines().take(MAX_LINES) {
        let line = line.map_err(|e| e.to_string())?;
        total += line.len() + 1;
        if total > MAX_BYTES {
            break;
        }
        lines.push(line);
    }
    Ok(lines.join("\n"))
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

// ── Search provider ───────────────────────────────────────────────────────────

struct FileEntry {
    path: String,
    name: String,
    parent: String,
    is_dir: bool,
    file_size: Option<u64>,
    created: Option<u64>,
    modified: Option<u64>,
}

pub struct FileProvider {
    entries: Vec<FileEntry>,
    shared: SharedConfig,
}

impl FileProvider {
    pub fn new(files_cfg: &FilesConfig, shared: SharedConfig) -> Self {
        let roots: Vec<(PathBuf, usize)> = files_cfg
            .dirs
            .iter()
            .map(|d| (crate::config::Config::expand_path(&d.path), d.depth))
            .collect();

        let mut entries = Vec::new();
        for (dir, depth) in &roots {
            if !dir.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(dir)
                .max_depth(*depth)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.depth() == 0 {
                    continue;
                }
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned)
                else {
                    continue;
                };
                let parent = path
                    .parent()
                    .and_then(|p| p.to_str())
                    .unwrap_or("")
                    .to_owned();
                let is_dir = entry.file_type().is_dir();
                let (file_size, created, modified) = match entry.metadata() {
                    Ok(meta) => {
                        let size = if is_dir { None } else { Some(meta.len()) };
                        let cr = meta
                            .created()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs());
                        let mo = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs());
                        (size, cr, mo)
                    }
                    Err(_) => (None, None, None),
                };
                entries.push(FileEntry {
                    path: path.to_string_lossy().into_owned(),
                    name,
                    parent,
                    is_dir,
                    file_size,
                    created,
                    modified,
                });
            }
        }
        Self { entries, shared }
    }
}

impl Provider for FileProvider {
    fn id(&self) -> &'static str {
        "files"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim();
        if q.is_empty() || q.starts_with('!') {
            return vec![];
        }

        let cfg = self.shared.read().unwrap();
        let min_score = cfg.min_score_file;
        let recency_weight = cfg.recency_weight;
        let log_scores = cfg.log_scores;
        drop(cfg);

        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut char_buf = Vec::new();

        self.entries
            .iter()
            .filter_map(|entry| {
                let score =
                    pattern.score(Utf32Str::new(&entry.name, &mut char_buf), &mut matcher)?;
                let threshold = super::effective_min_score(min_score, query.chars().count());
                if log_scores {
                    eprintln!("[files] {:?} → {:?}  score={} threshold={}", query, entry.name, score, threshold);
                }
                if score < threshold {
                    return None;
                }
                let base = if entry.is_dir {
                    super::SCORE_FOLDER
                } else {
                    super::SCORE_FILE
                };
                let recency =
                    super::recency_bonus(entry.created, entry.modified, recency_weight);
                let escaped = entry.path.replace('"', "\\\"");
                Some(SearchResult {
                    id: format!("file:{}", entry.path),
                    title: entry.name.clone(),
                    subtitle: Some(entry.parent.clone()),
                    kind: if entry.is_dir { "folder" } else { "file" }.to_string(),
                    score: base + score as f32 + recency,
                    exec: Some(format!("xdg-open \"{}\"", escaped)),
                    icon_path: None,
                    file_size: entry.file_size,
                    created: entry.created,
                    snippet: None,
                modified: entry.modified,
                })
            })
            .collect()
    }
}

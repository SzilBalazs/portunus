use tauri::Manager;

type PdfRenderMsg = (String, std::sync::mpsc::Sender<Result<Vec<u8>, String>>);

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
pub fn read_office_preview(path: String) -> Result<String, String> {
    const MAX_LINES: usize = 300;
    const MAX_BYTES: usize = 32 * 2048;
    let text = crate::office::extract_office_text(&path)?;
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

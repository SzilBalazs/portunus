use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tauri::{Emitter, Manager};

use super::{CommandDescriptor, Provider, SearchResult};
use crate::{ClipboardOcrState, ConfigState};

/// How long to wait after hiding the launcher before synthesizing Ctrl+V. On a
/// layer-shell exclusive-keyboard surface the compositor needs a frame or two to
/// return focus to the previously focused toplevel once our surface unmaps.
const PASTE_FOCUS_DELAY_MS: u64 = 150;

// ── capabilities ────────────────────────────────────────────────────────────

/// Whether `wtype` is on PATH. Cached: PATH doesn't change within a session.
fn wtype_available() -> bool {
    static AVAIL: OnceLock<bool> = OnceLock::new();
    *AVAIL.get_or_init(|| crate::util::binary_in_path("wtype"))
}

#[derive(serde::Serialize, Clone)]
pub struct ClipboardCapabilities {
    /// Enter pastes into the focused window (wtype present on a Wayland session)
    /// rather than only copying to the clipboard.
    pub smart_paste: bool,
}

#[tauri::command]
pub fn clipboard_capabilities() -> ClipboardCapabilities {
    ClipboardCapabilities {
        smart_paste: wtype_available() && std::env::var("WAYLAND_DISPLAY").is_ok(),
    }
}

// ── paste / decode / delete ───────────────────────────────────────────────────

/// Sniff the leading bytes for a known raster image format so wl-copy can offer
/// the right MIME type (some targets paste nothing for a generic blob).
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes[0] == 0x89 && bytes[1] == 0x50 {
        return Some("image/png");
    }
    if bytes[0] == 0xff && bytes[1] == 0xd8 {
        return Some("image/jpeg");
    }
    if &bytes[0..4] == b"GIF8" {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes[0] == 0x42 && bytes[1] == 0x4D {
        return Some("image/bmp");
    }
    None
}

#[tauri::command]
pub fn paste_clipboard(
    app: tauri::AppHandle,
    id: String,
    copy_only: Option<bool>,
    config: tauri::State<ConfigState>,
) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let paste_mode = config
        .lock()
        .map(|c| c.clipboard.paste_mode.clone())
        .unwrap_or_else(|_| "auto".to_string());

    let decoded = match Command::new("cliphist").args(["decode", &id]).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }
            return;
        }
    };

    // Tag images with their MIME so apps that inspect the offered type accept them.
    let mut cmd = Command::new("wl-copy");
    if let Some(mime) = image_mime(&decoded) {
        cmd.args(["--type", mime]);
    }
    if let Ok(mut child) = cmd.stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&decoded);
        }
        // Wait so wl-copy has taken ownership of the selection before we hide.
        let _ = child.wait();
    }

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }

    // Smart paste: after the launcher hides and focus returns to the prior window,
    // synthesize Ctrl+V. Best-effort - on compositors without the virtual-keyboard
    // protocol (e.g. GNOME) wtype exits non-zero and the user still has the copy.
    let smart = copy_only != Some(true)
        && paste_mode == "auto"
        && wtype_available()
        && std::env::var("WAYLAND_DISPLAY").is_ok();
    if smart {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(PASTE_FOCUS_DELAY_MS));
            let _ = Command::new("wtype")
                .args(["-M", "ctrl", "-k", "v", "-m", "ctrl"])
                .status();
        });
    }
}

#[tauri::command]
pub fn decode_clipboard_entry(id: String) -> Result<tauri::ipc::Response, String> {
    let out = std::process::Command::new("cliphist")
        .args(["decode", &id])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        // Raw ArrayBuffer across IPC - a JSON number array would ~5x a screenshot.
        Ok(tauri::ipc::Response::new(out.stdout))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

#[tauri::command]
pub fn clipboard_delete(id: String) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    // `cliphist delete` reads stdin, cuts at the first tab, and parses the prefix
    // as the entry id; everything after the tab is ignored, so a bare "<id>\t"
    // suffices - no need to reproduce the original content bytes.
    let mut child = Command::new("cliphist")
        .arg("delete")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{id}\t").as_bytes());
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("cliphist delete failed".to_string())
    }
}

// ── listing + classification ──────────────────────────────────────────────────

/// A clipboard entry enriched for the clipboard browser. `preview` is the
/// (possibly truncated) cliphist list snippet; authoritative metadata for text
/// entries comes from decoding the full content on the frontend.
#[derive(serde::Serialize, Clone)]
pub struct ClipboardEntry {
    pub id: String,
    /// "text" | "image"
    pub kind: String,
    pub preview: String,
    /// "text" | "url" | "color" | "json" | "image"
    pub content_type: String,
    /// Normalized CSS color string when `content_type == "color"`.
    pub color: Option<String>,
    /// Image entries only: decoded byte size parsed from the cliphist label.
    pub byte_size: Option<u64>,
    /// Image entries only: pixel dimensions parsed from the label.
    pub dimensions: Option<(u32, u32)>,
    /// Image entries only: lowercase format token (e.g. "png").
    pub format: Option<String>,
    /// Image entries only: cached OCR'd text (visible text in the image), used
    /// for search. `None` until the background OCR pass has indexed the entry.
    pub ocr_text: Option<String>,
}

struct ClipEntry {
    id: String,
    kind: ClipKind,
}

enum ClipKind {
    Text(String),
    Image { label: String },
}

fn list_entries() -> Vec<ClipEntry> {
    let output = match std::process::Command::new("cliphist").arg("list").output() {
        Ok(o) => o.stdout,
        Err(_) => return vec![],
    };
    String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|line| {
            let tab = line.find('\t')?;
            let id = line[..tab].to_string();
            id.parse::<u64>().ok()?; // skip garbage lines
            let content = line[tab + 1..].to_string();
            let kind = if content.starts_with("[[ binary data ") {
                // "[[ binary data 83 KiB png 1068x904 ]]"
                let inner = content
                    .trim_start_matches("[[ binary data ")
                    .trim_end_matches(" ]]");
                ClipKind::Image { label: inner.to_string() }
            } else {
                ClipKind::Text(content)
            };
            Some(ClipEntry { id, kind })
        })
        .collect()
}

/// Classify a text clipboard snippet for the list view. Runs on the (possibly
/// ~100-char-truncated) cliphist snippet, so it relies on signals that survive
/// truncation: colors and urls are short and never truncated; json is a heuristic
/// the frontend revalidates against the full decoded content.
fn detect_content_type(snippet: &str) -> (String, Option<String>) {
    let s = snippet.trim();
    if let Some(color) = parse_color(s) {
        return ("color".to_string(), Some(color));
    }
    if is_url(s) {
        return ("url".to_string(), None);
    }
    if looks_like_json(s) {
        return ("json".to_string(), None);
    }
    ("text".to_string(), None)
}

fn parse_color(s: &str) -> Option<String> {
    if let Some(hex) = s.strip_prefix('#') {
        let len = hex.len();
        if (len == 3 || len == 6 || len == 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(s.to_string());
        }
    }
    let lower = s.to_lowercase();
    if (lower.starts_with("rgb(") || lower.starts_with("rgba(")) && lower.ends_with(')') {
        let open = lower.find('(').unwrap();
        let body = &lower[open + 1..lower.len() - 1];
        if !body.is_empty()
            && body
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ' ' | '%'))
        {
            return Some(s.to_string());
        }
    }
    None
}

fn is_url(s: &str) -> bool {
    (s.starts_with("http://") || s.starts_with("https://"))
        && s.len() > 8
        && !s.chars().any(char::is_whitespace)
}

fn looks_like_json(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("{\"")
        || t.starts_with("[{")
        || t.starts_with("[[")
        || (t.starts_with('{') && t[1..].trim_start().starts_with('"'))
}

/// Parse a cliphist binary-data label like "83 KiB png 1068x904" into
/// (byte_size, format, dimensions). Tolerant of token order and missing pieces.
fn parse_image_label(label: &str) -> (Option<u64>, Option<String>, Option<(u32, u32)>) {
    let tokens: Vec<&str> = label.split_whitespace().collect();
    let mut byte_size = None;
    let mut format = None;
    let mut dimensions = None;
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        if dimensions.is_none() {
            if let Some((w, h)) = tok.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    dimensions = Some((w, h));
                    i += 1;
                    continue;
                }
            }
        }
        if byte_size.is_none() {
            if let Ok(num) = tok.parse::<f64>() {
                let unit = tokens.get(i + 1).copied().unwrap_or("");
                let mult = match unit {
                    "B" => Some(1.0),
                    "KiB" => Some(1024.0),
                    "MiB" => Some(1024.0 * 1024.0),
                    "GiB" => Some(1024.0 * 1024.0 * 1024.0),
                    _ => None,
                };
                if let Some(m) = mult {
                    byte_size = Some((num * m) as u64);
                    i += 2;
                    continue;
                }
            }
        }
        if format.is_none()
            && tok.chars().all(|c| c.is_ascii_alphabetic())
            && !matches!(tok, "B" | "KiB" | "MiB" | "GiB")
        {
            format = Some(tok.to_lowercase());
        }
        i += 1;
    }
    (byte_size, format, dimensions)
}

#[tauri::command]
pub fn clipboard_list(
    limit: usize,
    ocr_store: tauri::State<ClipboardOcrState>,
) -> Result<Vec<ClipboardEntry>, String> {
    if !crate::util::binary_in_path("cliphist") {
        return Err("cliphist not found".to_string());
    }
    let store = ocr_store.inner().as_ref();
    let out = list_entries()
        .into_iter()
        .take(limit.max(1))
        .map(|e| match e.kind {
            ClipKind::Text(content) => {
                let first = content.lines().next().unwrap_or("").trim();
                let (content_type, color) = detect_content_type(first);
                let preview = if first.is_empty() { "(blank)".to_string() } else { first.to_string() };
                ClipboardEntry {
                    id: e.id,
                    kind: "text".to_string(),
                    preview,
                    content_type,
                    color,
                    byte_size: None,
                    dimensions: None,
                    format: None,
                    ocr_text: None,
                }
            }
            ClipKind::Image { label } => {
                let (byte_size, format, dimensions) = parse_image_label(&label);
                // Cheap cache lookup - no decode/OCR here. Only surface text on a
                // byte-size match so a reused id (post-deletion) can't mislabel.
                let ocr_text = store.and_then(|s| s.get(&e.id)).and_then(|(bs, text)| {
                    if byte_size.map_or(true, |b| b == bs) && !text.trim().is_empty() {
                        Some(text)
                    } else {
                        None
                    }
                });
                ClipboardEntry {
                    id: e.id,
                    kind: "image".to_string(),
                    preview: label,
                    content_type: "image".to_string(),
                    color: None,
                    byte_size,
                    dimensions,
                    format,
                    ocr_text,
                }
            }
        })
        .collect();
    Ok(out)
}

/// Set while the background clipboard-OCR pass runs, to coalesce overlapping
/// triggers (the frontend fires one each time clipboard mode opens).
static OCR_INDEXING: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize, Clone)]
struct ClipboardOcrProgress {
    indexed: usize,
    total: usize,
}

/// Background pass: decode + OCR every image entry whose cache is missing or
/// stale, store the text, then emit `clipboard-ocr-done` so the frontend can
/// reload and pick up `ocr_text`. Cheap to call repeatedly - cached entries are
/// skipped and a second concurrent run is a no-op.
#[tauri::command]
pub fn index_clipboard_ocr(
    app: tauri::AppHandle,
    config: tauri::State<ConfigState>,
    ocr_store: tauri::State<ClipboardOcrState>,
) {
    let (enabled, lang) = config
        .lock()
        .map(|c| (c.clipboard.ocr_images, c.content.ocr_language.clone()))
        .unwrap_or((false, "eng".to_string()));
    if !enabled || !crate::util::binary_in_path("cliphist") {
        return;
    }
    let store = match ocr_store.inner().clone() {
        Some(s) => s,
        None => return,
    };
    // Coalesce: bail if a pass is already running. Cleared at the end of the thread.
    if OCR_INDEXING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    std::thread::spawn(move || {
        let entries = list_entries();
        // Live ids for pruning rows of deleted clipboard entries.
        let live_ids: HashSet<String> = entries.iter().map(|e| e.id.clone()).collect();

        // Image entries whose cache is missing or whose byte_size changed.
        let pending: Vec<(String, Option<u64>)> = entries
            .into_iter()
            .filter_map(|e| match e.kind {
                ClipKind::Image { label } => {
                    let (byte_size, _, _) = parse_image_label(&label);
                    let cached = store.get(&e.id);
                    let fresh = matches!((&cached, byte_size), (Some((bs, _)), Some(b)) if *bs == b)
                        || cached.as_ref().is_some_and(|(bs, _)| byte_size.is_none() && *bs == 0);
                    if fresh {
                        None
                    } else {
                        Some((e.id, byte_size))
                    }
                }
                ClipKind::Text(_) => None,
            })
            .collect();

        let total = pending.len();
        for (i, (id, byte_size)) in pending.into_iter().enumerate() {
            let bytes = match std::process::Command::new("cliphist")
                .args(["decode", &id])
                .output()
            {
                Ok(o) if o.status.success() => o.stdout,
                _ => continue,
            };
            match crate::content_index::ocr_bytes(&bytes, &lang) {
                // Store even empty text as a negative-cache tombstone so a
                // text-free image isn't re-OCR'd on every pass.
                Ok(text) => store.upsert(&id, byte_size.unwrap_or(0), text.trim()),
                Err(e) => eprintln!("[clipboard] ocr failed for {id}: {e}"),
            }
            let _ = app.emit(
                "clipboard-ocr-progress",
                ClipboardOcrProgress { indexed: i + 1, total },
            );
        }

        store.prune(&live_ids);
        OCR_INDEXING.store(false, Ordering::Release);
        let _ = app.emit("clipboard-ocr-done", ());
    });
}

// ── provider ────────────────────────────────────────────────────────────────

pub struct ClipboardProvider;

impl ClipboardProvider {
    pub fn is_available() -> bool {
        use crate::util::binary_in_path;
        let cliphist = binary_in_path("cliphist");
        let wl_copy = binary_in_path("wl-copy");
        if !cliphist {
            eprintln!("[portunus] clipboard: cliphist not found - clipboard provider disabled");
        } else if !wl_copy {
            eprintln!("[portunus] clipboard: wl-copy not found - clipboard provider disabled");
        }
        cliphist && wl_copy
    }
}

impl Provider for ClipboardProvider {
    fn id(&self) -> &str {
        "clipboard"
    }

    // The clipboard browser is a frontend takeover; the provider contributes
    // no inline rows - only the command entry below.
    fn search(&self, _query: &str) -> Vec<SearchResult> {
        vec![]
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        use crate::providers::command::{CommandRoute, CommandSource, ModeKind};
        vec![CommandDescriptor {
            id: "cmd:clipboard".to_string(),
            title: "Clipboard History".to_string(),
            chip: "Clipboard".to_string(),
            subtitle: Some("Browse, paste and manage copied items".to_string()),
            source: CommandSource::Builtin,
            mode_kind: ModeKind::Scope,
            keywords: vec![
                "clip".into(),
                "clipboard".into(),
                "paste".into(),
                "history".into(),
                "copy".into(),
            ],
            placeholder: Some("Search clipboard history…".to_string()),
            min_query_len: 0,
            result_kind: "clipboard".to_string(),
            glyph: Some("clipboard".to_string()),
            icon_data_uri: None,
            route: CommandRoute::UiTakeover,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hex_colors() {
        assert_eq!(detect_content_type("#ff8800").0, "color");
        assert_eq!(detect_content_type("#FFF").0, "color");
        assert_eq!(detect_content_type("#11223344").0, "color");
        assert_eq!(detect_content_type("#ggg").0, "text");
        assert_eq!(detect_content_type("#12").0, "text");
    }

    #[test]
    fn detects_rgb_colors() {
        assert_eq!(detect_content_type("rgb(12, 34, 56)").0, "color");
        assert_eq!(detect_content_type("rgba(0,0,0,0.5)").0, "color");
        assert_eq!(detect_content_type("rgb(nope)").0, "text");
    }

    #[test]
    fn detects_urls() {
        assert_eq!(detect_content_type("https://example.com/x").0, "url");
        assert_eq!(detect_content_type("http://a.bc").0, "url");
        assert_eq!(detect_content_type("https://a b").0, "text");
        assert_eq!(detect_content_type("see https://x.com").0, "text");
    }

    #[test]
    fn detects_json() {
        assert_eq!(detect_content_type("{\"a\":1}").0, "json");
        assert_eq!(detect_content_type("[{\"a\":1}]").0, "json");
        assert_eq!(detect_content_type("plain text").0, "text");
    }

    #[test]
    fn parses_image_labels() {
        assert_eq!(parse_image_label("83 KiB png 1068x904"), (Some(83 * 1024), Some("png".into()), Some((1068, 904))));
        assert_eq!(parse_image_label("512 B jpeg 32x32"), (Some(512), Some("jpeg".into()), Some((32, 32))));
        let (_, fmt, dims) = parse_image_label("png 64x64");
        assert_eq!(fmt.as_deref(), Some("png"));
        assert_eq!(dims, Some((64, 64)));
    }
}

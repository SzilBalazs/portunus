use tauri::Manager;

use super::{Provider, SearchResult, SCORE_CLIPBOARD};

#[tauri::command]
pub fn paste_clipboard(app: tauri::AppHandle, id: String) {
    use std::process::{Command, Stdio};
    if let Ok(decoded) = Command::new("cliphist").args(["decode", &id]).output() {
        if decoded.status.success() {
            if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
                if let Some(stdin) = child.stdin.take() {
                    use std::io::Write;
                    let mut stdin = stdin;
                    let _ = stdin.write_all(&decoded.stdout);
                }
                let _ = child.wait();
            }
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

#[tauri::command]
pub fn decode_clipboard_entry(id: String) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new("cliphist")
        .args(["decode", &id])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
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

pub struct ClipboardProvider;

impl Provider for ClipboardProvider {
    fn id(&self) -> &'static str { "clipboard" }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim().to_lowercase();
        // Trigger: "clip" through "clipboard" (4-char minimum avoids false matches)
        const KEYWORD: &str = "clipboard";
        if q.len() < 4 { return vec![]; }
        let is_prefix = KEYWORD.starts_with(q.as_str());
        let has_keyword = q.starts_with(KEYWORD);
        if !is_prefix && !has_keyword { return vec![]; }

        let sub = if has_keyword {
            q[KEYWORD.len()..].trim().to_string()
        } else {
            String::new()
        };

        list_entries()
            .into_iter()
            .enumerate()
            .filter_map(|(rank, entry)| {
                let (title, subtitle, kind) = match &entry.kind {
                    ClipKind::Text(content) => {
                        if !sub.is_empty()
                            && !content.to_lowercase().contains(sub.as_str())
                        {
                            return None;
                        }
                        let first = content.lines().next().unwrap_or("").trim();
                        let title = if first.is_empty() { "(blank)" } else { first }.to_string();
                        let lines = content.lines().count();
                        let subtitle = if lines > 1 { Some(format!("{lines} lines")) } else { None };
                        (title, subtitle, "clipboard")
                    }
                    ClipKind::Image { label } => {
                        if !sub.is_empty() { return None; }
                        (format!("Image · {label}"), None, "clipboard-image")
                    }
                };
                Some(SearchResult {
                    id: format!("clipboard:{}", entry.id),
                    title,
                    subtitle,
                    kind: kind.to_string(),
                    score: SCORE_CLIPBOARD - rank as f32,
                    exec: Some(format!("clipboard:copy:{}", entry.id)),
                    icon_path: None,
                    file_size: None,
                    created: None,
                    modified: None,
                })
            })
            .collect()
    }
}

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::{Provider, SearchResult};
use crate::config::SearchConfig;

// ── data types ───────────────────────────────────────────────────────────────

#[derive(Debug)]
struct DesktopEntry {
    name: String,
    #[allow(dead_code)]
    exec: String,
    description: Option<String>,
    icon_path: Option<String>,
}

/// Intermediate: raw fields straight from the .desktop file, icon not yet resolved.
struct ParsedEntry {
    name: String,
    exec: String,
    description: Option<String>,
    icon_name: Option<String>,
}

// ── provider ─────────────────────────────────────────────────────────────────

pub struct AppProvider {
    apps: Vec<DesktopEntry>,
    min_score: u32,
    log_scores: bool,
}

impl AppProvider {
    pub fn new(search_cfg: &SearchConfig, log_scores: bool) -> Self {
        Self {
            apps: load_apps(),
            min_score: search_cfg.min_score_app,
            log_scores,
        }
    }
}

// ── loading ───────────────────────────────────────────────────────────────────

fn xdg_data_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let data_home =
        std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{home}/.local/share"));
    let system_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());

    let mut dirs = vec![PathBuf::from(data_home)];
    dirs.extend(system_dirs.split(':').map(PathBuf::from));
    dirs
}

fn load_apps() -> Vec<DesktopEntry> {
    let icon_index = build_icon_index();
    let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();

    let mut seen: HashSet<String> = HashSet::new();
    let mut apps = Vec::new();

    for data_dir in xdg_data_dirs() {
        let apps_dir = data_dir.join("applications");
        if !apps_dir.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&apps_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            // Dedup by file stem (XDG spec): user dirs take priority over system dirs.
            let stem = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if !seen.insert(stem) {
                continue;
            }
            if let Some(parsed) = parse_desktop(entry.path(), &current_desktop) {
                let icon_path = parsed
                    .icon_name
                    .as_deref()
                    .and_then(|n| resolve_icon(n, &icon_index));
                apps.push(DesktopEntry {
                    name: parsed.name,
                    exec: parsed.exec,
                    description: parsed.description,
                    icon_path,
                });
            }
        }
    }

    apps.sort_by(|a, b| a.name.cmp(&b.name));
    apps
}

// ── icon index ────────────────────────────────────────────────────────────────

/// Build a `stem → best_path` map by reading only the specific subdirectories
/// that contain app icons. Avoids walking the full icon tree (which can be
/// 80k+ files on a system with Papirus or similar large themes installed).
fn build_icon_index() -> HashMap<String, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let roots = [
        format!("{home}/.local/share/icons"),
        "/usr/share/icons".to_string(),
        "/usr/share/pixmaps".to_string(),
    ];

    // (size_dir, category_dir, base_score) — scanned in declaration order.
    // SVG gets an additional +100 bonus inside index_dir.
    let targets: &[(&str, &str, u32)] = &[
        ("scalable", "apps", 190),
        ("scalable", "applications", 190),
        ("256x256", "apps", 180),
        ("128x128", "apps", 170),
        ("64x64", "apps", 160),
        ("48x48", "apps", 150),
        ("48x48", "applications", 150),
        ("32x32", "apps", 140),
        ("24x24", "apps", 130),
        ("22x22", "apps", 125),
        ("16x16", "apps", 110),
    ];

    let mut index: HashMap<String, (String, u32)> = HashMap::new();

    for root_str in &roots {
        let root = PathBuf::from(root_str);
        if !root.is_dir() {
            continue;
        }

        // /usr/share/pixmaps — icons live directly in the root dir.
        index_dir(&root, 35, &mut index);

        // Enumerate installed themes (hicolor, Papirus, Adwaita, …).
        let Ok(theme_entries) = fs::read_dir(&root) else {
            continue;
        };
        let themes: Vec<PathBuf> = theme_entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.path())
            .collect();

        for theme in &themes {
            for (size, cat, score) in targets {
                let dir = theme.join(size).join(cat);
                if dir.is_dir() {
                    index_dir(&dir, *score, &mut index);
                }
            }
        }
    }

    index.into_iter().map(|(k, (path, _))| (k, path)).collect()
}

/// Read one flat directory and insert every SVG/PNG file into the index.
/// SVG gets a +100 bonus on top of `base_score` so it beats same-size PNGs.
fn index_dir(dir: &PathBuf, base_score: u32, index: &mut HashMap<String, (String, u32)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let fmt_bonus: u32 = match path.extension().and_then(|e| e.to_str()) {
            Some("svg") => 100,
            Some("png") => 0,
            _ => continue,
        };
        let score = base_score + fmt_bonus;
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            let slot = index
                .entry(stem.to_string())
                .or_insert_with(|| (String::new(), 0));
            if score > slot.1 {
                slot.0 = path.to_string_lossy().into_owned();
                slot.1 = score;
            }
        }
    }
}

/// Resolve a raw icon field to an absolute path using the pre-built index.
fn resolve_icon(icon: &str, index: &HashMap<String, String>) -> Option<String> {
    // Absolute path: check directly, with extension fallback for bare paths.
    if icon.starts_with('/') {
        let p = std::path::Path::new(icon);
        if p.exists() {
            return Some(icon.to_string());
        }
        for ext in ["svg", "png"] {
            let q = p.with_extension(ext);
            if q.exists() {
                return q.to_str().map(String::from);
            }
        }
        return None;
    }

    index.get(icon).cloned()
}

// ── .desktop parser ───────────────────────────────────────────────────────────

fn parse_desktop(path: &std::path::Path, current_desktop: &str) -> Option<ParsedEntry> {
    let content = fs::read_to_string(path).ok()?;

    let mut in_entry = false;
    let mut fields: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line == "[Desktop Entry]" {
            in_entry = true;
            continue;
        }
        if line.starts_with('[') {
            if in_entry {
                break;
            }
            continue;
        }
        if in_entry && !line.starts_with('#') {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                if !k.contains('[') {
                    fields.insert(k.to_string(), v.trim().to_string());
                }
            }
        }
    }

    if fields.get("Type").map(String::as_str) != Some("Application") {
        return None;
    }
    if fields.get("NoDisplay").map(String::as_str) == Some("true") {
        return None;
    }
    if fields.get("Hidden").map(String::as_str) == Some("true") {
        return None;
    }

    // XDG_CURRENT_DESKTOP is colon-separated; OnlyShowIn/NotShowIn are semicolon-separated.
    if !current_desktop.is_empty() {
        let desktops: Vec<&str> = current_desktop.split(':').collect();
        if let Some(only_in) = fields.get("OnlyShowIn") {
            let allowed: Vec<&str> = only_in.split(';').filter(|s| !s.is_empty()).collect();
            if !desktops.iter().any(|d| allowed.contains(d)) {
                return None;
            }
        }
        if let Some(not_in) = fields.get("NotShowIn") {
            let blocked: Vec<&str> = not_in.split(';').filter(|s| !s.is_empty()).collect();
            if desktops.iter().any(|d| blocked.contains(d)) {
                return None;
            }
        }
    }

    let name = fields.remove("Name")?;
    let exec = fields.remove("Exec")?;
    let description = fields
        .remove("Comment")
        .or_else(|| fields.remove("GenericName"));
    let icon_name = fields.remove("Icon");

    Some(ParsedEntry {
        name,
        exec,
        description,
        icon_name,
    })
}

// ── Provider impl ─────────────────────────────────────────────────────────────

impl Provider for AppProvider {
    fn id(&self) -> &'static str {
        "apps"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        if query.trim().is_empty() {
            return vec![];
        }

        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut char_buf = Vec::new();

        self.apps
            .iter()
            .filter_map(|app| {
                let score = pattern.score(Utf32Str::new(&app.name, &mut char_buf), &mut matcher)?;
                let threshold = super::effective_min_score(self.min_score, query.chars().count());
                if self.log_scores {
                    eprintln!("[apps] {:?} → {:?}  score={} threshold={}", query, app.name, score, threshold);
                }
                if score < threshold {
                    return None;
                }
                Some(SearchResult {
                    id: format!("app:{}", app.name),
                    title: app.name.clone(),
                    subtitle: app.description.clone(),
                    kind: "app".to_string(),
                    score: super::SCORE_APP + score as f32,
                    exec: Some(app.exec.clone()),
                    icon_path: app.icon_path.clone(),
                    file_size: None,
                    created: None,
                    modified: None,
                })
            })
            .collect()
    }
}

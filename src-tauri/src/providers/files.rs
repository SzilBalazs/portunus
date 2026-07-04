use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::UNIX_EPOCH;

use nucleo_matcher::Utf32Str;

use super::{Provider, SearchResult};
use crate::config::{FilesConfig, SharedConfig};
use crate::util;

// ── Data types ────────────────────────────────────────────────────────────────

pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub parent: String,
    pub is_dir: bool,
    pub file_size: Option<u64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
}

// ── Search provider ───────────────────────────────────────────────────────────

pub struct FileProvider {
    entries: Arc<RwLock<Vec<FileEntry>>>,
    shared: SharedConfig,
}

impl FileProvider {
    pub fn walk_dirs(files_cfg: &FilesConfig) -> Vec<FileEntry> {
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
        entries
    }

    /// Returns entries for `path` and, if it is a directory, all of its contents up to
    /// the remaining depth budget. Use this instead of `entry_from_path` when handling
    /// a directory that may have been moved in (e.g. a rename event).
    pub fn entries_for_path(path: &Path, base: &Path, max_depth: usize) -> Vec<FileEntry> {
        let Some(root) = Self::entry_from_path(path, base, max_depth) else {
            return vec![];
        };
        if !root.is_dir {
            return vec![root];
        }
        let rel_depth = match path.strip_prefix(base) {
            Ok(rel) => rel.components().count(),
            Err(_) => return vec![root],
        };
        let remaining = max_depth.saturating_sub(rel_depth);
        let mut entries = vec![root];
        if remaining > 0 {
            for wentry in walkdir::WalkDir::new(path)
                .max_depth(remaining)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if wentry.depth() == 0 {
                    continue;
                }
                if let Some(fe) = Self::entry_from_path(wentry.path(), base, max_depth) {
                    entries.push(fe);
                }
            }
        }
        entries
    }

    pub fn entry_from_path(path: &Path, base: &Path, max_depth: usize) -> Option<FileEntry> {
        let rel = path.strip_prefix(base).ok()?;
        let depth = rel.components().count();
        if depth == 0 || depth > max_depth {
            return None;
        }
        let name = path.file_name()?.to_str()?.to_owned();
        let parent = path.parent()?.to_str()?.to_owned();
        let meta = std::fs::metadata(path).ok()?;
        let is_dir = meta.is_dir();
        let file_size = if is_dir { None } else { Some(meta.len()) };
        let created = meta
            .created()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        Some(FileEntry {
            path: path.to_string_lossy().into_owned(),
            name,
            parent,
            is_dir,
            file_size,
            created,
            modified,
        })
    }

    pub fn with_entries(entries: Arc<RwLock<Vec<FileEntry>>>, shared: SharedConfig) -> Self {
        Self { entries, shared }
    }

    /// Full-walk `files_cfg` and replace `entries` with the result. Shared by
    /// the startup build and the config-reload full-rewalk path.
    pub fn populate(entries: &Arc<RwLock<Vec<FileEntry>>>, files_cfg: &FilesConfig) {
        *util::write(entries) = Self::walk_dirs(files_cfg);
    }
}

fn has_hidden_component(path: &str) -> bool {
    use std::path::Component;
    std::path::Path::new(path).components().any(|c| {
        matches!(c, Component::Normal(s) if s.to_string_lossy().starts_with('.'))
    })
}

/// Extensions with a real preview renderer. MUST stay in sync with
/// `isFilePreviewable` / the ext maps in src/utils.ts.
fn is_previewable_ext(name: &str) -> bool {
    // extensionless "Dockerfile"/"Makefile" match via lowercased whole name
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(),
        "pdf"
        | "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif"
        | "svg"
        | "csv" | "tsv"
        | "docx" | "pptx" | "odt" | "odp"
        | "xlsx" | "ods"
        | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go"
        | "sh" | "bash" | "zsh" | "json" | "toml" | "ini" | "conf" | "cfg"
        | "env" | "yaml" | "yml" | "md" | "css" | "scss" | "less"
        | "html" | "htm" | "xml" | "vue" | "c" | "h" | "cpp" | "cc" | "cxx"
        | "hh" | "hpp" | "java" | "rb" | "kt" | "kts" | "sql" | "php" | "lua"
        | "swift" | "dockerfile" | "makefile" | "rst" | "log" | "txt"
    )
}

impl Provider for FileProvider {
    fn id(&self) -> &str {
        "files"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim();
        if q.is_empty() {
            return vec![];
        }

        let cfg = util::read(&self.shared);
        let min_quality = cfg.min_quality;
        let show_dotfiles = cfg.show_dotfiles;
        let log_scores = cfg.log_scores;
        drop(cfg);

        let (pattern, mut matcher, mut char_buf) = super::fuzzy_setup(query);
        let threshold = super::quality_threshold(min_quality, query.chars().count());
        let entries = util::read(&self.entries);

        let mut candidates: Vec<(u32, SearchResult)> = entries
            .iter()
            .filter_map(|entry| {
                if !show_dotfiles && has_hidden_component(&entry.path) {
                    return None;
                }
                let score =
                    pattern.score(Utf32Str::new(&entry.name, &mut char_buf), &mut matcher)?;
                let base = if entry.is_dir {
                    super::SCORE_FOLDER
                } else {
                    super::SCORE_FILE
                };
                let mut penalty = 0.0;
                // folders always render a listing → no preview penalty for dirs
                if !entry.is_dir && !is_previewable_ext(&entry.name) {
                    penalty += super::PENALTY_NO_PREVIEW;
                }
                if has_hidden_component(&entry.path) {
                    penalty += super::PENALTY_HIDDEN;
                }
                let escaped = entry.path.replace('"', "\\\"");
                Some((score, SearchResult {
                    id: format!("file:{}", entry.path),
                    title: entry.name.clone(),
                    subtitle: Some(entry.parent.clone()),
                    kind: if entry.is_dir { "folder" } else { "file" }.to_string(),
                    score: base + super::fuzzy_bonus(score) - penalty,
                    exec: Some(format!("xdg-open \"{}\"", escaped)),
                    file_size: entry.file_size,
                    created: entry.created,
                    modified: entry.modified,
                    ..Default::default()
                }))
            })
            .collect();

        candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0));

        // Adaptive floor: relax threshold so top 3 always survive.
        let floor = candidates.get(2).map(|c| c.0).unwrap_or(0) as f32;
        let effective = threshold.min(floor);

        candidates
            .into_iter()
            .filter(|(score, result)| {
                if log_scores {
                    eprintln!(
                        "[files] {:?} → {:?}  score={} effective_threshold={:.1}",
                        query, result.title, score, effective
                    );
                }
                (*score as f32) >= effective
            })
            .map(|(_, result)| result)
            .collect()
    }
}

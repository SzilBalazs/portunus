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

impl Provider for FileProvider {
    fn id(&self) -> &str {
        "files"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        let q = query.trim();
        if q.is_empty() || q.starts_with('!') {
            return vec![];
        }

        let cfg = util::read(&self.shared);
        let min_score = cfg.min_score_file;
        let recency_weight = cfg.recency_weight;
        let log_scores = cfg.log_scores;
        drop(cfg);

        let (pattern, mut matcher, mut char_buf) = super::fuzzy_setup(query);
        let entries = util::read(&self.entries);

        entries
            .iter()
            .filter_map(|entry| {
                let score =
                    pattern.score(Utf32Str::new(&entry.name, &mut char_buf), &mut matcher)?;
                let threshold = super::effective_min_score(min_score, query.chars().count());
                if log_scores {
                    eprintln!(
                        "[files] {:?} → {:?}  score={} threshold={}",
                        query, entry.name, score, threshold
                    );
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
                    file_size: entry.file_size,
                    created: entry.created,
                    modified: entry.modified,
                    ..Default::default()
                })
            })
            .collect()
    }
}

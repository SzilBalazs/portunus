use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::{Provider, SearchResult};

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
}

impl FileProvider {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        let roots: Vec<(PathBuf, usize)> = vec![
            (PathBuf::from(&home).join("Downloads"), 2),
        ];
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
                entries.push(FileEntry { path: path.to_string_lossy().into_owned(), name, parent, is_dir, file_size, created, modified });
            }
        }
        Self { entries }
    }
}

impl Provider for FileProvider {
    fn id(&self) -> &'static str {
        "files"
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

        self.entries
            .iter()
            .filter_map(|entry| {
                let score =
                    pattern.score(Utf32Str::new(&entry.name, &mut char_buf), &mut matcher)?;
                if score < super::MIN_NUCLEO_SCORE {
                    return None;
                }
                let base = if entry.is_dir { super::SCORE_FOLDER } else { super::SCORE_FILE };
                let recency = super::recency_bonus(entry.created, entry.modified);
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
                    modified: entry.modified,
                })
            })
            .collect()
    }
}

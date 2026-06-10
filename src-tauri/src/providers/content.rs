use std::path::Path;
use std::sync::Arc;

use super::{Provider, SearchResult, SCORE_CONTENT};
use crate::content_index::ContentIndex;

pub struct ContentProvider {
    index: Arc<ContentIndex>,
}

impl ContentProvider {
    pub fn new(index: Arc<ContentIndex>) -> Self {
        Self { index }
    }
}

impl Provider for ContentProvider {
    fn id(&self) -> &str {
        "content"
    }

    fn search(&self, query: &str) -> Vec<SearchResult> {
        // Content scope is selected by the caller (PluginRegistry::search_content),
        // so the raw query is the search term - no activation prefix to strip.
        let q = query.trim();
        if q.len() < 2 {
            return vec![];
        }

        // Replace non-alphanumeric chars (except apostrophe) with spaces,
        // then join tokens - FTS5 treats space-separated terms as AND.
        let cleaned: String = q
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
            .collect();
        let tokens: Vec<&str> = cleaned.split_whitespace().collect();
        let fts_query = tokens.join(" ");

        if fts_query.is_empty() {
            return vec![];
        }

        match self.index.search(&fts_query, 50) {
            Ok(results) => results
                .into_iter()
                .map(|(path, rank, snip, mtime, size)| {
                    let p = Path::new(&path);
                    let title = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&path)
                        .to_owned();
                    let parent = p
                        .parent()
                        .and_then(|p| p.to_str())
                        .unwrap_or("")
                        .to_owned();
                    let escaped = path.replace('"', "\\\"");
                    let match_page = if path.to_lowercase().ends_with(".pdf") {
                        self.index.best_page(&path, &fts_query)
                    } else {
                        None
                    };
                    let created = std::fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.created().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    SearchResult {
                        id: format!("file:{path}"),
                        title,
                        subtitle: Some(parent),
                        snippet: Some(snip),
                        kind: "file".to_string(),
                        score: SCORE_CONTENT + (-rank as f32) * 1000.0,
                        exec: Some(format!("xdg-open \"{escaped}\"")),
                        file_size: if size > 0 { Some(size) } else { None },
                        created,
                        modified: if mtime > 0 { Some(mtime as u64) } else { None },
                        match_page,
                        ..Default::default()
                    }
                })
                .collect(),
            Err(e) => {
                eprintln!("[content] search error: {e}");
                vec![]
            }
        }
    }
}

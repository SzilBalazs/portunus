use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::{Provider, SearchResult};

struct RecentEntry {
    path: String,
    name: String,
    parent: String,
    is_dir: bool,
    visited: u64,
    file_size: Option<u64>,
    created: Option<u64>,
    modified: Option<u64>,
}

pub struct RecentProvider {
    entries: Vec<RecentEntry>,
}

impl RecentProvider {
    pub fn new() -> Self {
        Self {
            entries: load_entries(),
        }
    }
}

impl Provider for RecentProvider {
    fn id(&self) -> &'static str {
        "recent"
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
                let base = if entry.is_dir {
                    super::SCORE_FOLDER
                } else {
                    super::SCORE_FILE
                };
                let recency = super::recency_bonus(None, Some(entry.visited));
                let escaped = entry.path.replace('"', "\\\"");
                Some(SearchResult {
                    id: format!("recent:{}", entry.path),
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

fn load_entries() -> Vec<RecentEntry> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let xbel_path = PathBuf::from(home).join(".local/share/recently-used.xbel");

    let content = match std::fs::read_to_string(&xbel_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let doc = match roxmltree::Document::parse(&content) {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    let mut entries = Vec::new();

    for node in doc.descendants().filter(|n| n.has_tag_name("bookmark")) {
        let href = match node.attribute("href") {
            Some(h) => h,
            None => continue,
        };

        let path_str = match decode_file_uri(href) {
            Some(p) => p,
            None => continue,
        };

        let path = PathBuf::from(&path_str);
        if !path.exists() {
            continue;
        }

        let name = match path.file_name().and_then(|n| n.to_str()).map(str::to_owned) {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };

        let parent = path
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_owned();

        let visited = node
            .attribute("visited")
            .or_else(|| node.attribute("modified"))
            .and_then(parse_iso8601)
            .unwrap_or(0);

        let is_dir = path.is_dir();

        let (file_size, created, modified) = match std::fs::metadata(&path) {
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

        entries.push(RecentEntry {
            path: path_str,
            name,
            parent,
            is_dir,
            visited,
            file_size,
            created,
            modified,
        });
    }

    entries.reverse();
    entries.truncate(500);
    entries
}

// Decodes a "file:///path/to/file" URI into an absolute path string.
// Returns None for non-file:// URIs (smb://, ftp://, etc.).
fn decode_file_uri(href: &str) -> Option<String> {
    let encoded = href.strip_prefix("file://")?;
    let raw = encoded.as_bytes();
    let mut bytes: Vec<u8> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hi = (raw[i + 1] as char).to_digit(16)? as u8;
            let lo = (raw[i + 2] as char).to_digit(16)? as u8;
            bytes.push((hi << 4) | lo);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

// Parses "YYYY-MM-DDTHH:MM:SS[.frac][Z]" into a Unix timestamp (UTC).
fn parse_iso8601(s: &str) -> Option<u64> {
    let s = s.get(..19)?;
    let b = s.as_bytes();
    if b.get(4).copied() != Some(b'-')
        || b.get(7).copied() != Some(b'-')
        || b.get(10).copied() != Some(b'T')
        || b.get(13).copied() != Some(b':')
        || b.get(16).copied() != Some(b':')
    {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let mo: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    let h: i64 = s[11..13].parse().ok()?;
    let mi: i64 = s[14..16].parse().ok()?;
    let se: i64 = s[17..19].parse().ok()?;

    // Julian Day Number → Unix day (JDN of 1970-01-01 is 2440588)
    let a = (14 - mo) / 12;
    let yy = y + 4800 - a;
    let m = mo + 12 * a - 3;
    let jdn = d + (153 * m + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045;
    let unix_day = jdn - 2440588;

    let secs = unix_day * 86400 + h * 3600 + mi * 60 + se;
    if secs < 0 {
        return None;
    }
    Some(secs as u64)
}

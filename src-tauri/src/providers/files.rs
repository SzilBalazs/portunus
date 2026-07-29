use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::UNIX_EPOCH;

use nucleo_matcher::Utf32Str;

use super::{ranking, Provider, SearchResult};
use crate::config::{FilesConfig, SharedConfig};
use crate::util;

/// Folders sink below files within the file band (they carry less signal than
/// a name-matched file but still beat the dict-fill band below).
const FOLDER_OFFSET: f32 = -700_000.0;

/// How many top-scored entries `search` materializes into `SearchResult`s. The
/// registry only ever shows `max_results` (default 20) of them, but it re-ranks
/// with frecency and pins afterwards, so the provider hands over enough
/// headroom for that reordering to matter. Everything past this cut is
/// unreachable: a one-character query matches nearly every indexed entry, and
/// building a result for each one cost ~240 ms on a 60k-entry index.
const CANDIDATE_CAP: usize = 128;

// ── Data types ────────────────────────────────────────────────────────────────

pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub parent: String,
    pub is_dir: bool,
    pub file_size: Option<u64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
    /// Path-static, so they are decided once at walk time rather than per
    /// keystroke: both used to cost a fresh allocation per entry per search.
    pub hidden: bool,
    pub previewable: bool,
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
            let walk = walkdir::WalkDir::new(dir)
                .max_depth(*depth)
                .follow_links(false)
                .into_iter()
                // Prune, don't filter: skipping an ignored directory's whole
                // subtree is the point - descending into node_modules to throw
                // the entries away afterwards costs the same walk.
                .filter_entry(|e| e.depth() == 0 || !is_ignored_name(e.file_name(), &files_cfg.ignore));
            for entry in walk.filter_map(|e| e.ok()) {
                if entry.depth() == 0 {
                    continue;
                }
                let is_dir = entry.file_type().is_dir();
                if let Some(fe) = make_entry(entry.path(), is_dir, entry.metadata().ok().as_ref()) {
                    entries.push(fe);
                }
            }
        }
        entries
    }

    /// Returns entries for `path` and, if it is a directory, all of its contents up to
    /// the remaining depth budget. Use this instead of `entry_from_path` when handling
    /// a directory that may have been moved in (e.g. a rename event).
    pub fn entries_for_path(
        path: &Path,
        base: &Path,
        max_depth: usize,
        ignore: &[String],
    ) -> Vec<FileEntry> {
        let Some(root) = Self::entry_from_path(path, base, max_depth, ignore) else {
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
                .filter_entry(|e| e.depth() == 0 || !is_ignored_name(e.file_name(), ignore))
                .filter_map(|e| e.ok())
            {
                if wentry.depth() == 0 {
                    continue;
                }
                if let Some(fe) = Self::entry_from_path(wentry.path(), base, max_depth, ignore) {
                    entries.push(fe);
                }
            }
        }
        entries
    }

    pub fn entry_from_path(
        path: &Path,
        base: &Path,
        max_depth: usize,
        ignore: &[String],
    ) -> Option<FileEntry> {
        let rel = path.strip_prefix(base).ok()?;
        let depth = rel.components().count();
        if depth == 0 || depth > max_depth {
            return None;
        }
        // A watcher event can name a path inside an ignored tree, which the
        // walk would never have produced.
        if rel.iter().any(|c| is_ignored_name(c, ignore)) {
            return None;
        }
        let meta = std::fs::metadata(path).ok()?;
        make_entry(path, meta.is_dir(), Some(&meta))
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

/// Build an entry from a path already known to be in scope. `meta` is passed in
/// because both callers have it in hand (walkdir caches it), and a missing one
/// only costs the size/time fields.
fn make_entry(path: &Path, is_dir: bool, meta: Option<&std::fs::Metadata>) -> Option<FileEntry> {
    let name = path.file_name()?.to_str()?.to_owned();
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or("").to_owned();
    let stamp = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    };
    let (file_size, created, modified) = match meta {
        Some(m) => (
            (!is_dir).then(|| m.len()),
            stamp(m.created()),
            stamp(m.modified()),
        ),
        None => (None, None, None),
    };
    let path_str = path.to_string_lossy().into_owned();
    Some(FileEntry {
        hidden: has_hidden_component(&path_str),
        previewable: !is_dir && is_previewable_ext(&name),
        path: path_str,
        name,
        parent,
        is_dir,
        file_size,
        created,
        modified,
    })
}

/// Allocation-free stand-in for `ranking::detect_tier`, used only to order the
/// candidate cut: 3 = whole name, 2 = prefix, 1 = word start, 0 = fuzzy. Case
/// folding is ASCII-only (`detect_tier` also folds diacritics), which at worst
/// ranks a diacritic title as fuzzy for the cut - the surviving results still
/// get their true tier in phase 2.
fn cheap_tier(name: &str, query: &str) -> u8 {
    let (n, q) = (name.as_bytes(), query.as_bytes());
    if q.is_empty() || q.len() > n.len() {
        return 0;
    }
    let matches_at = |i: usize| n[i..i + q.len()].eq_ignore_ascii_case(q);
    if n.len() == q.len() && matches_at(0) {
        return 3;
    }
    if matches_at(0) {
        return 2;
    }
    // A word start is any position following a non-alphanumeric byte. Bytes are
    // enough: a UTF-8 continuation byte is never ASCII-alphanumeric, so a
    // multi-byte char reads as a separator - the same tier the real
    // `detect_tier` would reach via `char::is_alphanumeric` on punctuation.
    for i in 1..=(n.len() - q.len()) {
        if !n[i - 1].is_ascii_alphanumeric() && matches_at(i) {
            return 1;
        }
    }
    0
}

/// True when `name` is one of the configured ignore names. Exact, whole-component
/// match: an ignore entry is a directory name, not a glob or a substring.
fn is_ignored_name(name: &std::ffi::OsStr, ignore: &[String]) -> bool {
    name.to_str()
        .is_some_and(|n| ignore.iter().any(|i| i == n))
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

        // Phase 1: score only, no allocation. A one-character query matches
        // nearly every entry in the index, so anything built per candidate here
        // is built tens of thousands of times per keystroke.
        let mut scored: Vec<(u8, u32, u32)> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                if !show_dotfiles && entry.hidden {
                    return None;
                }
                let score =
                    pattern.score(Utf32Str::new(&entry.name, &mut char_buf), &mut matcher)?;
                Some((cheap_tier(&entry.name, q), score, i as u32))
            })
            .collect();

        // Keep the best CANDIDATE_CAP by (tier, score) - the same order the
        // registry's band composition leads with, so the cut can't drop a title
        // that a tier boost would have put on top. Partition around the cut
        // instead of sorting the whole candidate list.
        let rank = |c: &(u8, u32, u32)| (c.0, c.1);
        if scored.len() > CANDIDATE_CAP {
            scored.select_nth_unstable_by(CANDIDATE_CAP, |a, b| rank(b).cmp(&rank(a)));
            scored.truncate(CANDIDATE_CAP);
        }
        scored.sort_unstable_by(|a, b| rank(b).cmp(&rank(a)));

        // Adaptive floor: relax the threshold so the top 3 fuzzy scores always
        // survive. Read off the kept candidates, which are ordered by tier
        // first, so the third-best score is not simply the third element.
        let mut top: Vec<u32> = scored.iter().map(|c| c.1).collect();
        top.sort_unstable_by(|a, b| b.cmp(a));
        let floor = top.get(2).copied().unwrap_or(0) as f32;
        let effective = threshold.min(floor);

        // Phase 2: materialize the survivors, capped - the registry re-ranks
        // with frecency and pins, then shows only `max_results` of them.
        scored
            .into_iter()
            .filter(|(_, score, _)| (*score as f32) >= effective)
            .map(|(_, score, i)| {
                let entry = &entries[i as usize];
                if log_scores {
                    eprintln!(
                        "[files] {:?} → {:?}  score={} effective_threshold={:.1}",
                        query, entry.name, score, effective
                    );
                }
                let mut intra = 0.0;
                // Folders sink below files within the band but always render a
                // listing → no preview penalty for dirs.
                if entry.is_dir {
                    intra += FOLDER_OFFSET;
                } else if !entry.previewable {
                    intra -= super::PENALTY_NO_PREVIEW;
                }
                if entry.hidden {
                    intra -= super::PENALTY_HIDDEN;
                }
                let mut parts = ranking::ScoreParts::new(
                    ranking::Category::File,
                    ranking::detect_tier(&entry.name, q),
                    score,
                );
                parts.intra = intra;
                let escaped = entry.path.replace('"', "\\\"");
                SearchResult {
                    id: format!("file:{}", entry.path),
                    title: entry.name.clone(),
                    subtitle: Some(entry.parent.clone()),
                    kind: if entry.is_dir { "folder" } else { "file" }.to_string(),
                    exec: Some(format!("xdg-open \"{}\"", escaped)),
                    file_size: entry.file_size,
                    created: entry.created,
                    modified: entry.modified,
                    parts: Some(parts),
                    ..Default::default()
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DirEntry, SharedSearchConfig};

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("portunus-files-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cfg_for(root: &Path, ignore: &[&str]) -> FilesConfig {
        FilesConfig {
            dirs: vec![DirEntry { path: root.to_string_lossy().into_owned(), depth: 4 }],
            show_dotfiles: true,
            colored_icons: true,
            ignore: ignore.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// An ignored directory contributes nothing - neither itself nor its
    /// subtree, however deep.
    #[test]
    fn walk_prunes_ignored_subtrees() {
        let root = tmpdir("ignore-prune");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg/deep")).unwrap();
        std::fs::write(root.join("src/notes.md"), "x").unwrap();
        std::fs::write(root.join("node_modules/pkg/deep/notes.md"), "x").unwrap();

        let entries = FileProvider::walk_dirs(&cfg_for(&root, &["node_modules"]));
        let names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(names.iter().any(|p| p.ends_with("src/notes.md")));
        assert!(
            !names.iter().any(|p| p.contains("node_modules")),
            "ignored subtree leaked: {names:?}"
        );
    }

    /// The watcher resolves paths directly, so the ignore list has to hold on
    /// that path too - otherwise a write inside an ignored tree re-adds it.
    #[test]
    fn watcher_path_inside_ignored_tree_is_rejected() {
        let root = tmpdir("ignore-watch");
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::write(root.join(".git/objects/blob"), "x").unwrap();
        let ignore = vec![".git".to_string()];

        assert!(
            FileProvider::entry_from_path(&root.join(".git/objects/blob"), &root, 4, &ignore)
                .is_none()
        );
        assert!(FileProvider::entries_for_path(&root.join(".git"), &root, 4, &ignore).is_empty());
    }

    /// Path-static flags are decided at walk time; search reads them instead of
    /// re-deriving them per entry per keystroke.
    #[test]
    fn walk_records_hidden_and_previewable() {
        let root = tmpdir("flags");
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden/notes.md"), "x").unwrap();
        std::fs::write(root.join("archive.zip"), "x").unwrap();

        let entries = FileProvider::walk_dirs(&cfg_for(&root, &[]));
        let by_name = |n: &str| entries.iter().find(|e| e.name == n).unwrap();
        assert!(by_name("notes.md").hidden);
        assert!(by_name("notes.md").previewable);
        assert!(!by_name("archive.zip").hidden);
        assert!(!by_name("archive.zip").previewable);
        assert!(!by_name(".hidden").previewable, "a directory is never previewable");
    }

    /// A one-character query matches nearly every indexed entry. The provider
    /// must hand the registry a bounded list and still lead with the best match.
    #[test]
    fn search_caps_candidates_and_keeps_the_best() {
        let root = tmpdir("cap");
        for i in 0..(CANDIDATE_CAP * 3) {
            std::fs::write(root.join(format!("a-filler-{i}.txt")), "x").unwrap();
        }
        // Whole-name match, written last so file order can't carry the test.
        std::fs::write(root.join("a"), "x").unwrap();

        let files_cfg = cfg_for(&root, &[]);
        let mut cfg = Config::default();
        cfg.files = files_cfg.clone();
        let shared: SharedConfig = Arc::new(RwLock::new(SharedSearchConfig::from_config(&cfg)));
        let entries = Arc::new(RwLock::new(FileProvider::walk_dirs(&files_cfg)));
        let provider = FileProvider::with_entries(entries, shared);

        let results = provider.search("a");
        assert!(results.len() <= CANDIDATE_CAP, "unbounded result list: {}", results.len());
        assert_eq!(
            results[0].title, "a",
            "the candidate cut must not drop a whole-name match"
        );
    }

    #[test]
    fn cheap_tier_matches_the_real_tier_for_ascii() {
        use ranking::MatchTier;
        let rank = |t: MatchTier| match t {
            MatchTier::Exact => 3,
            MatchTier::Prefix => 2,
            MatchTier::WordStart => 1,
            MatchTier::Fuzzy => 0,
        };
        for (name, query) in [
            ("notes", "notes"),
            ("Notes", "notes"),
            ("notes.md", "notes"),
            ("my-notes.md", "notes"),
            ("my notes.md", "notes"),
            ("cannotes.md", "notes"),
            ("nt", "notes"),
            ("notes", ""),
        ] {
            assert_eq!(
                cheap_tier(name, query),
                rank(ranking::detect_tier(name, query)),
                "tier mismatch for {name:?} / {query:?}"
            );
        }
    }
}

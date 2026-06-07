pub mod apps;
pub mod calc;
pub mod clipboard;
pub mod content;
pub mod dict;
pub mod files;
pub mod recent;
pub mod timer;
pub mod wasm;

use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::frecency::FrecencyStore;

// Category base scores (not user-tunable; define search priority ordering).
pub const SCORE_CONTENT: f32 = 6_000_000.0;
pub const SCORE_CLIPBOARD: f32 = 5_000_000.0;
pub const SCORE_TIMER: f32 = 4_000_000.0;
pub const SCORE_CALC: f32 = 3_000_000.0;
pub const SCORE_DICT: f32 = 3_000_000.0;
/// Sparse-fill dict rows — kept below files (1M) so they sink to the bottom and
/// only matter when little else matched.
pub const SCORE_DICT_FILL: f32 = 500_000.0;
pub const SCORE_APP: f32 = 2_000_000.0;
/// Base for extension results — between apps (2M) and calc/dict (3M).
pub const SCORE_EXTENSION: f32 = 2_500_000.0;
/// Width of the band extension relevance (0–100) maps into, on top of
/// `SCORE_EXTENSION`. Host-private: authors only ever see the 0–100 scale.
/// Sized so frecency can compete: one launch ≈ weight (5k) ≈ 20 relevance
/// points — a couple of launches reorder typical relevance gaps, while
/// relevance still rules untouched results. At 100k frecency was invisible.
pub const EXTENSION_BAND: f32 = 25_000.0;
pub const SCORE_FILE: f32 = 1_000_000.0;
pub const SCORE_FOLDER: f32 = 0.0;
/// Subtracted from file/content/folder results whose path contains a hidden
/// component (e.g. `.config`), so configs/caches sink below normal results.
pub const SCORE_HIDDEN_PATH_PENALTY: f32 = 5_000_000.0;

/// True if any path segment (dir or file) starts with '.', ignoring '.'/'..'.
pub fn path_has_hidden_component(path: &str) -> bool {
    use std::path::Component;
    std::path::Path::new(path).components().any(|c| {
        matches!(c, Component::Normal(s) if s.to_string_lossy().starts_with('.'))
    })
}

/// Score threshold that scales with query length.
/// Two-phase ramp so the jump to full threshold is gradual:
///   len 1 →   0%    len 2 → 33%    len 3 → 66%
///   len 4 →  77%    len 5 → 88%    len 6+ → 100%
pub fn effective_min_score(min_score: u32, query_len: usize) -> u32 {
    if query_len <= 1 {
        return 0;
    }
    if query_len <= 3 {
        let factor = (query_len - 1) as u64;
        return (min_score as u64 * factor / 3) as u32;
    }
    let extra = (query_len - 3).min(3) as u64;
    let base = min_score as u64 * 2 / 3;
    (base + (min_score as u64 - base) * extra / 3) as u32
}

pub fn recency_bonus(created: Option<u64>, modified: Option<u64>, weight: f32) -> f32 {
    const ONE_YEAR_SECS: f64 = 365.0 * 24.0 * 3600.0;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let newest = [created, modified]
        .iter()
        .filter_map(|&t| t)
        .max()
        .unwrap_or(0);
    if newest == 0 || now <= newest {
        return 0.0;
    }
    let age = (now - newest) as f64;
    let factor = (1.0 - age / ONE_YEAR_SECS).max(0.0) as f32;
    factor * weight
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// FTS snippet with \x02/\x03 as highlight start/end markers.
    pub snippet: Option<String>,
    pub kind: String,
    pub score: f32,
    pub exec: Option<String>,
    pub icon_path: Option<String>,
    /// Pre-built `data:` URI for a validated extension-supplied icon. The
    /// frontend renders it directly; never read `ext.icon` (unvalidated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_data_uri: Option<String>,
    pub file_size: Option<u64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
    /// 0-based page of a PDF where the content query mainly matched. Set only by
    /// the content provider for PDF results; drives which page the preview opens on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_page: Option<u32>,
    /// Original extension DTO for `ext:` results. The frontend passes it back
    /// verbatim on activate/preview so extensions stay stateless.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<portunus_ext_sdk::ExtensionResult>,
}

impl Default for SearchResult {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            subtitle: None,
            snippet: None,
            kind: String::new(),
            score: 0.0,
            exec: None,
            icon_path: None,
            icon_data_uri: None,
            file_size: None,
            created: None,
            modified: None,
            match_page: None,
            ext: None,
        }
    }
}

/// Returns a configured `(Pattern, Matcher, char_buf)` for fuzzy matching `query`.
/// All three fuzzy providers (apps, files, recent) use identical nucleo settings.
pub fn fuzzy_setup(query: &str) -> (nucleo_matcher::pattern::Pattern, nucleo_matcher::Matcher, Vec<char>) {
    use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Config, Matcher};
    let pattern = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
    let matcher = Matcher::new(Config::DEFAULT);
    (pattern, matcher, Vec::new())
}

pub trait Provider: Send + Sync {
    #[allow(dead_code)]
    fn id(&self) -> &str;
    fn search(&self, query: &str) -> Vec<SearchResult>;
}

pub struct PluginRegistry {
    providers: Vec<Box<dyn Provider>>,
    /// WASM extensions, keyed by extension name. Kept out of `providers` so
    /// search can fan them out in parallel instead of paying each one's
    /// timeout sequentially per keystroke.
    extensions: HashMap<String, Arc<wasm::WasmProvider>>,
    max_results: usize,
    frecency: Option<Arc<FrecencyStore>>,
    frecency_weight: f32,
    /// (fill_threshold, fill_max) for dict sparse-fill gating. None disables
    /// gating (dict rows pass through untouched).
    dict_fill: Option<(usize, usize)>,
}

impl PluginRegistry {
    pub fn new(max_results: usize) -> Self {
        Self {
            providers: Vec::new(),
            extensions: HashMap::new(),
            max_results,
            frecency: None,
            frecency_weight: 0.0,
            dict_fill: None,
        }
    }

    /// Adds, replaces, or (with None) removes an extension. The single
    /// mutation path for extension state — build instances before taking the
    /// write lock, this only swaps pointers.
    pub fn set_extension(&mut self, name: &str, provider: Option<Arc<wasm::WasmProvider>>) {
        match provider {
            Some(p) => {
                self.extensions.insert(name.to_string(), p);
            }
            None => {
                self.extensions.remove(name);
            }
        }
    }

    pub fn extension(&self, name: &str) -> Option<Arc<wasm::WasmProvider>> {
        self.extensions.get(name).cloned()
    }

    pub fn extension_names(&self) -> Vec<String> {
        self.extensions.keys().cloned().collect()
    }

    /// Configure dict sparse-fill gating: `(fill_threshold, fill_max)`, or None
    /// to disable gating.
    pub fn set_dict_fill(&mut self, fill: Option<(usize, usize)>) {
        self.dict_fill = fill;
    }

    pub fn register(&mut self, provider: impl Provider + 'static) {
        self.providers.push(Box::new(provider));
    }

    /// Replace a provider by id, or remove it if `new` is None.
    /// Acquires the write lock only for the retain+push (microseconds),
    /// so index building should happen before calling this.
    pub fn replace(&mut self, id: &str, new: Option<Box<dyn Provider>>) {
        self.providers.retain(|p| p.id() != id);
        if let Some(p) = new {
            self.providers.push(p);
        }
    }

    pub fn set_frecency(&mut self, store: Arc<FrecencyStore>, weight: f32) {
        self.frecency = Some(store);
        self.frecency_weight = weight;
    }

    pub fn update_settings(&mut self, max_results: usize, frecency_weight: f32) {
        self.max_results = max_results;
        self.frecency_weight = frecency_weight;
    }

    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        // Fan extensions out first so their wall-clock overlaps the built-ins.
        // Threads are detached: a hung extension is abandoned at the deadline
        // (its own watchdog cancels it; repeated failures bench it), so the
        // keystroke path is bounded by one budget regardless of extension count.
        let pending = if self.extensions.is_empty() || query.is_empty() {
            None
        } else {
            let (tx, rx) = mpsc::channel();
            let mut budget_ms = 0;
            let mut spawned = 0;
            for ext in self.extensions.values() {
                // Benched extensions would fail instantly anyway — don't pay
                // a thread spawn per keystroke for them.
                if ext.is_benched() {
                    continue;
                }
                budget_ms = budget_ms.max(ext.search_budget_ms());
                let ext = ext.clone();
                let tx = tx.clone();
                let q = query.to_string();
                std::thread::spawn(move || {
                    let _ = tx.send(ext.search(&q));
                });
                spawned += 1;
            }
            (spawned > 0).then_some((rx, spawned, budget_ms + 50))
        };

        let mut results: Vec<SearchResult> = self
            .providers
            .iter()
            .flat_map(|p| p.search(query))
            .collect();

        if let Some((rx, count, budget_ms)) = pending {
            let deadline = Instant::now() + Duration::from_millis(budget_ms);
            for _ in 0..count {
                let remaining = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(remaining) {
                    Ok(batch) => results.extend(batch),
                    Err(_) => break,
                }
            }
        }

        // Dict sparse-fill gating: for a plain query, dict rows are fill
        // candidates — keep them only when other results are sparse, capped at
        // fill_max. Explicit "define"/"dict" queries bypass this entirely.
        if let Some((fill_threshold, fill_max)) = self.dict_fill {
            if !dict::is_explicit_dict_query(query) {
                let is_dict = |r: &SearchResult| r.kind == "dict" || r.kind == "dict-hint";
                let non_dict = results.iter().filter(|r| !is_dict(r)).count();
                if non_dict >= fill_threshold {
                    results.retain(|r| !is_dict(r));
                } else {
                    let mut kept = 0;
                    results.retain(|r| {
                        if !is_dict(r) {
                            return true;
                        }
                        kept += 1;
                        kept <= fill_max
                    });
                }
            }
        }

        // Penalize results inside (or being) hidden paths so configs/caches sink.
        for r in &mut results {
            if let Some(path) = r.id.strip_prefix("file:") {
                if path_has_hidden_component(path) {
                    r.score -= SCORE_HIDDEN_PATH_PENALTY;
                }
            }
        }

        // Apply frecency bonus before sort so heavily-used items surface correctly.
        // Skip content results — they are already ranked by FTS5/BM25 relevance and
        // frecency (O(1000) points) would completely override text relevance (O(1) points).
        if let Some(store) = &self.frecency {
            let scores = store.all_scores();
            for r in &mut results {
                if r.score >= SCORE_CONTENT {
                    continue;
                }
                if let Some(&fs) = scores.get(&r.id) {
                    r.score += fs * self.frecency_weight;
                }
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // Deduplicate by exec: same file may appear from both FileProvider and RecentProvider;
        // keep the highest-scored occurrence (already first after sort).
        let mut seen = std::collections::HashSet::new();
        results.retain(|r| match &r.exec {
            Some(e) => seen.insert(e.clone()),
            None => true,
        });
        results.truncate(self.max_results);
        results
    }
}

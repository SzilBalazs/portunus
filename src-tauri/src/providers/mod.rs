pub mod apps;
pub mod breaker;
pub mod calc;
pub mod clipboard;
pub mod command;
pub mod content;
pub mod dict;
pub mod files;
pub mod wasm;

pub use command::CommandDescriptor;

use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::frecency::FrecencyStore;

// ── Utility provider bases (intent-triggered; not competed against by files/apps) ──
pub const SCORE_CONTENT: f32 = 6_000_000.0;
pub const SCORE_CALC: f32 = 3_000_000.0;
pub const SCORE_DICT: f32 = 3_000_000.0;
/// Sparse-fill dict rows - kept below files (1M) so they sink to the bottom and
/// only matter when little else matched.
pub const SCORE_DICT_FILL: f32 = 500_000.0;

// ── Discoverable provider bases (apps, files, folders, extensions) ──
pub const SCORE_APP: f32 = 2_000_000.0;
/// Base for scoped extension results - the user explicitly entered the
/// extension's scope, so like other intent-triggered rows it outranks calc/dict
/// (3M) while staying below clipboard (5M) and content (6M).
pub const SCORE_EXTENSION_TRIGGERED: f32 = 4_000_000.0;
/// Base for always-mode extension results - above apps (2M), below calc/dict (3M).
pub const SCORE_EXTENSION: f32 = 2_500_000.0;
/// Width of the band extension relevance (0-100) maps into, on top of
/// `SCORE_EXTENSION`. Matches FUZZY_MAX_BONUS so relevance and fuzzy quality
/// are on the same scale; frecency can bridge a few relevance points per launch.
pub const EXTENSION_BAND: f32 = 300_000.0;
pub const SCORE_FILE: f32 = 1_000_000.0;
pub const SCORE_FOLDER: f32 = 0.0;

/// Base for command entries ("Define Word", "Search Issues") in root search.
/// Above the scoped extension band (4M) so a well-matched command name leads,
/// below clipboard/content utility rows. The quality band on top is
/// `fuzzy_bonus` (up to FUZZY_MAX_BONUS), so command entries rank on the same
/// fuzzy scale as apps and files.
pub const SCORE_COMMAND: f32 = 4_500_000.0;

// ── File-result penalties (down-rank low-value hits within the file band) ─────
/// Subtracted from a file result that has no preview renderer (archives, video,
/// audio, unknown binaries). Same scale as FUZZY_MAX_BONUS so it outweighs fuzzy
/// quality but never crosses into the folder/app band.
pub const PENALTY_NO_PREVIEW: f32 = 300_000.0;
/// Subtracted from a result whose path has any dot-prefixed component
/// (hidden dir or dotfile). Config/cache noise sinks below normal hits.
pub const PENALTY_HIDDEN: f32 = 200_000.0;

// ── Scoring normalisation constants ──────────────────────────────────────────

/// Nucleo score at which fuzzy bonus is fully awarded. Scores above this are clamped.
pub const FUZZY_REFERENCE: f32 = 1500.0;
/// Max bonus added to a discoverable result's score for a perfect fuzzy match.
pub const FUZZY_MAX_BONUS: f32 = 300_000.0;
/// Frecency score (raw launches after decay) that maps to 100% of history bonus.
pub const FRECENCY_REFERENCE: f32 = 40.0;

/// Returns the normalised fuzzy bonus for a raw nucleo score.
pub fn fuzzy_bonus(nucleo_score: u32) -> f32 {
    (nucleo_score as f32 / FUZZY_REFERENCE).min(1.0) * FUZZY_MAX_BONUS
}

/// Score threshold (in nucleo score units) that scales with query length.
/// Two-phase ramp so the jump to full threshold is gradual:
///   len 1 →   0%    len 2 → 33%    len 3 → 66%
///   len 4 →  77%    len 5 → 88%    len 6+ → 100%
pub fn quality_threshold(min_quality: f32, query_len: usize) -> f32 {
    let base = min_quality * FUZZY_REFERENCE;
    if query_len <= 1 {
        return 0.0;
    }
    if query_len <= 3 {
        let factor = (query_len - 1) as f32;
        return base * factor / 3.0;
    }
    let extra = (query_len - 3).min(3) as f32;
    let low = base * 2.0 / 3.0;
    low + (base - low) * extra / 3.0
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
    /// Name of the extension command that produced an `ext:` result; passed
    /// back on activate/preview so the guest can dispatch on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext_command: Option<String>,
    /// The descriptor behind a `kind: "command"` entry row; the frontend uses
    /// it to enter the command's mode (or run it) on launch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandDescriptor>,
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
            ext_command: None,
            command: None,
        }
    }
}

/// Returns a configured `(Pattern, Matcher, char_buf)` for fuzzy matching `query`.
/// The fuzzy providers (apps, files) use identical nucleo settings.
pub fn fuzzy_setup(query: &str) -> (nucleo_matcher::pattern::Pattern, nucleo_matcher::Matcher, Vec<char>) {
    use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Config, Matcher};
    let pattern = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
    let matcher = Matcher::new(Config::DEFAULT);
    (pattern, matcher, Vec::new())
}

/// Best weighted nucleo score across `fields` (each a `(haystack, weight)`),
/// plus the index of the winning field. `weight` scales a field's raw score so
/// a secondary field (e.g. a description at 0.8) down-ranks against the primary
/// name. Returns `None` when nothing matched. Shared by the apps and command
/// matchers so both search identical nucleo settings across multiple fields.
pub fn fuzzy_best(
    pattern: &nucleo_matcher::pattern::Pattern,
    matcher: &mut nucleo_matcher::Matcher,
    buf: &mut Vec<char>,
    fields: &[(&str, f32)],
) -> Option<(usize, u32)> {
    let mut best: Option<(usize, u32)> = None;
    for (i, (hay, weight)) in fields.iter().enumerate() {
        if let Some(raw) = pattern.score(nucleo_matcher::Utf32Str::new(hay, buf), matcher) {
            let score = (raw as f32 * weight) as u32;
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((i, score));
            }
        }
    }
    best
}

/// Applies the frecency bonus in place. Shared by the sync keystroke path and
/// the streamed async-query path so the formula cannot drift between them.
/// Content results are skipped - they are already ranked by FTS5/BM25 relevance.
pub fn apply_frecency(results: &mut [SearchResult], store: &FrecencyStore, weight: f32) {
    let scores = store.all_scores();
    for r in results.iter_mut() {
        if r.score >= SCORE_CONTENT {
            continue;
        }
        if let Some(&fs) = scores.get(&r.id) {
            let normalized = (fs / FRECENCY_REFERENCE).min(1.0);
            r.score += normalized * weight;
        }
    }
}

pub trait Provider: Send + Sync {
    #[allow(dead_code)]
    fn id(&self) -> &str;
    fn search(&self, query: &str) -> Vec<SearchResult>;
    /// Commands this provider contributes to the catalog. Default: none.
    fn commands(&self) -> Vec<CommandDescriptor> {
        Vec::new()
    }
    /// Scoped search, called only while one of this provider's Scope commands
    /// is the active mode. `command_id` is the descriptor id; providers with a
    /// single scope can ignore it. Default: fall back to root search.
    fn search_scoped(&self, _command_id: &str, query: &str) -> Vec<SearchResult> {
        self.search(query)
    }
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
    /// mutation path for extension state - build instances before taking the
    /// write lock, this only swaps pointers.
    pub fn set_extension(&mut self, name: &str, provider: Option<Arc<wasm::WasmProvider>>) {
        // A reloaded/unloaded extension must not keep streaming from its old
        // instance - cancel its in-flight async query (the worker's own Arc
        // keeps the old provider alive until it unwinds).
        if let Some(qm) = crate::extensions::query::manager() {
            qm.cancel_ext(name);
        }
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

    /// Frecency store + weight, for scoring streamed async-query results with
    /// the exact same bonus as the sync path.
    pub fn frecency_params(&self) -> (Option<Arc<FrecencyStore>>, f32) {
        (self.frecency.clone(), self.frecency_weight)
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
                // Benched extensions would fail instantly anyway - don't pay
                // a thread spawn per keystroke for them.
                if ext.is_benched() {
                    continue;
                }
                // Root-search gating happens here, before any thread exists: a
                // keystroke with no `always` command spawns nothing and adds
                // nothing to the deadline budget. gate() resolves at most ONE
                // command per extension, preserving the one-thread-per-
                // extension invariant with multi-command manifests. Root results
                // are discovery-band (intent = false); explicit invocation is the
                // scope path (search_scope), which bands higher.
                let Some(gc) = ext.gate(query) else {
                    continue;
                };
                budget_ms = budget_ms.max(ext.search_budget_ms());
                let ext = ext.clone();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(ext.search_gated(gc, false));
                });
                spawned += 1;
            }
            (spawned > 0).then_some((rx, spawned, budget_ms + 50))
        };

        // The content provider runs only in its scope (search_scope); the
        // normal launcher never competes file-contents against names/apps.
        let mut results: Vec<SearchResult> = self
            .providers
            .iter()
            .filter(|p| p.id() != "content")
            .flat_map(|p| p.search(query))
            .collect();

        // Searchable command entries ("Define Word", "Clipboard History", …).
        results.extend(command::match_entries(&self.commands(), query));

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

        // Dict sparse-fill gating: dict rows are fill candidates - keep them
        // only when other results are sparse, capped at fill_max. (Explicit
        // lookups happen in the Dict scope, not root search, so there is no
        // longer a query shape that bypasses this.)
        if let Some((fill_threshold, fill_max)) = self.dict_fill {
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

        // Apply frecency bonus before sort so heavily-used items surface correctly.
        if let Some(store) = &self.frecency {
            apply_frecency(&mut results, store, self.frecency_weight);
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // Deduplicate by exec: keep highest-scored occurrence (already first after sort).
        let mut seen = std::collections::HashSet::new();
        results.retain(|r| match &r.exec {
            Some(e) => seen.insert(e.clone()),
            None => true,
        });
        results.truncate(self.max_results);
        results
    }

    /// The full command catalog: built-in provider commands plus (once
    /// extensions declare them) extension commands. Recomputed per call - the
    /// descriptor lists are tiny and this keeps reload invalidation free.
    pub fn commands(&self) -> Vec<CommandDescriptor> {
        let mut out: Vec<CommandDescriptor> = builtin_commands();
        out.extend(self.providers.iter().flat_map(|p| p.commands()));
        for ext in self.extensions.values() {
            out.extend(ext.commands());
        }
        out
    }

    /// Scoped search for an active Scope command (`command_id` is the
    /// descriptor id). Routes to the owning provider/extension; results keep
    /// the provider's own ranking (no frecency/dict-fill - scopes are already
    /// intent-filtered). Unknown ids return empty.
    pub fn search_scope(&self, command_id: &str, query: &str) -> Vec<SearchResult> {
        let commands = self.commands();
        let Some(cmd) = commands.iter().find(|c| c.id == command_id) else {
            return Vec::new();
        };
        let mut results = match &cmd.route {
            command::CommandRoute::Builtin { provider_id } => self
                .providers
                .iter()
                .filter(|p| p.id() == provider_id.as_str())
                .flat_map(|p| p.search_scoped(command_id, query))
                .collect(),
            // Sync tier of an extension scope; the async `query` tier is
            // dispatched by the command layer (lib.rs) alongside this.
            command::CommandRoute::Extension { name, command } => match self.extensions.get(name) {
                Some(ext) if !ext.is_benched() => ext
                    .gate_scoped(command, query)
                    .map(|gc| ext.search_gated(gc, true))
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            command::CommandRoute::UiTakeover => Vec::new(),
            // Action route: activated frontend-side, never scope-searched.
            command::CommandRoute::Invoke { .. } => Vec::new(),
        };
        sort_by_score(&mut results);
        results.truncate(self.max_results);
        results
    }
}

/// Built-in commands that belong to no single provider (app-level actions).
fn builtin_commands() -> Vec<CommandDescriptor> {
    use command::{CommandRoute, CommandSource, ModeKind};
    vec![CommandDescriptor {
        id: "cmd:settings".to_string(),
        title: "Open Settings".to_string(),
        chip: "Settings".to_string(),
        subtitle: Some("Portunus".to_string()),
        source: CommandSource::Builtin,
        mode_kind: ModeKind::Action,
        keywords: vec![
            "settings".into(),
            "preferences".into(),
            "config".into(),
            "configuration".into(),
            "options".into(),
        ],
        placeholder: None,
        min_query_len: 0,
        result_kind: "command".to_string(),
        glyph: Some("settings".to_string()),
        icon_data_uri: None,
        opens_form: false,
        route: CommandRoute::Invoke { command: "open_settings_window".to_string() },
    }]
}

fn sort_by_score(results: &mut [SearchResult]) {
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

pub mod apps;
pub mod calc;
pub mod clipboard;
pub mod dict;
pub mod files;
pub mod recent;
pub mod timer;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::frecency::FrecencyStore;

// Category base scores (not user-tunable; define search priority ordering).
pub const SCORE_TIMER: f32 = 4_000_000.0;
pub const SCORE_CLIPBOARD: f32 = 5_000_000.0;
pub const SCORE_CALC: f32 = 3_000_000.0;
pub const SCORE_DICT: f32 = 3_000_000.0;
pub const SCORE_APP: f32 = 2_000_000.0;
pub const SCORE_FILE: f32 = 1_000_000.0;
pub const SCORE_FOLDER: f32 = 0.0;

/// Score threshold that scales with query length.
/// min_score is the target for 4+ char queries; shorter queries get a proportionally
/// lower threshold because nucleo scores naturally drop with query length.
pub fn effective_min_score(min_score: u32, query_len: usize) -> u32 {
    if query_len <= 1 {
        return 0;
    }
    let factor = (query_len - 1).min(3) as u64;
    (min_score as u64 * factor / 3) as u32
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
    pub kind: String,
    pub score: f32,
    pub exec: Option<String>,
    pub icon_path: Option<String>,
    pub file_size: Option<u64>,
    pub created: Option<u64>,
    pub modified: Option<u64>,
}

pub trait Provider: Send + Sync {
    #[allow(dead_code)]
    fn id(&self) -> &'static str;
    fn search(&self, query: &str) -> Vec<SearchResult>;
}

pub struct PluginRegistry {
    providers: Vec<Box<dyn Provider>>,
    max_results: usize,
    frecency: Option<Arc<FrecencyStore>>,
    frecency_weight: f32,
}

impl PluginRegistry {
    pub fn new(max_results: usize) -> Self {
        Self {
            providers: Vec::new(),
            max_results,
            frecency: None,
            frecency_weight: 0.0,
        }
    }

    pub fn register(&mut self, provider: impl Provider + 'static) {
        self.providers.push(Box::new(provider));
    }

    pub fn set_frecency(&mut self, store: Arc<FrecencyStore>, weight: f32) {
        self.frecency = Some(store);
        self.frecency_weight = weight;
    }

    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let mut results: Vec<SearchResult> = self
            .providers
            .iter()
            .flat_map(|p| p.search(query))
            .collect();

        // Apply frecency bonus before sort so heavily-used items surface correctly.
        if let Some(store) = &self.frecency {
            let scores = store.all_scores();
            for r in &mut results {
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

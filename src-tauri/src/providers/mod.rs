pub mod apps;
pub mod calc;
pub mod files;
pub mod recent;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

pub const SCORE_CALC: f32 = 3_000_000.0;
pub const SCORE_APP: f32 = 2_000_000.0;
pub const SCORE_FILE: f32 = 1_000_000.0;
pub const SCORE_FOLDER: f32 = 0.0;

pub const MIN_NUCLEO_SCORE: u32 = 80;
pub const MIN_NUCLEO_SCORE_APP: u32 = 50;
pub const RECENCY_WEIGHT: f32 = 50.0;
const ONE_YEAR_SECS: f64 = 365.0 * 24.0 * 3600.0;

pub fn recency_bonus(created: Option<u64>, modified: Option<u64>) -> f32 {
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
    factor * RECENCY_WEIGHT
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
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: impl Provider + 'static) {
        self.providers.push(Box::new(provider));
    }

    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let mut results: Vec<SearchResult> = self
            .providers
            .iter()
            .flat_map(|p| p.search(query))
            .collect();
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // deduplicate by exec: same file may appear from both FileProvider and RecentProvider;
        // keep the highest-scored occurrence (already first after sort)
        let mut seen = std::collections::HashSet::new();
        results.retain(|r| match &r.exec {
            Some(e) => seen.insert(e.clone()),
            None => true,
        });
        results.truncate(8);
        results
    }
}

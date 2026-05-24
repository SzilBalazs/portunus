pub mod apps;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub kind: String,
    pub score: f32,
    pub exec: Option<String>,
    pub icon_path: Option<String>,
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
        Self { providers: Vec::new() }
    }

    pub fn register(&mut self, provider: impl Provider + 'static) {
        self.providers.push(Box::new(provider));
    }

    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let mut results: Vec<SearchResult> = self.providers
            .iter()
            .flat_map(|p| p.search(query))
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(5);
        results
    }
}

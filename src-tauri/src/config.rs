use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

// ── top-level ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub providers: ProvidersConfig,
    pub files: FilesConfig,
    pub recent: RecentConfig,
    pub search: SearchConfig,
    pub frecency: FrecencyConfig,
    pub debug: DebugConfig,
    pub content: ContentConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            providers: ProvidersConfig::default(),
            files: FilesConfig::default(),
            recent: RecentConfig::default(),
            search: SearchConfig::default(),
            frecency: FrecencyConfig::default(),
            debug: DebugConfig::default(),
            content: ContentConfig::default(),
        }
    }
}

// ── sections ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub max_results: usize,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self { max_results: 20 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub apps: bool,
    pub files: bool,
    pub recent: bool,
    pub calc: bool,
    pub dict: bool,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            apps: true,
            files: true,
            recent: true,
            calc: true,
            dict: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DirEntry {
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
}

fn default_depth() -> usize {
    2
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct FilesConfig {
    pub dirs: Vec<DirEntry>,
}

impl Default for FilesConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        Self {
            dirs: vec![
                DirEntry { path: format!("{home}/Downloads"), depth: 2 },
                DirEntry { path: format!("{home}/Documents"), depth: 2 },
                DirEntry { path: format!("{home}/.config/hypr"), depth: 2 },
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct RecentConfig {
    pub max_entries: usize,
}

impl Default for RecentConfig {
    fn default() -> Self {
        Self { max_entries: 500 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub min_score_file: u32,
    pub min_score_app: u32,
    pub recency_weight: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            min_score_file: 95,
            min_score_app: 95,
            recency_weight: 50.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct DebugConfig {
    pub log_scores: bool,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self { log_scores: false }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct FrecencyConfig {
    pub enabled: bool,
    pub half_life_days: f32,
    pub weight: f32,
}

impl Default for FrecencyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            half_life_days: 14.0,
            weight: 5000.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ContentDirEntry {
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
    pub extensions: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ContentConfig {
    pub enabled: bool,
    pub dirs: Vec<ContentDirEntry>,
    pub extensions: Vec<String>,
    pub max_file_bytes: u64,
    pub ocr_images: bool,
    pub ocr_pdf_fallback: bool,
    pub ocr_language: String,
    pub threads: usize,
}

impl Default for ContentConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        Self {
            enabled: false,
            dirs: vec![ContentDirEntry { path: format!("{home}/Documents"), depth: 3, extensions: None }],
            extensions: vec![
                "pdf".into(),
                "txt".into(), "md".into(), "rst".into(), "csv".into(), "log".into(),
                "toml".into(), "yaml".into(), "yml".into(), "json".into(), "xml".into(),
                "sh".into(), "py".into(), "rs".into(), "js".into(), "ts".into(),
                "go".into(), "c".into(), "cpp".into(), "h".into(),
                "jpg".into(), "jpeg".into(), "png".into(), "webp".into(),
                "bmp".into(), "tiff".into(), "tif".into(),
            ],
            max_file_bytes: 5 * 1024 * 1024,
            ocr_images: true,
            ocr_pdf_fallback: true,
            ocr_language: "eng".to_string(),
            threads: 2,
        }
    }
}

// ── shared runtime config ─────────────────────────────────────────────────────

/// Per-search scalars shared across all indexed providers.
/// Updated in-place on config reload; providers read it per search() call.
#[derive(Debug, Clone)]
pub struct SharedSearchConfig {
    pub min_score_file: u32,
    pub min_score_app: u32,
    pub recency_weight: f32,
    pub log_scores: bool,
}

pub type SharedConfig = Arc<RwLock<SharedSearchConfig>>;

impl SharedSearchConfig {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            min_score_file: cfg.search.min_score_file,
            min_score_app: cfg.search.min_score_app,
            recency_weight: cfg.search.recency_weight,
            log_scores: cfg.debug.log_scores,
        }
    }

    pub fn update_from(&mut self, cfg: &Config) {
        self.min_score_file = cfg.search.min_score_file;
        self.min_score_app = cfg.search.min_score_app;
        self.recency_weight = cfg.search.recency_weight;
        self.log_scores = cfg.debug.log_scores;
    }
}

// ── loader ────────────────────────────────────────────────────────────────────

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&path, DEFAULT_CONFIG) {
                    Ok(_) => eprintln!("[config] wrote default config to {}", path.display()),
                    Err(e) => eprintln!("[config] could not write default config to {}: {e}", path.display()),
                }
                return Self::default();
            }
        };
        match toml::from_str::<Config>(&content) {
            Ok(cfg) => {
                eprintln!("[config] loaded from {}", path.display());
                cfg
            }
            Err(e) => {
                eprintln!("[config] failed to parse {}: {e} — using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Expand a path string: replace a leading `~` with $HOME.
    pub fn expand_path(p: &str) -> PathBuf {
        if let Some(rest) = p.strip_prefix("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            PathBuf::from(home).join(rest)
        } else if p == "~" {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/root".to_string()))
        } else {
            PathBuf::from(p)
        }
    }
}

fn config_path() -> PathBuf {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            format!("{home}/.config")
        });
    PathBuf::from(config_home).join("portunus").join("config.toml")
}

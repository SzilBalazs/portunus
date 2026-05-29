use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

// ── top-level ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
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
    pub appearance: AppearanceConfig,
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
            appearance: AppearanceConfig::default(),
        }
    }
}

// ── sections ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub max_results: usize,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self { max_results: 20 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
pub struct DirEntry {
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
}

fn default_depth() -> usize {
    2
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct RecentConfig {
    pub max_entries: usize,
}

impl Default for RecentConfig {
    fn default() -> Self {
        Self { max_entries: 500 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct DebugConfig {
    pub log_scores: bool,
    pub log_watcher: bool,
    pub log_pdf: bool,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self { log_scores: false, log_watcher: false, log_pdf: false }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
pub struct ContentDirEntry {
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
    pub extensions: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
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

impl ContentConfig {
    /// True if the two configs would produce an identical index. Compares every
    /// field that affects *what* gets indexed, ignoring `threads` (which only
    /// affects indexing speed). A threads-only change must not trigger a reindex.
    pub fn contents_eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.dirs == other.dirs
            && self.extensions == other.extensions
            && self.max_file_bytes == other.max_file_bytes
            && self.ocr_images == other.ocr_images
            && self.ocr_pdf_fallback == other.ocr_pdf_fallback
            && self.ocr_language == other.ocr_language
    }
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
                "docx".into(), "xlsx".into(), "pptx".into(),
                "odt".into(), "ods".into(), "odp".into(),
            ],
            max_file_bytes: 5 * 1024 * 1024,
            ocr_images: true,
            ocr_pdf_fallback: true,
            ocr_language: "eng".to_string(),
            threads: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: String,
    pub font_size: u32,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self { theme: "warm-dark".to_string(), font_size: 13 }
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
    pub log_watcher: bool,
    pub log_pdf: bool,
}

pub type SharedConfig = Arc<RwLock<SharedSearchConfig>>;

impl SharedSearchConfig {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            min_score_file: cfg.search.min_score_file,
            min_score_app: cfg.search.min_score_app,
            recency_weight: cfg.search.recency_weight,
            log_scores: cfg.debug.log_scores,
            log_watcher: cfg.debug.log_watcher,
            log_pdf: cfg.debug.log_pdf,
        }
    }

    pub fn update_from(&mut self, cfg: &Config) {
        self.min_score_file = cfg.search.min_score_file;
        self.min_score_app = cfg.search.min_score_app;
        self.recency_weight = cfg.search.recency_weight;
        self.log_scores = cfg.debug.log_scores;
        self.log_watcher = cfg.debug.log_watcher;
        self.log_pdf = cfg.debug.log_pdf;
    }
}

// ── loader ────────────────────────────────────────────────────────────────────

/// Set when `load()` finds an unparseable config file. Drained by the
/// `take_config_error` command so the settings UI can warn the user that their
/// file failed to parse (and was backed up) rather than silently resetting it.
pub static LAST_LOAD_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

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
                // Preserve the unparseable file so a clobbering autosave can't destroy
                // it, and record the error so the settings UI can warn the user rather
                // than silently resetting every setting to defaults.
                let backup = path.with_extension("toml.bak");
                let detail = match std::fs::copy(&path, &backup) {
                    Ok(_) => {
                        eprintln!("[config] backed up unparseable config to {}", backup.display());
                        format!("{e}\n\nYour previous config was backed up to {}.", backup.display())
                    }
                    Err(err) => {
                        eprintln!("[config] could not back up config: {err}");
                        format!("{e}")
                    }
                };
                if let Ok(mut slot) = LAST_LOAD_ERROR.lock() {
                    *slot = Some(detail);
                }
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize config: {e}"))?;
        let path = config_path();
        std::fs::write(&path, toml_str)
            .map_err(|e| format!("failed to write config to {}: {e}", path.display()))
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

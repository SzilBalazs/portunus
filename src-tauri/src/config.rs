use std::path::PathBuf;

use serde::Deserialize;

// ── top-level ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub providers: ProvidersConfig,
    pub files: FilesConfig,
    pub recent: RecentConfig,
    pub search: SearchConfig,
    pub pdf: PdfConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            providers: ProvidersConfig::default(),
            files: FilesConfig::default(),
            recent: RecentConfig::default(),
            search: SearchConfig::default(),
            pdf: PdfConfig::default(),
        }
    }
}

// ── sections ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub max_results: usize,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self { max_results: 9 }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub apps: bool,
    pub files: bool,
    pub recent: bool,
    pub calc: bool,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            apps: true,
            files: true,
            recent: true,
            calc: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirEntry {
    pub path: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
}

fn default_depth() -> usize {
    2
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct RecentConfig {
    pub max_entries: usize,
}

impl Default for RecentConfig {
    fn default() -> Self {
        Self { max_entries: 500 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub min_score_file: u32,
    pub min_score_app: u32,
    pub recency_weight: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            min_score_file: 80,
            min_score_app: 50,
            recency_weight: 50.0,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct PdfConfig {
    pub render_width: u32,
}

impl Default for PdfConfig {
    fn default() -> Self {
        Self { render_width: 800 }
    }
}

// ── loader ────────────────────────────────────────────────────────────────────

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[config] not found at {}: {e} — using defaults", path.display());
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

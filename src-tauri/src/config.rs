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
    pub search: SearchConfig,
    pub frecency: FrecencyConfig,
    pub debug: DebugConfig,
    pub content: ContentConfig,
    pub appearance: AppearanceConfig,
    pub dict: DictConfig,
    pub clipboard: ClipboardConfig,
    pub extensions: ExtensionsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            providers: ProvidersConfig::default(),
            files: FilesConfig::default(),
            search: SearchConfig::default(),
            frecency: FrecencyConfig::default(),
            debug: DebugConfig::default(),
            content: ContentConfig::default(),
            appearance: AppearanceConfig::default(),
            dict: DictConfig::default(),
            clipboard: ClipboardConfig::default(),
            extensions: ExtensionsConfig::default(),
        }
    }
}

/// Clipboard history browser settings.
#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ClipboardConfig {
    /// "auto" = on Enter, paste into the previously focused window (synthesizes
    /// Ctrl+V via wtype); "copy" = only copy to the clipboard and let the user
    /// paste manually. Falls back to copy when wtype is unavailable.
    pub paste_mode: String,
    /// Maximum number of entries loaded into the clipboard browser.
    pub max_entries: usize,
    /// OCR copied images so their visible text is searchable in the browser.
    /// Each image is OCR'd once and cached (see `clipboard_ocr.rs`). The OCR
    /// language is the shared `content.ocr_language` (single canonical setting).
    pub ocr_images: bool,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            paste_mode: "auto".to_string(),
            max_entries: 250,
            ocr_images: true,
        }
    }
}

/// Per-extension state, keyed by extension name:
///
/// ```toml
/// [extensions.emoji]
/// enabled = true
/// [extensions.emoji.settings]
/// skin_tone = "medium"
/// ```
///
/// A discovered extension absent from the map is DISABLED - dropping a folder
/// into the extensions dir must never run code until the user reviews its
/// permissions and enables it in Settings.
pub type ExtensionsConfig = std::collections::HashMap<String, ExtensionEntry>;

#[derive(Debug, Clone, PartialEq, Default, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ExtensionEntry {
    pub enabled: bool,
    /// Values for the extension's `[[settings]]` schema. Validated against
    /// the schema when applied; unknown keys are ignored, never fatal.
    pub settings: toml::Table,
}

// ── sections ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub max_results: usize,
    /// Set to true once the first-launch onboarding wizard has been completed
    /// (or skipped). When false, the launcher shows the wizard on next show.
    pub onboarding_completed: bool,
    /// Use a wlr-layer-shell overlay surface on Wayland for compositor-agnostic
    /// always-on-top launcher behavior. Linux/Wayland only; takes effect on restart.
    pub layer_shell: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self { max_results: 20, onboarding_completed: false, layer_shell: true }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ProvidersConfig {
    pub apps: bool,
    pub files: bool,
    pub calc: bool,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            apps: true,
            files: true,
            calc: true,
        }
    }
}

/// Dictionary provider settings. Moved out of `[providers]` so the sparse-fill
/// behavior and scoring constants are tunable.
#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct DictConfig {
    /// Master on/off for the dict provider (explicit lookups + fill).
    pub enabled: bool,
    /// Fill sparse result lists with dictionary matches for the typed word.
    pub fill_sparse: bool,
    /// Allow edit-distance (typo) matches when filling; false = exact lemma only.
    pub correct_misspellings: bool,
    /// Ctrl+C on a dict result copies the first definition; false = copies the word.
    pub copy_definition: bool,
    /// Only fill when fewer than this many non-dict results exist.
    pub fill_threshold: usize,
    /// Max dictionary rows added when filling.
    pub fill_max: usize,
}

impl Default for DictConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fill_sparse: true,
            correct_misspellings: true,
            copy_definition: true,
            fill_threshold: 3,
            fill_max: 5,
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
    pub show_dotfiles: bool,
    pub colored_icons: bool,
}

impl Default for FilesConfig {
    fn default() -> Self {
        let home = crate::paths::home();
        Self {
            dirs: vec![
                DirEntry { path: format!("{home}/Downloads"), depth: 2 },
                DirEntry { path: format!("{home}/Documents"), depth: 2 },
                DirEntry { path: format!("{home}/.config/hypr"), depth: 2 },
            ],
            show_dotfiles: false,
            colored_icons: true,
        }
    }
}

impl FilesConfig {
    /// True when the index-affecting fields match. `colored_icons` is a
    /// display-only flag, so a change to it must not trigger a file re-walk.
    pub fn index_eq(&self, other: &Self) -> bool {
        self.dirs == other.dirs && self.show_dotfiles == other.show_dotfiles
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Minimum fuzzy match quality, 0.0-1.0. Applied as a fraction of FUZZY_REFERENCE.
    pub min_quality: f32,
    /// How strongly launch history boosts results, 0-100. Maps to history_max_bonus.
    pub history_weight: u8,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            min_quality: 0.06,
            history_weight: 50,
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
}

impl Default for FrecencyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            half_life_days: 14.0,
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
    /// Highlight matched terms over OCR'd image previews (Contents mode). Preview-only:
    /// does not change what is indexed, so toggling never triggers a reindex (excluded
    /// from `contents_eq`). When off, `image_match_rects` returns no boxes.
    pub ocr_highlight: bool,
    /// Cache OCR word boxes in the index at index time so image highlights are instant
    /// (no per-preview OCR). HEAVY: changes what indexing stores, so it is part of
    /// `contents_eq` and toggling forces a full reindex (re-OCRs images). Only meaningful
    /// when `ocr_highlight` is also on.
    pub ocr_highlight_cache: bool,
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
            // ocr_highlight is preview-only and deliberately excluded (no reindex).
            && self.ocr_highlight_cache == other.ocr_highlight_cache
    }
}

impl Default for ContentConfig {
    fn default() -> Self {
        let home = crate::paths::home();
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
            ocr_highlight: false,
            ocr_highlight_cache: false,
        }
    }
}

/// Result-list animation tier. Serialized lowercase ("off"/"slide"/"flip").
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResultAnimation {
    Off,
    Slide,
    Flip,
}

/// Accept the legacy boolean form (`true` → Slide, `false` → Off) as well as the
/// new string enum, so upgrading doesn't fail the whole-config parse and reset
/// every setting to defaults.
fn de_result_animation<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ResultAnimation, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Compat {
        Bool(bool),
        Enum(ResultAnimation),
    }
    Ok(match Compat::deserialize(d)? {
        Compat::Bool(true) => ResultAnimation::Slide,
        Compat::Bool(false) => ResultAnimation::Off,
        Compat::Enum(e) => e,
    })
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: String,
    pub font_size: u32,
    #[serde(deserialize_with = "de_result_animation")]
    pub animate_results: ResultAnimation,
    pub show_metadata: bool,
    /// Tint the selected row + preview panel with the app's dominant icon color.
    pub accent_bleed: bool,
    /// Slide a single highlight layer between rows instead of a static per-row one.
    pub slide_selection: bool,
    /// Film-grain noise overlay opacity (0.0 = off .. 0.25 = strong).
    pub grain: f32,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "warm-dark".to_string(),
            font_size: 13,
            animate_results: ResultAnimation::Slide,
            show_metadata: true,
            accent_bleed: true,
            slide_selection: true,
            grain: 0.07,
        }
    }
}

// ── shared runtime config ─────────────────────────────────────────────────────

/// Per-search scalars shared across all indexed providers.
/// Updated in-place on config reload; providers read it per search() call.
#[derive(Debug, Clone)]
pub struct SharedSearchConfig {
    pub min_quality: f32,
    pub show_dotfiles: bool,
    pub log_scores: bool,
    pub log_watcher: bool,
    pub log_pdf: bool,
}

pub type SharedConfig = Arc<RwLock<SharedSearchConfig>>;

impl SharedSearchConfig {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            min_quality: cfg.search.min_quality,
            show_dotfiles: cfg.files.show_dotfiles,
            log_scores: cfg.debug.log_scores,
            log_watcher: cfg.debug.log_watcher,
            log_pdf: cfg.debug.log_pdf,
        }
    }

    pub fn update_from(&mut self, cfg: &Config) {
        self.min_quality = cfg.search.min_quality;
        self.show_dotfiles = cfg.files.show_dotfiles;
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
                // Return the parsed bundled config, not Self::default(): the two
                // diverge (e.g. content dirs), so returning defaults would make the
                // first session behave differently from every restart afterwards.
                return toml::from_str::<Config>(DEFAULT_CONFIG).unwrap_or_default();
            }
        };
        match toml::from_str::<Config>(&content) {
            Ok(mut cfg) => {
                // Pre-v2 configs stored `[extensions.enabled] name = bool`; under
                // the per-extension map that parses as a junk extension literally
                // named "enabled". Drop it leniently (an extension can't be named
                // "enabled" without colliding here anyway) - never fail the whole
                // config over the old shape.
                if cfg.extensions.remove("enabled").is_some() {
                    eprintln!(
                        "[config] dropped pre-v2 [extensions.enabled] table - re-enable extensions in Settings"
                    );
                }
                eprintln!("[config] loaded from {}", path.display());
                cfg
            }
            Err(e) => {
                eprintln!("[config] failed to parse {}: {e} - using defaults", path.display());
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
            PathBuf::from(crate::paths::home()).join(rest)
        } else if p == "~" {
            PathBuf::from(crate::paths::home())
        } else {
            PathBuf::from(p)
        }
    }
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// `$XDG_CONFIG_HOME/portunus` (or `~/.config/portunus`). Holds `config.toml` and
/// the external `matugen.css` theme file.
pub fn config_dir() -> PathBuf {
    crate::paths::config_dir()
}

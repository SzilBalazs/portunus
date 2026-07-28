//! One home for the `$HOME` / XDG base-directory lookups that were previously
//! duplicated across a dozen modules. Each helper preserves the exact fallback
//! behavior of the sites it replaces (`std::env::var` semantics: an explicitly
//! empty `$XDG_*` is honored as-is, only an unset var falls back).

use std::path::PathBuf;

/// `$HOME`, falling back to `/root` to match the historical call sites.
pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

/// `$XDG_CONFIG_HOME`, or `~/.config` when unset.
pub fn xdg_config_home() -> PathBuf {
    let dir = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{}/.config", home()));
    PathBuf::from(dir)
}

/// `$XDG_DATA_HOME`, or `~/.local/share` when unset.
pub fn xdg_data_home() -> PathBuf {
    let dir = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home()));
    PathBuf::from(dir)
}

/// `$XDG_DATA_HOME` followed by the `$XDG_DATA_DIRS` entries, in lookup order.
/// Used for both `applications/` (.desktop files) and `icons/` (theme roots).
pub fn xdg_data_dirs() -> Vec<PathBuf> {
    let system_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());

    let mut dirs = vec![xdg_data_home()];
    dirs.extend(system_dirs.split(':').map(PathBuf::from));
    dirs
}

/// `$XDG_RUNTIME_DIR`, or `/tmp` when unset.
pub fn xdg_runtime_dir() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(dir)
}

/// `$XDG_CONFIG_HOME/portunus` (or `~/.config/portunus`). Holds `config.toml`
/// and the external `matugen.css` theme file.
pub fn config_dir() -> PathBuf {
    xdg_config_home().join("portunus")
}

/// `$XDG_DATA_HOME/portunus` (or `~/.local/share/portunus`). Holds the SQLite
/// databases, the content index, and the extensions directory.
pub fn data_dir() -> PathBuf {
    xdg_data_home().join("portunus")
}

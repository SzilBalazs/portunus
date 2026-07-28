//! XDG icon-theme lookup for the app provider.
//!
//! The predecessor of this module scanned a hardcoded list of `<size>/apps`
//! directories across every installed theme and picked whichever file happened
//! to have the largest pixel size. That misses three whole classes of icon:
//! names that live outside the `apps` context (`Icon=x-office-calendar`),
//! themes using the KDE `<context>/<size>` layout (breeze), and every size or
//! `symbolic` directory not in the hardcoded list (modern Adwaita ships app
//! icons *only* under `symbolic/`). It also had no notion of the configured
//! theme, so the winning file came from an arbitrary theme.
//!
//! Instead we read each theme's own `index.theme`, which declares its
//! directories together with their size, scale and context - so odd layouts and
//! unusual sizes need no special-casing - and walk the `Inherits` chain in rank
//! order. Theme rank dominates the ordering: the configured theme is exhausted
//! at any size before an inherited theme is consulted, which is what keeps the
//! launcher looking like one theme instead of a collage.
//!
//! Lookup is a single pass that enumerates the chain's directories and keeps
//! only the names the caller asked for. Enumerating beats probing per name
//! here: the chain is ~900 directories deep on a Papirus/breeze/hicolor stack,
//! so per-name probing would cost hundreds of thousands of `stat` calls, while
//! one `read_dir` per directory needs no `stat` at all.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Logical size we aim for. Result rows render icons at ~28 px, so 48 leaves
/// headroom for HiDPI without pulling in needlessly heavy 512 px artwork.
const TARGET_SIZE: u32 = 48;

/// Hard cap on the inherit chain. Guards against a pathological `Inherits`
/// graph; real themes are 2-4 deep.
const MAX_CHAIN: usize = 16;

/// One directory of one theme, already resolved to an absolute path.
struct ThemeDir {
    path: PathBuf,
    /// Position in the flattened lookup order. Lower wins.
    rank: usize,
}

/// Sort key for a candidate directory, most significant field first. Theme rank
/// leads, so a 22 px icon from the configured theme beats a 512 px icon from an
/// inherited one; `symbolic` sits directly under it, so every colour icon in a
/// theme (whatever its context) wins over that theme's monochrome glyphs.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct DirKey {
    theme_rank: usize,
    /// false sorts first: colour before monochrome.
    symbolic: bool,
    /// false sorts first, so `Context=Applications` leads. Keeps `firefox` in
    /// `apps/` ahead of an unrelated same-named icon in `mimetypes/`.
    not_app_context: bool,
    /// false sorts first: vector artwork before fixed-size bitmaps.
    not_scalable: bool,
    /// Distance from `TARGET_SIZE`, in logical pixels.
    size_distance: u32,
    /// 1x before 2x, so `@2x` duplicates only matter when nothing else has the name.
    scale: u32,
}

/// A parsed `index.theme`: the directories it declares plus the themes it falls
/// back to.
struct ThemeMeta {
    dirs: Vec<(PathBuf, DirKey)>,
    inherits: Vec<String>,
}

pub struct IconResolver {
    /// Flattened lookup order across the whole inherit chain.
    dirs: Vec<ThemeDir>,
    /// Name of the theme the chain starts at, for diagnostics.
    theme: String,
}

// ── theme detection ───────────────────────────────────────────────────────────

/// The icon-theme roots, in XDG lookup order. `~/.icons` is legacy but still
/// where several theme installers write.
fn icon_roots() -> Vec<PathBuf> {
    let mut roots = vec![crate::paths::xdg_data_home().join("icons")];
    roots.push(PathBuf::from(crate::paths::home()).join(".icons"));
    roots.extend(
        crate::paths::xdg_data_dirs()
            .into_iter()
            .skip(1) // xdg_data_home()/icons is already first
            .map(|d| d.join("icons")),
    );
    roots
}

/// Read `gtk-icon-theme-name` out of a GTK `settings.ini`.
fn gtk_theme(file: &Path) -> Option<String> {
    let content = fs::read_to_string(file).ok()?;
    content.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == "gtk-icon-theme-name").then(|| v.trim().to_string())
    })
}

/// Ask GNOME's settings daemon. Only reached when neither GTK ini exists, so
/// non-GNOME systems never pay for the subprocess.
fn gsettings_theme() -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_matches('\'')
        .to_string();
    (!name.is_empty()).then_some(name)
}

/// Configured icon theme: explicit config first, then the desktop's own
/// setting, then `hicolor` (which every compliant theme inherits anyway).
fn detect_theme(preferred: Option<&str>) -> String {
    if let Some(name) = preferred.map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    let cfg = crate::paths::xdg_config_home();
    gtk_theme(&cfg.join("gtk-4.0/settings.ini"))
        .or_else(|| gtk_theme(&cfg.join("gtk-3.0/settings.ini")))
        .or_else(gsettings_theme)
        .unwrap_or_else(|| "hicolor".to_string())
}

/// Every installed theme name, for the Settings dropdown. A directory counts as
/// a theme only if it carries an `index.theme`.
pub fn installed_themes() -> Vec<String> {
    let mut names: Vec<String> = icon_roots()
        .iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("index.theme").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names.dedup();
    names
}

// ── index.theme parsing ───────────────────────────────────────────────────────

/// A section name is used as a path relative to the theme root, so it must not
/// be able to escape it.
fn safe_subdir(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() {
        return None;
    }
    p.components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
        .then(|| p.to_path_buf())
}

/// Parse one theme's `index.theme`. Sections other than `[Icon Theme]` are
/// directory declarations; we take every one that exists on disk rather than
/// only those listed in `Directories=`, since some themes ship an incomplete
/// list.
fn parse_index_theme(root: &Path, theme_rank: usize) -> Option<ThemeMeta> {
    let content = fs::read_to_string(root.join("index.theme")).ok()?;

    /// Which section the parser is inside. `Ignored` covers a header that
    /// failed `safe_subdir`, so its fields are dropped rather than attributed to
    /// `[Icon Theme]` or the previous directory.
    enum Section {
        Meta,
        Dir(PathBuf),
        Ignored,
    }

    let mut dirs = Vec::new();
    let mut inherits = Vec::new();
    let mut section = Section::Meta;
    let mut fields: HashMap<String, String> = HashMap::new();

    // Flush the fields collected for `section` into a directory declaration.
    let mut flush = |section: &Section, fields: &mut HashMap<String, String>| {
        let Section::Dir(subdir) = section else {
            fields.clear();
            return;
        };
        let path = root.join(subdir);
        if !path.is_dir() {
            fields.clear();
            return;
        }
        let size: u32 = fields
            .get("Size")
            .and_then(|s| s.parse().ok())
            .unwrap_or(TARGET_SIZE);
        let scale: u32 = fields
            .get("Scale")
            .and_then(|s| s.parse().ok())
            .filter(|s| *s > 0)
            .unwrap_or(1);
        let ty = fields.get("Type").map(String::as_str).unwrap_or("Threshold");
        // A directory literally named `scalable` holds vectors even when the
        // theme forgets to say `Type=Scalable`.
        let named_scalable = subdir
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("scalable"));
        let symbolic = subdir
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("symbolic"));
        let key = DirKey {
            theme_rank,
            symbolic,
            not_app_context: fields.get("Context").map(String::as_str) != Some("Applications"),
            not_scalable: !(ty == "Scalable" || named_scalable),
            size_distance: size.abs_diff(TARGET_SIZE),
            scale,
        };
        dirs.push((path, key));
        fields.clear();
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&section, &mut fields);
            section = match header {
                "Icon Theme" => Section::Meta,
                name => safe_subdir(name).map_or(Section::Ignored, Section::Dir),
            };
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if matches!(section, Section::Meta) && k == "Inherits" {
                inherits.extend(
                    v.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                );
            }
            fields.insert(k.to_string(), v.to_string());
        }
    }
    flush(&section, &mut fields);

    Some(ThemeMeta { dirs, inherits })
}

// ── chain construction ────────────────────────────────────────────────────────

impl IconResolver {
    /// Build the lookup order for `preferred` (or the auto-detected theme).
    pub fn new(preferred: Option<&str>) -> Self {
        let theme = detect_theme(preferred);
        let roots = icon_roots();

        let mut queue: Vec<String> = vec![theme.clone()];
        let mut visited: HashSet<String> = HashSet::new();
        let mut dirs: Vec<(PathBuf, DirKey)> = Vec::new();
        let mut rank = 0usize;
        let mut hicolor_seen = false;

        while let Some(name) = (rank < MAX_CHAIN).then(|| queue.first().cloned()).flatten() {
            queue.remove(0);
            if !visited.insert(name.clone()) {
                continue;
            }
            if name == "hicolor" {
                hicolor_seen = true;
            }
            // A theme can be split across roots (user override plus system
            // install); every root that has it contributes at this rank.
            let mut found = false;
            for root in &roots {
                let dir = root.join(&name);
                if let Some(meta) = parse_index_theme(&dir, rank) {
                    dirs.extend(meta.dirs);
                    queue.extend(meta.inherits);
                    found = true;
                }
            }
            if found {
                rank += 1;
            }
        }

        // Every theme is required to inherit hicolor; add it explicitly for the
        // ones that forget, so app-shipped icons remain reachable.
        if !hicolor_seen {
            for root in &roots {
                if let Some(meta) = parse_index_theme(&root.join("hicolor"), rank) {
                    dirs.extend(meta.dirs);
                }
            }
            rank += 1;
        }

        // `/usr/share/pixmaps` is the spec's last resort: unthemed, unsized.
        for base in crate::paths::xdg_data_dirs() {
            let pixmaps = base.join("pixmaps");
            if pixmaps.is_dir() {
                dirs.push((
                    pixmaps,
                    DirKey {
                        theme_rank: rank,
                        symbolic: false,
                        not_app_context: false,
                        not_scalable: true,
                        size_distance: 0,
                        scale: 1,
                    },
                ));
            }
        }

        dirs.sort_by(|a, b| a.1.cmp(&b.1));
        let dirs = dirs
            .into_iter()
            .enumerate()
            .map(|(rank, (path, _))| ThemeDir { path, rank })
            .collect();

        Self { dirs, theme }
    }

    pub fn theme(&self) -> &str {
        &self.theme
    }

    /// Resolve every name in `wanted` in one pass over the chain.
    ///
    /// Returns absolute, canonicalized paths keyed by icon name. Only wanted
    /// names are retained, so memory stays proportional to the app list rather
    /// than to the ~60k files a large theme stack contains.
    pub fn resolve_all(&self, wanted: &HashSet<String>) -> HashMap<String, String> {
        // name -> (dir rank, format rank, path). Lower ranks win.
        let mut best: HashMap<&str, (usize, u8, PathBuf)> = HashMap::new();

        for dir in &self.dirs {
            let Ok(entries) = fs::read_dir(&dir.path) else {
                continue;
            };
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                // Vector first: an SVG is correct at every render size.
                let fmt_rank: u8 = match path.extension().and_then(|e| e.to_str()) {
                    Some("svg") => 0,
                    Some("png") => 1,
                    Some("xpm") => 2,
                    _ => continue,
                };
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Some(name) = wanted.get(stem) else {
                    continue;
                };
                let cand = (dir.rank, fmt_rank);
                match best.get(name.as_str()) {
                    Some((r, f, _)) if (*r, *f) <= cand => {}
                    _ => {
                        best.insert(name.as_str(), (cand.0, cand.1, path));
                    }
                }
            }
        }

        best.into_iter()
            .map(|(name, (_, _, path))| {
                // Canonicalize the winner only: theme files are frequently
                // symlinks (Papirus-Dark into Papirus) and the asset protocol
                // wants a real path.
                let resolved = fs::canonicalize(&path).unwrap_or(path);
                (name.to_string(), resolved.to_string_lossy().into_owned())
            })
            .collect()
    }
}

/// Strip a trailing image extension from an `Icon=` value. The spec says the
/// field is a bare name, but entries in the wild ship `Icon=foo.png`.
pub fn strip_icon_extension(name: &str) -> &str {
    for ext in [".png", ".svg", ".xpm"] {
        if let Some(base) = name.strip_suffix(ext) {
            return base;
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a theme with the given `index.theme` body and directory list.
    fn write_theme(root: &Path, body: &str, subdirs: &[&str]) {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("index.theme"), body).unwrap();
        for d in subdirs {
            fs::create_dir_all(root.join(d)).unwrap();
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("portunus-icon-test-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_hicolor_layout() {
        let root = tmpdir("hicolor-layout").join("themed");
        write_theme(
            &root,
            "[Icon Theme]\nName=Themed\nInherits=hicolor\n\n\
             [48x48/apps]\nSize=48\nContext=Applications\nType=Fixed\n\n\
             [scalable/apps]\nSize=48\nContext=Applications\nType=Scalable\n",
            &["48x48/apps", "scalable/apps"],
        );

        let meta = parse_index_theme(&root, 0).unwrap();
        assert_eq!(meta.inherits, vec!["hicolor"]);
        assert_eq!(meta.dirs.len(), 2);
        // Scalable sorts ahead of the fixed-size directory at equal size.
        let mut dirs = meta.dirs;
        dirs.sort_by(|a, b| a.1.cmp(&b.1));
        assert!(dirs[0].0.ends_with("scalable/apps"));
    }

    /// breeze puts the size *under* the context; the section name carries the
    /// layout so nothing needs to know about it.
    #[test]
    fn parses_category_first_layout() {
        let root = tmpdir("kde-layout").join("kdeish");
        write_theme(
            &root,
            "[Icon Theme]\nName=Kdeish\n\n\
             [apps/48]\nSize=48\nContext=Applications\nType=Scalable\nMinSize=48\n\n\
             [mimetypes/22]\nSize=22\nContext=MimeTypes\nType=Scalable\n",
            &["apps/48", "mimetypes/22"],
        );

        let meta = parse_index_theme(&root, 0).unwrap();
        assert_eq!(meta.dirs.len(), 2);
        let mut dirs = meta.dirs;
        dirs.sort_by(|a, b| a.1.cmp(&b.1));
        assert!(dirs[0].0.ends_with("apps/48"));
        assert!(dirs[1].0.ends_with("mimetypes/22"));
    }

    /// Declared-but-absent directories must not enter the lookup order, or
    /// every miss would pay a `read_dir` for them.
    #[test]
    fn skips_absent_directories() {
        let root = tmpdir("absent").join("sparse");
        write_theme(
            &root,
            "[Icon Theme]\nName=Sparse\n\n\
             [48x48/apps]\nSize=48\nContext=Applications\n\n\
             [512x512/apps]\nSize=512\nContext=Applications\n",
            &["48x48/apps"],
        );

        let meta = parse_index_theme(&root, 0).unwrap();
        assert_eq!(meta.dirs.len(), 1);
    }

    #[test]
    fn rejects_escaping_section_names() {
        assert!(safe_subdir("../../etc").is_none());
        assert!(safe_subdir("/etc/passwd").is_none());
        assert!(safe_subdir("48x48/apps").is_some());
    }

    /// Colour beats monochrome inside a theme, but a symbolic icon from the
    /// configured theme still beats anything inherited.
    #[test]
    fn symbolic_ranks_last_within_a_theme() {
        let colour = DirKey {
            theme_rank: 0,
            symbolic: false,
            not_app_context: true,
            not_scalable: true,
            size_distance: 32,
            scale: 1,
        };
        let symbolic = DirKey {
            theme_rank: 0,
            symbolic: true,
            not_app_context: false,
            not_scalable: false,
            size_distance: 0,
            scale: 1,
        };
        let inherited = DirKey {
            theme_rank: 1,
            symbolic: false,
            not_app_context: false,
            not_scalable: false,
            size_distance: 0,
            scale: 1,
        };
        assert!(colour < symbolic);
        assert!(symbolic < inherited);
    }

    #[test]
    fn strips_icon_extensions() {
        assert_eq!(strip_icon_extension("firefox.png"), "firefox");
        assert_eq!(strip_icon_extension("firefox"), "firefox");
        assert_eq!(strip_icon_extension("x-office-calendar"), "x-office-calendar");
    }

    /// An `Inherits` cycle must terminate.
    #[test]
    fn inherit_cycles_terminate() {
        let base = tmpdir("cycle");
        write_theme(
            &base.join("a"),
            "[Icon Theme]\nInherits=b\n\n[48x48/apps]\nSize=48\nContext=Applications\n",
            &["48x48/apps"],
        );
        write_theme(
            &base.join("b"),
            "[Icon Theme]\nInherits=a\n\n[48x48/apps]\nSize=48\nContext=Applications\n",
            &["48x48/apps"],
        );

        // Exercise the BFS directly against the temp root rather than the real
        // XDG dirs, which the test process must not depend on.
        let mut queue = vec!["a".to_string()];
        let mut visited: HashSet<String> = HashSet::new();
        let mut rank = 0usize;
        while let Some(name) = (rank < MAX_CHAIN).then(|| queue.first().cloned()).flatten() {
            queue.remove(0);
            if !visited.insert(name.clone()) {
                continue;
            }
            if let Some(meta) = parse_index_theme(&base.join(&name), rank) {
                queue.extend(meta.inherits);
                rank += 1;
            }
        }
        assert_eq!(rank, 2);
    }
}

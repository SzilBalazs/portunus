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

/// Logical size we aim for: the preview hero renders at 96 CSS px, which is the
/// largest consumer and wants 2x that on a HiDPI display. 128 covers it without
/// routinely pulling in 512 px artwork; `size_distance` below then does the
/// real work of preferring an oversized file to an undersized one.
const TARGET_SIZE: u32 = 128;

/// How much worse an undersized icon is than an oversized one, per pixel of
/// shortfall. Downscaling is free; upscaling is visibly soft, so a bitmap-only
/// theme (Steam writes 16-96 px PNGs and nothing else) must reach for its
/// largest file rather than tie-break onto a 32 px one. 5 is the smallest
/// weight that settles the 96-vs-256 pair - the one real themes actually
/// present - in favour of the sharp option instead of leaving it a tie.
const UNDERSIZE_PENALTY: u32 = 5;

/// Distance from `TARGET_SIZE`, with shortfall weighted `UNDERSIZE_PENALTY`x.
fn size_distance(size: u32) -> u32 {
    match size.checked_sub(TARGET_SIZE) {
        Some(over) => over,
        None => (TARGET_SIZE - size).saturating_mul(UNDERSIZE_PENALTY),
    }
}

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
    /// Distance from `TARGET_SIZE`, undersize-weighted. See `size_distance`.
    size_distance: u32,
    /// 1x before 2x, so `@2x` duplicates only matter when nothing else has the name.
    scale: u32,
}

/// One directory a theme declares, before it is bound to a base directory. The
/// spec allows a theme to be split across roots with `index.theme` present in
/// only one of them, so the declaration and the root it lives under are
/// resolved separately.
struct DirDecl {
    subdir: PathBuf,
    size: u32,
    scale: u32,
    /// `Type=Scalable`, or a path component literally named `scalable` for
    /// themes that forget to say so.
    scalable: bool,
    app_context: bool,
}

impl DirDecl {
    fn key(&self, theme_rank: usize) -> DirKey {
        DirKey {
            theme_rank,
            symbolic: self
                .subdir
                .components()
                .any(|c| c.as_os_str().eq_ignore_ascii_case("symbolic")),
            not_app_context: !self.app_context,
            not_scalable: !self.scalable,
            size_distance: size_distance(self.size),
            scale: self.scale,
        }
    }
}

/// A parsed `index.theme`: the directories it declares plus the themes it falls
/// back to.
struct ThemeMeta {
    dirs: Vec<DirDecl>,
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
/// directory declarations; we take every one rather than only those listed in
/// `Directories=`, since some themes ship an incomplete list. Whether a
/// declared directory exists is decided later, once per base directory.
fn parse_index_theme(root: &Path) -> Option<ThemeMeta> {
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
        dirs.push(DirDecl {
            subdir: subdir.clone(),
            size,
            scale,
            scalable: ty == "Scalable" || named_scalable,
            app_context: fields.get("Context").map(String::as_str) == Some("Applications"),
        });
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

// ── directories without an index.theme ────────────────────────────────────────

/// Logical size encoded in a directory name: `48x48`, `48`, or nothing.
fn size_from_component(name: &str) -> Option<u32> {
    name.split_once('x').map_or(name, |(head, _)| head).parse().ok()
}

/// Derive directory declarations by walking a theme root two levels deep, for
/// theme trees that ship no `index.theme` at all - the layout Steam creates
/// under `~/.local/share/icons/hicolor` when it installs per-game icons. Both
/// `<size>/<context>` and `<context>/<size>` orders are handled by taking
/// whichever component parses as a size.
fn synthesize_index(root: &Path) -> Vec<DirDecl> {
    let mut out = Vec::new();
    let Ok(level1) = fs::read_dir(root) else {
        return out;
    };
    for outer in level1.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
        let Ok(name_a) = outer.file_name().into_string() else {
            continue;
        };
        let Ok(level2) = fs::read_dir(outer.path()) else {
            continue;
        };
        for inner in level2.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
            let Ok(name_b) = inner.file_name().into_string() else {
                continue;
            };
            let (outer_size, inner_size) =
                (size_from_component(&name_a), size_from_component(&name_b));
            // Only the `<context>/<size>` order puts the context outermost;
            // `scalable/apps` carries no size at all and reads like the first.
            let context = if inner_size.is_some() { &name_a } else { &name_b };
            out.push(DirDecl {
                subdir: Path::new(&name_a).join(&name_b),
                size: outer_size.or(inner_size).unwrap_or(TARGET_SIZE),
                scale: 1,
                scalable: name_a.eq_ignore_ascii_case("scalable")
                    || name_b.eq_ignore_ascii_case("scalable"),
                app_context: context.eq_ignore_ascii_case("apps"),
            });
        }
    }
    out
}

/// Bind one theme's declarations to every base directory that carries it, and
/// append them to `dirs` at `theme_rank`. Returns the themes it inherits, or
/// `None` if no root has the theme at all.
///
/// A theme may be split across roots (a user override plus the system install)
/// with `index.theme` present in only one of them, so the declaration list is
/// read once and applied to all of them - a per-root parse would drop whichever
/// roots lack the file.
fn collect_theme(
    roots: &[PathBuf],
    name: &str,
    theme_rank: usize,
    dirs: &mut Vec<(PathBuf, DirKey)>,
) -> Option<Vec<String>> {
    let bases: Vec<PathBuf> = roots
        .iter()
        .map(|r| r.join(name))
        .filter(|p| p.is_dir())
        .collect();
    if bases.is_empty() {
        return None;
    }

    let (decls, inherits) = match bases.iter().find_map(|b| parse_index_theme(b)) {
        Some(meta) => (meta.dirs, meta.inherits),
        None => {
            // Same subdir can appear under several roots; declare it once and
            // let the binding loop below pick up every root that has it.
            let mut seen = HashSet::new();
            let mut decls: Vec<DirDecl> = bases.iter().flat_map(|b| synthesize_index(b)).collect();
            decls.retain(|d| seen.insert(d.subdir.clone()));
            (decls, Vec::new())
        }
    };

    for base in &bases {
        for decl in &decls {
            let path = base.join(&decl.subdir);
            // Declared-but-absent directories must not enter the lookup order,
            // or every miss would pay a `read_dir` for them.
            if path.is_dir() {
                dirs.push((path, decl.key(theme_rank)));
            }
        }
    }
    Some(inherits)
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
            if let Some(inherits) = collect_theme(&roots, &name, rank, &mut dirs) {
                queue.extend(inherits);
                rank += 1;
            }
        }

        // Every theme is required to inherit hicolor; add it explicitly for the
        // ones that forget, so app-shipped icons remain reachable.
        if !hicolor_seen {
            collect_theme(&roots, "hicolor", rank, &mut dirs);
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

        let meta = parse_index_theme(&root).unwrap();
        assert_eq!(meta.inherits, vec!["hicolor"]);
        assert_eq!(meta.dirs.len(), 2);
        // Scalable sorts ahead of the fixed-size directory at equal size.
        let mut dirs = meta.dirs;
        dirs.sort_by_key(|d| d.key(0));
        assert!(dirs[0].subdir.ends_with("scalable/apps"));
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

        let meta = parse_index_theme(&root).unwrap();
        assert_eq!(meta.dirs.len(), 2);
        let mut dirs = meta.dirs;
        dirs.sort_by_key(|d| d.key(0));
        assert!(dirs[0].subdir.ends_with("apps/48"));
        assert!(dirs[1].subdir.ends_with("mimetypes/22"));
    }

    /// Declared-but-absent directories must not enter the lookup order, or
    /// every miss would pay a `read_dir` for them.
    #[test]
    fn skips_absent_directories() {
        let base = tmpdir("absent");
        write_theme(
            &base.join("sparse"),
            "[Icon Theme]\nName=Sparse\n\n\
             [48x48/apps]\nSize=48\nContext=Applications\n\n\
             [512x512/apps]\nSize=512\nContext=Applications\n",
            &["48x48/apps"],
        );

        let mut dirs = Vec::new();
        collect_theme(&[base], "sparse", 0, &mut dirs).unwrap();
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].0.ends_with("48x48/apps"));
    }

    /// The spec puts `index.theme` in one base directory only; the directories
    /// it declares must still be picked up from every other root that has the
    /// theme.
    #[test]
    fn binds_declarations_to_every_root() {
        let base = tmpdir("split");
        let (user, system) = (base.join("user"), base.join("system"));
        write_theme(
            &system.join("hicolor"),
            "[Icon Theme]\nName=Hicolor\n\n[32x32/apps]\nSize=32\nContext=Applications\n",
            &["32x32/apps"],
        );
        // Steam's layout: icon directories, no index.theme of their own.
        fs::create_dir_all(user.join("hicolor/32x32/apps")).unwrap();

        let mut dirs = Vec::new();
        collect_theme(&[user.clone(), system.clone()], "hicolor", 0, &mut dirs).unwrap();
        assert_eq!(dirs.len(), 2);
        assert!(dirs.iter().any(|(p, _)| p.starts_with(&user)));
        assert!(dirs.iter().any(|(p, _)| p.starts_with(&system)));
    }

    /// A theme tree with no `index.theme` anywhere still yields directories,
    /// derived from the directory names.
    #[test]
    fn synthesizes_directories_without_an_index() {
        let base = tmpdir("no-index");
        for d in ["hicolor/32x32/apps", "hicolor/scalable/apps", "hicolor/16x16/mimetypes"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }

        let mut dirs = Vec::new();
        let inherits = collect_theme(&[base], "hicolor", 0, &mut dirs).unwrap();
        assert!(inherits.is_empty());
        assert_eq!(dirs.len(), 3);
        dirs.sort_by(|a, b| a.1.cmp(&b.1));
        // scalable/apps first (vector, app context), then 32x32/apps, then the
        // non-app context.
        assert!(dirs[0].0.ends_with("scalable/apps"));
        assert!(dirs[1].0.ends_with("32x32/apps"));
        assert!(dirs[2].0.ends_with("16x16/mimetypes"));
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

    /// A bitmap-only theme must reach for its largest file rather than tie-break
    /// onto a small one - Steam ships 16-96 px PNGs and the preview renders at
    /// 96 CSS px.
    #[test]
    fn prefers_oversize_to_undersize() {
        let decl = |size| DirDecl {
            subdir: PathBuf::from(format!("{size}x{size}/apps")),
            size,
            scale: 1,
            scalable: false,
            app_context: true,
        };
        let mut sizes = vec![16, 24, 32, 64, 96, 256];
        sizes.sort_by_key(|s| decl(*s).key(0));
        assert_eq!(sizes[0], 256, "an oversized icon downscales cleanly");
        assert_eq!(sizes[1], 96, "then the largest that fits under the target");

        // Without the undersize weighting these two would tie at distance 32.
        assert!(size_distance(160) < size_distance(96));
        // Steam's own spread, with no oversized file to fall back to.
        let mut steam = vec![16, 24, 32, 64, 96];
        steam.sort_by_key(|s| decl(*s).key(0));
        assert_eq!(steam[0], 96);
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
            if let Some(meta) = parse_index_theme(&base.join(&name)) {
                queue.extend(meta.inherits);
                rank += 1;
            }
        }
        assert_eq!(rank, 2);
    }
}

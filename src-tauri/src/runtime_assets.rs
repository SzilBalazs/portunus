//! Resolves the optional native assets that a packaged build (the AppImage)
//! bundles next to the binary: the pdfium library, the poppler command-line
//! tools, and the tesseract language data.
//!
//! When a bundled copy is present we use it, so PDF preview, PDF content
//! extraction, and OCR all work out of the box with nothing installed on the
//! host. When it is absent (a plain `cargo`/source build) every lookup falls
//! back to the system: pdfium binds to the system library, the poppler tools
//! resolve through `PATH`, and leptess reads the system tessdata. Callers keep
//! their existing "missing means degrade gracefully" behaviour either way.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

static RESOURCE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Records the Tauri resource directory. Call once during app setup. Passing
/// `None` (resource dir unavailable) leaves every lookup on the system path.
pub fn init(resource_dir: Option<PathBuf>) {
    let _ = RESOURCE_DIR.set(resource_dir);
}

/// Path to a bundled asset under the resource dir, but only if it exists on
/// disk. Returns `None` before `init` runs or for a source build with no
/// bundled resources.
fn bundled(rel: &str) -> Option<PathBuf> {
    let dir = RESOURCE_DIR.get()?.as_ref()?;
    let path = dir.join(rel);
    path.exists().then_some(path)
}

/// Directory holding `<lang>.traineddata` for leptess. Returns the bundled
/// tessdata dir only when it contains every requested language (`+`-separated,
/// e.g. `eng+hun`); otherwise `None`, so leptess falls back to the system
/// tessdata (via `TESSDATA_PREFIX` / default paths) where user-installed
/// languages live. The bundled dir ships English only, so any extra language
/// resolves against the system install.
pub fn tessdata_path(lang: &str) -> Option<String> {
    let dir = bundled("tessdata")?;
    let has_all = lang
        .split('+')
        .all(|l| dir.join(format!("{l}.traineddata")).exists());
    has_all.then(|| dir.to_string_lossy().into_owned())
}

/// Bundled `libpdfium.so` path for `Pdfium::bind_to_library`, if present.
pub fn pdfium_library() -> Option<PathBuf> {
    bundled("libpdfium.so")
}

/// Builds a `Command` for a poppler tool (`pdftotext`, `pdftoppm`). Uses the
/// bundled binary when present (pointing `LD_LIBRARY_PATH` at the bundled libs
/// so it finds libpoppler and friends), otherwise resolves the bare name
/// through PATH.
pub fn poppler_command(name: &str) -> Command {
    match bundled(&format!("bin/{name}")) {
        Some(path) => {
            let mut cmd = Command::new(path);
            if let Some(libdir) = bundled("lib") {
                let mut value = libdir.into_os_string();
                if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
                    value.push(":");
                    value.push(existing);
                }
                cmd.env("LD_LIBRARY_PATH", value);
            }
            cmd
        }
        None => Command::new(name),
    }
}

/// Whether the poppler tools ship bundled with this build.
pub fn poppler_bundled() -> bool {
    bundled("bin/pdftotext").is_some()
}

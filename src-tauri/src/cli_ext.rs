//! `portunus ext …` - extension developer CLI.
//!
//! Subcommands (hand-rolled parsing, matching the flag style in `cli.rs`):
//!   new <name>       scaffold a new extension project
//!   dev <dir>        symlink into the extensions dir + auto-reload on rebuild
//!   validate <dir>   manifest lint + wasm export check (no instantiation)
//!   pack <dir>       build a .portext archive + print its sha256

use std::path::PathBuf;

use crate::extensions::{extensions_dir, manifest};

const TMPL_CARGO: &str = include_str!("../../templates/extension/Cargo.toml.tmpl");
const TMPL_MANIFEST: &str = include_str!("../../templates/extension/manifest.toml.tmpl");
const TMPL_LIB: &str = include_str!("../../templates/extension/lib.rs.tmpl");
const TMPL_CARGO_CONFIG: &str = include_str!("../../templates/extension/cargo-config.toml.tmpl");
const TMPL_README: &str = include_str!("../../templates/extension/README.md.tmpl");

/// Entry point for `portunus ext …`. Returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("new") => cmd_new(args.get(1).map(String::as_str)),
        Some("dev") => cmd_dev(args.get(1).map(String::as_str)),
        Some("validate") => cmd_validate(args.get(1).map(String::as_str)),
        Some("pack") => cmd_pack(args.get(1).map(String::as_str)),
        _ => {
            eprintln!(
                "USAGE:
  portunus ext new <name>       Scaffold a new extension project
  portunus ext dev <dir>        Link <dir> into Portunus and auto-reload on rebuild
  portunus ext validate <dir>   Check manifest + wasm exports
  portunus ext pack <dir>       Build <name>.portext and print its sha256"
            );
            2
        }
    }
}

fn fill(template: &str, name: &str) -> String {
    template
        .replace("{{name}}", name)
        .replace("{{crate_name}}", &name.replace('-', "_"))
        .replace("{{sdk_tag}}", concat!("v", env!("CARGO_PKG_VERSION")))
}

fn cmd_new(name: Option<&str>) -> i32 {
    let Some(name) = name else {
        eprintln!("usage: portunus ext new <name>");
        return 2;
    };
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        eprintln!("name may only contain lowercase ASCII letters, digits, '-' and '_'");
        return 2;
    }
    let dir = PathBuf::from(name);
    if dir.exists() {
        eprintln!("{name}: already exists");
        return 1;
    }
    let write = |rel: &str, tmpl: &str| -> std::io::Result<()> {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, fill(tmpl, name))
    };
    let result = write("Cargo.toml", TMPL_CARGO)
        .and_then(|_| write("manifest.toml", TMPL_MANIFEST))
        .and_then(|_| write("src/lib.rs", TMPL_LIB))
        .and_then(|_| write(".cargo/config.toml", TMPL_CARGO_CONFIG))
        .and_then(|_| write("README.md", TMPL_README))
        .and_then(|_| std::fs::write(dir.join(".gitignore"), "/target\n/extension.wasm\n"));
    if let Err(e) = result {
        eprintln!("failed to scaffold: {e}");
        return 1;
    }
    println!(
        "Created {name}/ - next steps:
  cd {name}
  rustup target add wasm32-unknown-unknown   # once
  cargo build --release
  cp target/wasm32-unknown-unknown/release/{}.wasm extension.wasm
  portunus ext dev .",
        name.replace('-', "_")
    );
    0
}

/// Resolves a directory argument to (canonical dir, manifest, wasm path).
fn load_dir(dir: Option<&str>) -> Result<(PathBuf, manifest::ExtensionManifest, PathBuf), String> {
    let dir = PathBuf::from(dir.unwrap_or("."));
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let (m, wasm) = manifest::load(&dir).map_err(|e| e + &missing_wasm_hint(&dir))?;
    Ok((dir, m, wasm))
}

/// When the manifest's wasm entry is absent but a built artifact exists under
/// target/, the fix is one cp away - say so instead of a bare ENOENT.
fn missing_wasm_hint(dir: &std::path::Path) -> String {
    let Ok((_, entry)) = read_manifest_entry(dir) else { return String::new() };
    if dir.join(&entry).exists() {
        return String::new();
    }
    for profile in ["release", "debug"] {
        let candidates = dir.join("target/wasm32-unknown-unknown").join(profile);
        let Ok(entries) = std::fs::read_dir(&candidates) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "wasm") {
                return format!(
                    "\n  built wasm found - copy it next to manifest.toml:\n    cp {} {}",
                    p.display(),
                    dir.join(&entry).display()
                );
            }
        }
    }
    format!(
        "\n  build it first:\n    cargo build --release --target wasm32-unknown-unknown\n    cp target/wasm32-unknown-unknown/release/<crate>.wasm {entry}"
    )
}

/// Just the manifest's name + entry fields, without the full validation.
fn read_manifest_entry(dir: &std::path::Path) -> Result<(String, String), ()> {
    let raw = std::fs::read_to_string(dir.join("manifest.toml")).map_err(|_| ())?;
    let v: toml::Value = toml::from_str(&raw).map_err(|_| ())?;
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string();
    let entry = v
        .get("entry")
        .and_then(|e| e.as_str())
        .unwrap_or("extension.wasm")
        .to_string();
    Ok((name, entry))
}

fn cmd_validate(dir: Option<&str>) -> i32 {
    let (_, m, wasm_path) = match load_dir(dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("manifest: FAIL - {e}");
            return 1;
        }
    };
    println!("manifest: ok ({} v{}, api {})", m.name, m.version, m.api);
    if m.trigger.is_none() {
        println!("note: no [trigger] section - the extension will run on every keystroke");
    }

    // Export scan only - never instantiate: a malicious module must not run
    // during validation.
    let bytes = match std::fs::read(&wasm_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: FAIL - {e}", wasm_path.display());
            return 1;
        }
    };
    let mut exports: Vec<String> = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        match payload {
            Ok(wasmparser::Payload::ExportSection(reader)) => {
                for export in reader {
                    match export {
                        Ok(e) if matches!(e.kind, wasmparser::ExternalKind::Func) => {
                            exports.push(e.name.to_string());
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("wasm: FAIL - {e}");
                            return 1;
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("wasm: FAIL - {e}");
                return 1;
            }
        }
    }
    let mut failed = false;
    for (export, required) in [("search", true), ("activate", false), ("preview", false), ("refresh", false)]
    {
        let present = exports.iter().any(|e| e == export);
        match (present, required) {
            (true, _) => println!("export {export}: ok"),
            (false, true) => {
                eprintln!("export {export}: MISSING (required)");
                failed = true;
            }
            (false, false) => println!("export {export}: absent (optional)"),
        }
    }
    if m.background.is_some() && !exports.iter().any(|e| e == "refresh") {
        eprintln!("warning: [background] declared but no refresh export");
    }
    if failed {
        1
    } else {
        println!("valid.");
        0
    }
}

fn cmd_dev(dir: Option<&str>) -> i32 {
    let (dir, m, _) = match load_dir(dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let name = m.name.clone();

    let link = extensions_dir().join(&name);
    if let Err(e) = std::fs::create_dir_all(extensions_dir()) {
        eprintln!("cannot create extensions dir: {e}");
        return 1;
    }
    match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.is_symlink() => {
            let _ = std::fs::remove_file(&link); // re-link (target may have moved)
        }
        Ok(_) => {
            eprintln!(
                "{} already exists and is not a symlink - remove it first (a real install would be clobbered)",
                link.display()
            );
            return 1;
        }
        Err(_) => {}
    }
    if let Err(e) = std::os::unix::fs::symlink(&dir, &link) {
        eprintln!("failed to link {}: {e}", link.display());
        return 1;
    }
    println!("linked {} -> {}", link.display(), dir.display());

    let signal = |name: &str| {
        if crate::ipc::try_signal_running(&format!("reload-extension:{name}")) {
            println!("[dev] reloaded {name}");
        } else {
            println!("[dev] Portunus not running - the extension loads on next start");
        }
    };
    signal(&name);
    println!("[dev] enable \"{name}\" once in Settings → Extensions, then rebuild to hot-reload (Ctrl-C to stop)");

    // Watch the whole directory, debounced: cargo/cp replace the wasm by
    // rename (a file watch would lose the inode) and manifest.toml edits
    // must retrigger too.
    use notify_debouncer_full::notify::{RecursiveMode, Watcher};
    use notify_debouncer_full::{new_debouncer, DebounceEventResult};
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = match new_debouncer(
        std::time::Duration::from_millis(400),
        None,
        move |result: DebounceEventResult| {
            if let Ok(events) = result {
                let relevant = events.iter().any(|e| {
                    e.paths.iter().any(|p| {
                        p.extension().is_some_and(|x| x == "wasm")
                            || p.file_name().is_some_and(|f| f == "manifest.toml")
                    })
                });
                if relevant {
                    let _ = tx.send(());
                }
            }
        },
    ) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to start watcher: {e}");
            return 1;
        }
    };
    // Non-recursive: target/ churn would spam the debouncer; the entry wasm
    // and manifest live at the top level.
    if let Err(e) = debouncer.watcher().watch(&dir, RecursiveMode::NonRecursive) {
        eprintln!("failed to watch {}: {e}", dir.display());
        return 1;
    }

    while rx.recv().is_ok() {
        // Coalesce bursts (editor save + cp) into one reload.
        while rx.try_recv().is_ok() {}
        signal(&name);
    }
    0
}

fn cmd_pack(dir: Option<&str>) -> i32 {
    let (dir, m, _) = match load_dir(dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match crate::extensions::install::pack(&dir, &m) {
        Ok((path, sha256)) => {
            println!("{}", path.display());
            println!("sha256: {sha256}");
            0
        }
        Err(e) => {
            eprintln!("pack failed: {e}");
            1
        }
    }
}

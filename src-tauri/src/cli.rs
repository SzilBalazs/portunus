use std::sync::Arc;

use crate::{config, content_index, ipc};

/// Handle CLI flags. Returns true if a flag was handled and the process should exit.
pub fn handle_cli_args() -> bool {
    let args: Vec<String> = std::env::args().collect();

    // `portunus ext …` - extension developer subcommands.
    if args.get(1).map(String::as_str) == Some("ext") {
        std::process::exit(crate::cli_ext::run(&args[2..]));
    }

    // `portunus native-host …` - browser native-messaging shim for the
    // extension message bus (spawned by the browser, or `install` by the user).
    if args.get(1).map(String::as_str) == Some("native-host") {
        std::process::exit(crate::native_host::run(&args[2..]));
    }

    // --reload-extension <name>: targeted hot-reload of one extension.
    if let Some(pos) = args.iter().position(|a| a == "--reload-extension") {
        let Some(name) = args.get(pos + 1) else {
            eprintln!("usage: portunus --reload-extension <name>");
            std::process::exit(2);
        };
        if !ipc::try_signal_running(&format!("reload-extension:{name}")) {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return true;
    }

    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("portunus {}", env!("CARGO_PKG_VERSION"));
        return true;
    }

    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("portunus {}: application launcher and power-user search for Linux

USAGE:
  portunus [FLAG]

FLAGS:
  --show              Show the launcher window (signals running instance)
  --clipboard         Show the launcher pre-filled with \"clipboard\"
  --reindex           Rebuild the content search index
  --reload-config     Reload config from file without restarting
  --reload-extensions Re-discover and reload WASM extensions (picks up rebuilt wasm)
  --reload-extension <name>
                      Reload a single extension (used by `portunus ext dev`)
  --reload-theme      Re-read the external matugen.css theme (matugen post_hook)
  --version, -V       Print version and exit
  --help, -h          Show this help message

SUBCOMMANDS:
  ext new <name>      Scaffold a new extension project
  ext dev <dir>       Link a working dir into Portunus + auto-reload on rebuild
  ext validate <dir>  Check an extension's manifest and wasm exports
  ext pack <dir>      Build a distributable .portext archive
  native-host <name>  Relay browser native messaging to the extension message
                      bus (normally spawned by the browser, not by hand)
  native-host install <name> --ff-ext-id <id@domain>
                      Write the wrapper script + Firefox manifest for <name>", env!("CARGO_PKG_VERSION"));
        return true;
    }

    if std::env::args().any(|a| a == "--show") {
        if !ipc::try_signal_running("show") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return true;
    }
    if std::env::args().any(|a| a == "--clipboard") {
        if !ipc::try_signal_running("show:clipboard ") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return true;
    }
    if std::env::args().any(|a| a == "--reload-config") {
        if !ipc::try_signal_running("reload-config") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return true;
    }
    if std::env::args().any(|a| a == "--reload-extensions") {
        if !ipc::try_signal_running("reload-extensions") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return true;
    }
    if std::env::args().any(|a| a == "--reload-theme") {
        if !ipc::try_signal_running("reload-theme") {
            eprintln!("portunus: no running instance found");
            std::process::exit(1);
        }
        return true;
    }
    if std::env::args().any(|a| a == "--reindex") {
        if !ipc::try_signal_running("reindex") {
            // No running instance - run standalone with stderr progress.
            let cfg = config::Config::load();
            if cfg.content.enabled {
                match content_index::ContentIndex::open() {
                    Ok(index) => {
                        let index = Arc::new(index);
                        index.clear().ok();
                        content_index::run_content_indexer(
                            Arc::clone(&index),
                            &cfg.content,
                            Some(Arc::new(|indexed, total| {
                                eprint!("\r[content] {indexed}/{total}");
                                if indexed >= total {
                                    eprintln!();
                                }
                            })),
                        );
                        eprintln!("[content] reindex complete");
                    }
                    Err(e) => eprintln!("[content] failed to open index: {e}"),
                }
            } else {
                eprintln!("[content] content indexing is disabled in config");
            }
        }
        return true;
    }

    false
}

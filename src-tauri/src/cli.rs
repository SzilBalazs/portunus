use std::sync::Arc;

use crate::{config, content_index, ipc};

/// Handle CLI flags. Returns true if a flag was handled and the process should exit.
pub fn handle_cli_args() -> bool {
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
  --version, -V       Print version and exit
  --help, -h          Show this help message", env!("CARGO_PKG_VERSION"));
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

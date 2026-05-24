# Portunus

macOS Spotlight-style app launcher for Linux (Hyprland).

## Tech Stack

| Layer | Tool |
|---|---|
| Frontend | React 19, TypeScript, Vite |
| Desktop | Tauri 2 |
| Package manager | Bun |
| Fuzzy matching | nucleo-matcher 0.3 |
| File traversal | walkdir 2 |
| Serialization | serde + serde_json |

## Commands

```bash
bun tauri dev       # dev mode (hot reload)
bun tauri build     # production build (requires linuxdeploy for AppImage)
cargo check         # type-check Rust only
```

## Structure

```
src/
  App.tsx           # Search UI, result list, keyboard nav
  App.css           # Dark card styles

src-tauri/
  tauri.conf.json   # Window (800×400, transparent, alwaysOnTop, center)
                    # assetProtocol enabled for icon dirs
  Cargo.toml        # protocol-asset feature required for assetProtocol
  capabilities/
    default.json    # ACL: core:default, core:window:allow-set-size

  src/
    main.rs         # Binary entry point (do not edit)
    lib.rs          # Tauri commands + setup:
                    #   search, launch_app, close_window
                    #   AppProvider loaded in background thread → emits "apps-ready"
    providers/
      mod.rs        # Provider trait, SearchResult, PluginRegistry
      apps.rs       # AppProvider: parses .desktop files, builds icon index,
                    # fuzzy-matches with nucleo-matcher
```

## Architecture Notes

**Provider system** — `Provider` trait in `providers/mod.rs`. Implement `id()` + `search(query) -> Vec<SearchResult>` and register via `PluginRegistry::register()`. Results are merged, sorted by score, truncated to 5.

**Startup** — `AppProvider::new()` runs in a background thread. Window opens immediately; frontend shows "Loading…" until the `apps-ready` Tauri event fires.

**Icon index** — Built once at startup by reading only `{theme}/{size}/apps/` dirs (not a full recursive walk). SVG preferred over PNG; larger sizes preferred over smaller. Stored as `HashMap<stem, path>`.

**Background mode** — Portunus runs hidden at all times. To show it, send `--show` to a running instance:
```bash
portunus --show   # signals the daemon via $XDG_RUNTIME_DIR/portunus.sock, then exits
```
On Escape or app launch the window hides (does not exit). State (query, results) resets on the next show.

**Hyprland** — Window rule and keybind in `~/.config/hypr/hyprland.conf`:
```
windowrule = float on, stay_focused 1, no_blur 1, opacity 1 1, border_size 0, match:class portunus
bind = CTRL, SPACE, exec, /path/to/portunus --show
```

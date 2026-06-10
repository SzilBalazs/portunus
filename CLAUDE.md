# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
bun tauri dev                                    # dev mode (hot reload)
bun tauri build                                  # production build (OCR always on; needs libtesseract-dev + libleptonica-dev; linuxdeploy for AppImage)
cargo check --manifest-path src-tauri/Cargo.toml # type-check Rust only
bun x tsc --noEmit                               # type-check TypeScript only
```

Package manager is **bun**, not npm or yarn.

## Architecture

Portunus is a Tauri 2 app: a Rust backend exposed via Tauri IPC to a React 19 / TypeScript frontend. The window is decorationless, transparent, always-on-top, and hidden at startup - it surfaces only when signaled via `portunus --show` (Unix socket IPC).

### Backend (`src-tauri/src/`)

`lib.rs` defines all Tauri commands: `search`, `launch_app`, `get_config`, `save_config`, `trigger_full_reindex`, `is_content_index_empty`, `open_settings_window`, `hide_window`, `is_apps_ready`.

**Provider system** - `providers/mod.rs` defines the `Provider` trait and `PluginRegistry`. The built-in providers are: `apps`, `files`, `recent`, `clipboard`, `calc`, `dict`, `content` (plus WASM extensions). Each implements `id()` + `search(query) -> Vec<SearchResult>`. `PluginRegistry::search()` merges results, applies frecency bonuses, sorts by composite score, and truncates to `max_results` (default 9).

**Scoring** - Composite score = category base + fuzzy score (nucleo-matcher) + frecency bonus. Base scores by kind: clipboard 5M, calc 3M, app 2M, file 1M, folder 0. Results with `nucleo_score < MIN_NUCLEO_SCORE` are filtered out.

**Frecency** (`frecency.rs`) - SQLite DB at `$XDG_DATA_HOME/portunus/frecency.db`. Half-life exponential decay: `new_score = old_score × 2^(−elapsed_days / half_life) + 1.0`. Tracks `app`, `file`, `folder` kinds. `recent:` IDs are normalized to `file:` so both providers share one score.

**Content index** (`content_index.rs`) - SQLite full-text search over file contents. OCR via Tesseract (`leptess`) is always compiled in. Triggered by `trigger_full_reindex` command or watcher. Progress events emitted as `content-index-progress { indexed, total }`.

**Config** (`config.rs`) - TOML at `~/.config/portunus/config.toml`. Hot-reloaded via `watcher.rs`. Config changes propagate through `provider_reload.rs` which rebuilds affected providers and emits `search-invalidated` to the frontend.

**IPC** (`ipc.rs`) - Unix socket at `$XDG_RUNTIME_DIR/portunus.sock` handles `--show`, `--clipboard`, `--reindex`, `--reload-config`, `--reload-extensions` CLI flags from a second instance.

**Startup sequence** - `CalcProvider` registers synchronously. `FileProvider`, `RecentProvider`, and `AppProvider` load in a background thread. Tauri emits `apps-ready` when done; frontend shows a loading state until then.

### Frontend (`src/`)

`App.tsx` - Main launcher UI. Manages query, results, selected index, and preview panel. Listens for Tauri events (`window-show`, `apps-ready`, `content-index-progress`, `appearance-changed`, `search-invalidated`). The `launch` function takes a full `SearchResult` (not a bare exec string) and calls `invoke("launch_app", { exec, id, kind })`.

`Settings.tsx` - Config editor with 8 tabs (General, Providers, Files, Search, Frecency, Content, Debug, Appearance). Autosaves with 800 ms debounce for cheap edits. OCR and max-file-size changes are staged behind an "Apply & Reindex" confirmation to avoid surprise slow rebuilds.

`components/FooterHints.tsx` - Context-aware keyboard hint bar; hints vary by result `kind`.

`providers/registry.ts` - Keyboard dispatch: Up/Down/Enter/Tab/Alt+1-9/Ctrl+C/Ctrl+Enter/Escape.

`types.ts` - TypeScript interfaces: `Config`, `SearchResult`, `DirEntry`.

### Two-window setup

`tauri.conf.json` defines two windows: `main` (900×576, hidden at startup) and `settings` (800×560, pre-created hidden, shown via `open_settings_window` command). Both are WebKit2GTK 4.1 WebViews. Asset protocol is enabled to load system icons from `~/.local/share/icons` and XDG icon dirs.

## System dependencies

Packaged AppImage builds bundle libpdfium, the poppler tools, and English tesseract data (see `runtime_assets.rs` + `tauri.bundle.conf.json`); the items below are needed only for source builds or AppImage-excluded features.

- `libpdfium.so` - PDF preview via `pdfium-render`. Install on Arch: `yay -S pdfium-bin`. `runtime_assets::bind_pdfium` prefers a bundled `libpdfium.so`, else `Pdfium::bind_to_system_library()`.
- `dict` (dictd) - Dictionary lookups. Silently disabled if not found at runtime. Not bundled.
- `cliphist` + `wl-clipboard` - Clipboard history provider. Not bundled.
- `poppler` - PDF content indexing (not preview). Bundled in the AppImage; system binary used for source builds.
- `tesseract` data - OCR (via `leptess`, always compiled in). English data bundled in the AppImage; other languages need the system `tesseract-data-<lang>`.
- `libgtk-layer-shell` - wlr-layer-shell overlay surface for the launcher window (`layer_shell.rs`, opt-in via `[general] layer_shell`). Required at build time on Linux. Install on Arch: `pacman -S gtk-layer-shell`; Debian: `libgtk-layer-shell-dev`. Wayland-only at runtime; no-op on X11.

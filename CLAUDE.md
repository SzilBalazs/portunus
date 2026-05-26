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
| PDF rendering | pdfium-render 0.8 + image 0.25 |
| Frecency DB | rusqlite 0.31 (bundled SQLite) |

## Commands

```bash
bun tauri dev       # dev mode (hot reload)
bun tauri build     # production build (requires linuxdeploy for AppImage)
cargo check --manifest-path src-tauri/Cargo.toml  # type-check Rust only (must target src-tauri)
bun x tsc --noEmit  # type-check TypeScript only
```

## System dependencies

- `libpdfium.so` (x86_64) — required for PDF preview. Install via AUR: `yay -S pdfium-bin` (must be the 64-bit build). Loaded at runtime via `Pdfium::bind_to_system_library()`.
- `dict` — required for dictionary lookups (`define word` / `dict word`). Install: `sudo pacman -S dictd`. If `dict` is not found at runtime the provider is silently disabled.

## Structure

```
src/
  App.tsx           # Search UI, result list, keyboard nav, preview panel
  App.css           # Dark card styles (warm brown palette)
  components/
    ResultsList.tsx # Result rows; onLaunch passes full SearchResult (not just exec)
    PreviewPanel.tsx
    AppPreview.tsx
    FilePreview.tsx
    ClipboardPreview.tsx

src-tauri/
  tauri.conf.json   # Window (900×576, transparent, alwaysOnTop, center)
                    # assetProtocol enabled for icon dirs
  Cargo.toml        # protocol-asset feature required for assetProtocol
  capabilities/
    default.json    # ACL: core:default, core:window:allow-set-size

  src/
    main.rs         # Binary entry point (do not edit)
    lib.rs          # Tauri commands + setup:
                    #   search, launch_app (records frecency), hide_window, is_apps_ready
                    #   render_pdf_page (async, spawn_blocking)
                    #   FileProvider + AppProvider loaded in background thread
                    #   → emits "apps-ready" when both are ready
    frecency.rs     # FrecencyStore: SQLite-backed launch history
                    #   DB at $XDG_DATA_HOME/portunus/frecency.db
                    #   record_launch(id, kind) — half-life decay upsert
                    #   all_scores() → HashMap<id, score>
    config.rs       # Config loader (TOML at ~/.config/portunus/config.toml)
                    # Sections: general, providers, files, recent, search, pdf, frecency
    providers/
      mod.rs        # Provider trait, SearchResult, PluginRegistry
                    # Scoring constants + recency_bonus()
                    # PluginRegistry::search() applies frecency bonus before sort
      apps.rs       # AppProvider: parses .desktop files, builds icon index,
                    # fuzzy-matches with nucleo-matcher
      calc.rs       # CalcProvider: evaluates math expressions via exp-rs
      files.rs      # FileProvider: indexes configured dirs
      recent.rs     # RecentProvider: ~/.local/share/recently-used.xbel
      clipboard.rs  # ClipboardProvider: cliphist integration
      timer.rs      # TimerProvider: countdown timers
```

## Architecture Notes

**Provider system** — `Provider` trait in `providers/mod.rs`. Implement `id()` + `search(query) -> Vec<SearchResult>` and register via `PluginRegistry::register()`. Results are merged, frecency-boosted, sorted by composite score, truncated to `max_results` (default 9).

**SearchResult fields** — `id`, `title`, `subtitle`, `kind`, `score`, `exec`, `icon_path`, `file_size: Option<u64>`, `created: Option<u64>` (Unix secs), `modified: Option<u64>` (Unix secs).

**Scoring system** — Composite score = category base + fuzzy score + recency bonus + frecency bonus. No result with `nucleo_score < MIN_NUCLEO_SCORE` is returned.

| Category | Base score |
|---|---|
| Clipboard | 5,000,000 |
| Timer | 4,000,000 |
| Calc | 3,000,000 (fixed) |
| App | 2,000,000 + nucleo score |
| File | 1,000,000 + nucleo score + recency bonus (0–50) |
| Folder | 0 + nucleo score + recency bonus (0–50) |

Recency bonus decays linearly from 50 (modified today) to 0 (modified ≥ 1 year ago).

**Frecency system** — Every `launch_app` call records the launched item's `id` and `kind` into `~/.local/share/portunus/frecency.db`. Score uses half-life exponential decay:

```
new_score = old_score × 2^(−elapsed_days / half_life) + 1.0
```

Default half-life: 14 days. Default weight multiplier: 5000 (added to `result.score`). Only `kind` values `"app"`, `"file"`, `"folder"` are tracked. `recent:` IDs are normalized to `file:` so both providers share one score. Frecency bonus is applied **before** sort/dedup/truncate in `PluginRegistry::search()`, so heavily-used items can surface above lower-base-score categories.

**launch_app signature** — `fn launch_app(app, exec: String, id: Option<String>, kind: Option<String>, frecency: State<FrecencyState>)`. Frontend passes `id` and `kind` from the full `SearchResult`; the `launch` function in `App.tsx` takes a `SearchResult` (not a bare exec string).

**Startup** — `CalcProvider` registers synchronously. `FileProvider`, `RecentProvider`, and `AppProvider` load in a background thread. Window opens immediately; frontend shows "Loading…" until the `apps-ready` Tauri event fires.

**Icon index** — Built once at startup by reading only `{theme}/{size}/apps/` dirs. SVG preferred over PNG; larger sizes preferred over smaller. Stored as `HashMap<stem, path>`.

**File provider** — Indexes configured dirs (default: `~/Downloads`, `~/Documents`, `~/.config/hypr`) at configurable depth. Collects name, parent path, is_dir, file_size, created, modified from filesystem metadata at index time. `exec` is `xdg-open "<path>"`. Folders get `file_size: None`.

**Preview panel** — Right column of the body. Variants:
- `AppPreview` — for `kind = "app"`: icon, name, description, Launch button
- `FilePreview` — for `kind = "file" | "folder"`: icon, name/tag, optional media preview (image, PDF, text, folder listing), compact metadata strip
- `ClipboardPreview` — for `kind = "clipboard" | "clipboard-image"`
- Empty — for `kind = "calc"` and no selection

**PDF preview** — `render_pdf_page` Tauri command renders page 0 via PDFium at configured width, returns JPEG bytes. Runs in `spawn_blocking`. Frontend uses a two-level cache: `pdfPromiseCache` (Promise) and `pdfUrlCache` (blob URL). Image fades in via `onLoad` opacity transition.

**Background mode** — Portunus runs hidden at all times. To show it:
```bash
portunus --show        # show launcher
portunus --clipboard   # show launcher pre-filled with "clipboard"
```
Both signal a running instance via `$XDG_RUNTIME_DIR/portunus.sock`. On Escape or launch the window hides. State (query, results) resets on next show.

**Hyprland** — Window rule and keybind in `~/.config/hypr/hyprland.conf`:
```
windowrule = float on, stay_focused 1, no_blur 1, opacity 1 1, border_size 0, match:class portunus
bind = CTRL, SPACE, exec, /path/to/portunus --show
```

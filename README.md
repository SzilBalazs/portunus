# Portunus

A keyboard-first application launcher and search tool for Wayland. Type to find
and launch apps, jump to files, do quick math, look up words, browse
clipboard history, and search the text inside your documents, all from one box.
The window stays hidden until you summon it with a keybind, then disappears the
moment you launch something or press Escape.

## Features

- **Fuzzy app & file search**: `.desktop` entries, indexed directories, and GTK recent files, ranked by how often you use them
- **Inline calculator**: type `log2(10^8)` straight into the search bar
- **Dictionary lookup**: `define serendipity` or `dict serendipity` (needs `dictd`)
- **Clipboard history**: full-text search through `cliphist` entries (Wayland)
- **Content search**: prefix a query with `!` to search the text inside PDFs, office documents, and images. OCR reads scanned PDFs and screenshots
- **Preview panel**: images, PDFs, text files, folder listings, and clipboard contents
- **Fast by default**: a non-blocking Rust backend indexes in the background, so results appear as you type

## Install

### AppImage (all Wayland distros)

Download the latest release from the [Releases page](https://github.com/SzilBalazs/portunus/releases).

```bash
chmod +x Portunus-x86_64.AppImage
./Portunus-x86_64.AppImage
```

### Optional runtime dependencies

The AppImage already bundles everything needed for PDF preview, content search,
and OCR (libpdfium, the poppler tools, and the English tesseract data), so those
work with no extra setup. Two features rely on system tools that are not bundled:

| Package | Feature | Arch | Ubuntu/Debian |
|---|---|---|---|
| `cliphist` + `wl-clipboard` | Clipboard history | `sudo pacman -S cliphist wl-clipboard` | `sudo apt install cliphist wl-clipboard` |
| `dictd` | Dictionary definitions | `sudo pacman -S dictd` | `sudo apt install dictd` |

If you build from source instead of using the AppImage, you also need the PDF and
OCR tools installed on your system: `poppler` (or `poppler-utils`), a `pdfium`
build such as `pdfium-bin`, and tesseract with the language data you want
(`tesseract` + `tesseract-data-eng`).

## Compositor setup

Portunus runs hidden at startup. Bind `portunus --show` to a key to reveal it. On launch or Escape it hides again.

| Compositor | Status | Notes |
|---|---|---|
| **Hyprland** | Works wonderfully | See below |
| **Sway** | Should work, not yet tested | `for_window [app_id="portunus"] floating enable, sticky enable` |
| **GNOME Wayland** | Should work, not yet tested | Bind via Settings → Keyboard |
| **KDE Plasma (Wayland)** | Should work, not yet tested | Bind via System Settings → Shortcuts |
| **river / niri / labwc** | Should work, not yet tested | Generic Wayland; configure keybind per compositor |
| **X11** | Partial, not yet tested | Clipboard features require Wayland |

Reports for the untested compositors are welcome, whether it worked or not, so I can update this table. Open a GitHub issue either way.

### Hyprland

```conf
# ~/.config/hypr/hyprland.conf
exec-once = /path/to/portunus

windowrule = pin on, float on, stay_focused 1, no_blur 1, opacity 1 1, border_size 0, match:class portunus
bind = CTRL, SPACE, exec, /path/to/portunus --show
bind = SUPER, V, exec, /path/to/portunus --clipboard
```

## Usage

| Query | Result |
|---|---|
| `firefox` | Fuzzy-match apps and files |
| `define serendipity` | Dictionary definition |
| `log2(10^8)` | Calculator |
| `clipboard search term` | Browse clipboard history |
| `!invoice 2024` | Search file contents |

## Configuration

On first launch Portunus writes a default config to `~/.config/portunus/config.toml`. Every key is documented inline. Config changes are hot-reloaded without a restart.

### Themes

Pick a theme in **Settings → Appearance**. Nine dark themes ship built-in.

#### Matugen (Material You from your wallpaper)

The **Matugen** theme pulls its colors from an external file, so [matugen](https://github.com/InioX/matugen) can recolor Portunus to match your wallpaper. Copy [`templates/portunus.css`](templates/portunus.css) into your matugen config and wire it up:

```toml
# ~/.config/matugen/config.toml
[templates.portunus]
input_path  = "~/.config/matugen/portunus.css"   # copy of templates/portunus.css
output_path = "~/.config/portunus/matugen.css"
post_hook   = "portunus --reload-theme"
```

Run `matugen image <wallpaper>` (add `--mode light` for a light scheme), then select **Matugen** in Settings → Appearance. Every subsequent matugen run recolors the launcher live via the `post_hook`. If `~/.config/portunus/matugen.css` is missing, the theme falls back to default colors.

## Building from source

### Dependencies

| Dependency | Notes |
|---|---|
| Rust stable | via `rustup` |
| Bun | package manager + JS runtime |
| `libwebkit2gtk-4.1-dev` | Tauri WebView |
| `libssl-dev` | |
| `libtesseract-dev` + `libleptonica-dev` | OCR is always built in, so these are required |

```bash
# Build
bun tauri build

# Type-check only
cargo check --manifest-path src-tauri/Cargo.toml
bun x tsc --noEmit
```

## CLI flags

```
portunus [FLAG]

  --show              Show the launcher window (signals a running instance)
  --clipboard         Show the launcher pre-filled with "clipboard"
  --reindex           Rebuild the content search index
  --reload-config     Reload config from file without restarting
  --reload-extensions Re-discover and reload WASM extensions (picks up rebuilt wasm)
  --reload-theme      Re-read the external matugen.css theme (matugen post_hook)
  --version, -V       Print version and exit
  --help, -h          Show this help message
```

## License

[Apache-2.0](LICENSE)

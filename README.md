# Portunus

Application launcher and power-user search for Wayland.

> TODO

## Features

- **Fuzzy app & file search** — `.desktop` files, indexed directories, and GTK recent files with frecency ranking
- **Inline calculator** — type `log2(10^8)` directly in the search bar
- **Dictionary lookup** — `define serendipity` or `dict serendipity` (requires `dictd`)
- **Countdown timers** — `timer 5m break`, `timer 1h30m`
- **Clipboard history** — full-text search through `cliphist` entries (Wayland)
- **Content search** — `! invoice 2024` instantly search the contents of images and PDFs with optional OCR - you can easily search your screenshots!
- **Preview panel** — images, PDFs, text files, folder listings, and clipboard content
- **Extreme speed** — non-blocking Rust backend with blazing-fast runtime indexing

## Install

### AppImage (all Wayland distros)

Download the latest release from the [Releases page](https://github.com/SzilBalazs/portunus/releases).

```bash
chmod +x Portunus-x86_64.AppImage
./Portunus-x86_64.AppImage
```

### Optional runtime dependencies

| Package | Feature | Arch | Ubuntu/Debian |
|---|---|---|---|
| `cliphist` + `wl-clipboard` | Clipboard history | `sudo pacman -S cliphist wl-clipboard` | `sudo apt install cliphist wl-clipboard` |
| `dictd` | Dictionary lookup | `sudo pacman -S dictd` | `sudo apt install dictd` |
| `poppler` | PDF content search | `sudo pacman -S poppler` | `sudo apt install poppler-utils` |
| `pdfium-bin` | PDF preview | bundled | bundled |

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

I would greatly appreciate it if you could report issues for untested compositors. If everything worked easily out of the box feel free to open a github issue so I can update the table.

### Hyprland

```conf
# ~/.config/hypr/hyprland.conf
exec-once = /path/to/portunus

windowrule = float on, stay_focused 1, no_blur 1, opacity 1 1, border_size 0, match:class portunus
bind = CTRL, SPACE, exec, /path/to/portunus --show
bind = SUPER, V, exec, /path/to/portunus --clipboard
```

## Usage

| Query | Result |
|---|---|
| `firefox` | Fuzzy-match apps and files |
| `define serendipity` | Dictionary definition |
| `log2(10^8)` | Calculator |
| `timer 5m break` | Countdown timer |
| `clipboard search term` | Browse clipboard history |
| `!invoice 2024` | Search file contents |

## Configuration

On first launch Portunus writes a default config to `~/.config/portunus/config.toml`. Every key is documented inline. Config changes are hot-reloaded without a restart.

## Building from source

### Dependencies

| Dependency | Notes |
|---|---|
| Rust stable | via `rustup` |
| Bun | package manager + JS runtime |
| `libwebkit2gtk-4.1-dev` | Tauri WebView |
| `libssl-dev` | |
| `libtesseract-dev` + `libleptonica-dev` | Only needed with `--features ocr` |

```bash
# Standard build (no OCR)
bun tauri build

# With OCR (requires libtesseract-dev + libleptonica-dev)
bun tauri build -- --features ocr

# Type-check only
cargo check --manifest-path src-tauri/Cargo.toml
bun x tsc --noEmit
```

## CLI flags

```
portunus [FLAG]

  --show              Show the launcher (signals a running instance)
  --clipboard         Show the launcher pre-filled with "clipboard"
  --reindex           Rebuild the content search index
  --reload-config     Reload config without restarting
  --version, -V       Print version and exit
  --help, -h          Show this message
```

## License

[Apache-2.0](LICENSE)

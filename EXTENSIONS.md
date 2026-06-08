# Portunus Extensions

Portunus can be extended with sandboxed WebAssembly modules. An extension is a
search provider: it receives the launcher query, returns results, and handles
activation (Enter) and an optional preview panel, all through a small, versioned
JSON wire contract. Extensions never ship UI; the host renders everything.

- **Runtime:** [Extism](https://extism.org/) (wasmtime). No WASI.
- **Wire API version:** `1` (see `extension-sdk/`, the single source of truth).
- **Distribution:** a directory dropped into `~/.local/share/portunus/extensions/`.

## Anatomy

```
~/.local/share/portunus/extensions/<name>/
├── manifest.toml      # identity, permissions, limits
└── extension.wasm     # your compiled module
```

A newly installed extension is **disabled**. The user reviews its permissions in
Settings → Extensions and enables it explicitly. Dropping a folder never runs code.

## manifest.toml

```toml
api = 1                        # REQUIRED: wire API major; unknown majors are rejected
name = "emoji"                 # must match the directory name; [a-zA-Z0-9_-] only
version = "0.1.0"
description = "Search and copy emoji"
author = "you"
kinds = ["ext-emoji"]          # result kind; must start with "ext-"; defaults to "ext-<name>"
entry = "extension.wasm"       # default; no path separators

[permissions]
network = ["api.github.com"]   # exact hosts only; wildcards are rejected; omit for none
kv = true                      # per-extension key-value storage
clipboard = true               # clipboard_write host function

[limits]
search_timeout_ms = 100        # clamped to [10, 500]
activate_timeout_ms = 2000     # clamped to [10, 10000]

[background]                   # optional: schedules your `refresh` export
refresh_interval_secs = 600    # clamped to [60, 86400]
```

**Permissions are self-declared.** They bound what your extension *can* do, and
they are shown to the user before enabling. The user's consent at enable time is
the trust gate; declare the minimum you need.

## Exports

Your module exports up to three functions. Each takes one JSON string and
returns one JSON string. With the Rust SDK the (de)serialization is handled
for you.

### `search` (required)

```jsonc
// in
{ "query": "smile" }
// out
{ "results": [ {
    "id": "grinning-face",       // opaque local id; host namespaces it
    "title": "😄 grinning face",
    "subtitle": "smile happy",    // optional
    "relevance": 87.5,            // 0-100, higher = better
    "actions": ["copy"],          // optional; first = default on Enter
    "icon": {                     // optional; shown instead of the default glyph
      "mime": "image/png",        // png/jpeg/gif/webp only
      "data_base64": "iVBOR..."   // ≤ 32 KB base64
    }
} ] }
```

- Relevance is your only ranking input. The host maps 0-100 into its internal
  score space. Out-of-range values are clamped.
- At most 200 results are taken per query; titles/subtitles are clamped to 2 KB.
- Icons are validated host-side: png/jpeg/gif/webp, at most 32 KB base64. An
  invalid icon is dropped (the result keeps the default glyph) and the error
  shows in Settings; a bad icon never fails the search.
- **`search` runs on the keystroke path with a hard ~150 ms budget. Never do
  network I/O in `search`.** Overruns are cancelled and count as failures.

### `activate` (required for actionable results)

```jsonc
// in: the result EXACTLY as you returned it, plus the chosen action (or null)
{ "result": { ...ExtensionResult }, "action": "copy" }
// out
{ "ok": true }
```

The full result round-trips back to you, so you never need to persist search
state. Side effects (clipboard, HTTP, kv) happen here. Budget: 2 s by default.

### `preview` (optional)

Called lazily when the user selects one of your results (500 ms budget).
Return one of the declarative content types; the host renders it natively:

```jsonc
{ "type": "markdown",  "content": "## GFM markdown (raw HTML is not rendered)" }
{ "type": "metadata",  "items": [ { "label": "Version", "value": "1.2.0" } ] }
{ "type": "image",     "mime": "image/png", "data_base64": "..." }   // ≤ 1 MB; png/jpeg/gif/webp only
{ "type": "list",      "items": [ { "title": "...", "subtitle": "...", "tag": "v2", "mono": true } ] }
{ "type": "sections",  "items": [ { "heading": "Basic usage", "rows": [ ["cmd --flag", "description"], ["solo-cmd"] ] } ] }
{ "type": "code",      "lang": "bash", "content": "echo hello" }
{ "type": "html",      "content": "<style>…</style><div>…</div>" }   // ≤ 128 KB; see below
```

#### `list` options

| Field | Type | Notes |
|-------|------|-------|
| `title` | string | required |
| `subtitle` | string? | muted second line |
| `tag` | string? | small badge chip to the right of the title |
| `mono` | bool | render `title` in monospace (default false) |

#### `sections`

Two-column command/description table, optionally grouped under headings. Ideal
for cheat sheets, man pages, shortcut references.

Each `rows` entry is a `Vec<String>`. **Two or more cells**: first cell is the
command (monospace, accent colour); remaining cells are the description (muted).
**Single cell**: spans both columns as a standalone code block.

#### `code`

Monospace preformatted block. `lang` is reserved for future syntax highlighting
and is passed through unchanged; no highlighting is applied today.

#### `html`

Arbitrary HTML+CSS rendered in a **sandboxed `<iframe>`**. Use this when the
declarative types can't express your layout (weather cards, file trees, CSS
charts, custom metadata grids).

**What the host injects** into every srcdoc before your content:

```html
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src 'unsafe-inline' data:; img-src data:;">
<style>
  :root { --fg:…; --fg-mute:…; --fg-dim:…; --fg-desc:…;
          --bg:…; --bg-deep:…; --bg-card:…;
          --accent:…; --accent-soft:…; --accent-border:… }
  * { box-sizing: border-box; margin: 0; padding: 0 }
  body { background: transparent; color: var(--fg);
         font-size: 13px; line-height: 1.5;
         font-family: system-ui, -apple-system, sans-serif }
</style>
```

All current theme values are baked in at render time, so `var(--fg)`,
`var(--accent)`, `var(--bg-deep)`, etc. work in your inline CSS.

**What is allowed**: inline `<style>` blocks, inline `style=""` attributes,
`data:` URIs (for embedded images), flexbox/grid, CSS custom properties,
CSS animations.

**What is blocked** (enforced by both `sandbox=""` and CSP):
- JavaScript (no `<script>`, no event handlers, no `javascript:` URIs)
- External network requests (`url()` to external hosts, `<img src="https://…">`)
- Forms and navigation
- Cookies and storage

**Cap**: 128 KB. Larger content is rejected by the host and surfaces an error in
Settings; the result still appears without a preview.

#### Utility classes

The host injects a small design-system sheet. Use these classes directly; no
`<style>` block needed for common patterns.

**Color**

| Class | Effect |
|---|---|
| `.text-mute` | `--fg-mute` text |
| `.text-dim` | `--fg-dim` text |
| `.text-desc` | `--fg-desc` text |
| `.text-accent` | `--accent` text |

**Typography**

| Class | Effect |
|---|---|
| `.text-xs` | 10 px, slight letter-spacing |
| `.text-sm` | 11 px |
| `.text-lg` | 16 px |
| `.text-hero` | 42 px, weight 200, for big numbers |
| `.text-label` | 10 px uppercase, muted, for section headers |
| `.mono` | monospace stack, 12 px |
| `.truncate` | single-line ellipsis |

**Layout**

| Class | Effect |
|---|---|
| `.row` | `flex; align-items:center; gap:8px` |
| `.col` | `flex-direction:column; gap:6px` |
| `.fill` | `flex:1; min-width:0` |
| `.between` | `justify-content:space-between` |
| `.wrap` | `flex-wrap:wrap` |

**Surfaces**

| Class | Effect |
|---|---|
| `.card` | `--bg-card` background, `--radius-sm` corners, 10/12 px padding |
| `.surface` | `--bg-deep` background, same shape |
| `.divider` | 1 px `--line` separator (use as `<hr>` or `<div>`) |

**Badges & accents**

| Class | Effect |
|---|---|
| `.tag` | small muted chip (`--accent-soft` fill) |
| `.tag-accent` | small accent-filled chip (dark text) |
| `.bar` | 3 px accent progress bar; set `width` inline |
| `.accent-line` | left border in `--accent-border` + 8 px indent |

**Example** (no custom CSS needed):
```html
<div class="card col" style="gap:12px">
  <div class="row between">
    <span class="text-label">Budapest</span>
    <span class="tag">partly cloudy</span>
  </div>
  <div class="text-hero">22°</div>
  <div class="bar" style="width:72%"></div>
  <div class="row between text-sm text-mute">
    <span>Mon 24°</span><span>Tue 19°</span><span>Wed 21°</span>
  </div>
</div>
```

### `refresh` (optional)

Declared via `[background]` in the manifest. The host calls it once when the
extension loads and then on the interval, on a **dedicated instance**, so a
slow refresh never blocks search. This is where network work belongs: fetch,
write kv, return. 30 s budget. Failures show in Settings but never bench the
extension; five consecutive failures pause the schedule until reload.

```jsonc
// in:  { "reason": "load" | "scheduled" }
// out: { "ok": true }
```

### The kv-as-cache pattern

The canonical recipe for network extensions:

1. `refresh` fetches over HTTP and writes results to kv, timestamped with `now_ms`.
2. `search` reads only kv, so it is always instant and always within budget.
3. `activate` may also fetch (user-triggered, 2-10 s budget) for manual refresh.

Never do network in `search`; the 150 ms keystroke budget will cancel it.

## Host functions

Importable from the `extism:host/user` namespace; the Rust SDK wraps them:

| Function | Permission | Notes |
|---|---|---|
| `kv_get(key) -> Option<String>` | `kv` | per-extension namespace |
| `kv_set(key, value)` | `kv` | 10 MB quota per extension |
| `kv_list(prefix) -> Vec<String>` | `kv` | keys only, max 10 000 |
| `kv_delete(key)` | `kv` | |
| `clipboard_write(text)` | `clipboard` | wl-copy under the hood |
| `now_ms() -> u64` | none | wall clock (std::time doesn't work in wasm32-unknown-unknown) |
| `open_url(url)` | `open_url` | http(s) only, opens default browser |
| `log_message(text)` | none | `[ext:<name>] …` to stderr, 4 KB cap |

SDK wrapper names: `kv_read`, `kv_write`, `kv_keys`, `kv_remove`, `clipboard`,
`now`, `open`, `debug`.

**Compatibility:** importing a host function the running Portunus doesn't have
fails at load with a visible Settings error. Extensions using `kv_list`,
`kv_delete`, `now_ms`, `open_url`, `log_message`, or `refresh` scheduling
require a Portunus build that ships them.

**HTTP** has no custom host function; use Extism's built-in HTTP
(`extism_pdk::http::request` in Rust). The host derives the allowlist from
`[permissions].network`; requests to other hosts fail.

## Writing an extension in Rust

```bash
cargo new --lib my-ext && cd my-ext
rustup target add wasm32-unknown-unknown
```

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
portunus-ext-sdk = { path = "../portunus/extension-sdk" }  # or git
```

```rust
use portunus_ext_sdk::guest::extism_pdk; // satisfies pdk macro paths
use portunus_ext_sdk::guest::{plugin_fn, FnResult, Json};
use portunus_ext_sdk::*;

#[plugin_fn]
pub fn search(input: Json<SearchInput>) -> FnResult<Json<SearchOutput>> {
    let q = input.0.query;
    // ...
    Ok(Json(SearchOutput { results: vec![] }))
}
```

Build + install + reload:

```bash
cargo build --release --target wasm32-unknown-unknown
DEST=~/.local/share/portunus/extensions/my-ext
mkdir -p "$DEST"
cp target/wasm32-unknown-unknown/release/my_ext.wasm "$DEST/extension.wasm"
cp manifest.toml "$DEST/"
portunus --reload-extensions
```

`portunus --reload-extensions` force-reloads the wasm bytes of every extension;
that command is your hot-reload loop. The Rescan button in Settings → Extensions
does the same.

Any language with an [Extism PDK](https://extism.org/docs/concepts/pdk)
(Go/TinyGo, Zig, C, JS via the JS PDK, and more) works too: implement the same JSON
exports and import the host functions above.

## Sandbox, limits, failure behavior

- No filesystem, no process spawning, no network beyond the manifest allowlist.
- 64 MB linear memory cap; `extension.wasm` must be ≤ 32 MB.
- Each call runs under a wall-clock watchdog (cancelled via wasmtime epoch
  interruption). A trapped/timed-out call returns empty results; it never
  crashes or stalls the launcher.
- After a failed call the instance is rebuilt from disk before the next one.
  **Three consecutive failures bench the extension for the session**; the error
  shows in Settings → Extensions (your primary debugging signal, check there
  first when "my extension returns nothing").
- Result ids are namespaced `ext:<name>:<your-id>` by the host. Your local id is
  opaque and may contain anything, including colons.
- Frecency: results the user activates rank higher in later searches
  automatically, keyed by your stable result id.
- Uninstalling (deleting the directory) removes the extension's kv data and
  frecency history on the next rescan/startup.

## Versioning

`api` in the manifest is the wire-contract major. The host refuses to load
extensions targeting a different major and shows why. Additive changes (new
optional fields, new preview types) do not bump the major; field removals or
semantic changes do.

## Example

`examples/extensions/emoji/` is a complete, buildable extension demonstrating
search, activate (clipboard), preview (metadata), and the dev loop.

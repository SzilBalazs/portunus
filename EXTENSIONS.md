# Portunus Extensions

Portunus can be extended with sandboxed WebAssembly modules. An extension is a
search provider: it receives the launcher query, returns results, and handles
activation (Enter) and an optional preview panel, all through a small, versioned
JSON wire contract. Extensions never ship UI; the host renders everything.

- **Runtime:** [Extism](https://extism.org/) (wasmtime). No WASI.
- **Wire API version:** `2` (see `extension-sdk/`, the single source of truth).
- **Distribution:** a `.portext` archive installed from Settings → Extensions
  (URL or file), or a directory in `~/.local/share/portunus/extensions/`.

## Quick start

```bash
portunus ext new my-ext         # scaffold a working extension project
cd my-ext
cargo build --release           # targets wasm32-unknown-unknown via .cargo/config.toml
cp target/wasm32-unknown-unknown/release/my_ext.wasm extension.wasm
portunus ext dev .              # link into Portunus + auto-reload on rebuild
```

Enable it once in Settings → Extensions, then type `my-ext ` in the launcher.
When it's ready to ship: `portunus ext pack .` produces `my-ext.portext` and
prints its sha256.

## Anatomy

```
~/.local/share/portunus/extensions/<name>/
├── manifest.toml      # identity, trigger, permissions, limits, settings
└── extension.wasm     # your compiled module
```

A hand-dropped extension is **disabled** until the user reviews its permissions
in Settings → Extensions and enables it. Installing through the Settings dialog
shows the same permission review before anything lands, so dialog installs
arrive enabled. Dropping a folder never runs code.

## manifest.toml

```toml
api = 2                        # REQUIRED: wire API major; unknown majors are rejected
name = "emoji"                 # must match the directory name; [a-zA-Z0-9_-] only
version = "0.2.0"
description = "Search and copy emoji"
author = "you"
homepage = "https://github.com/you/emoji"  # optional, shown in Settings
kinds = ["ext-emoji"]          # result kind; must start with "ext-"; defaults to "ext-<name>"
entry = "extension.wasm"       # default; no path separators

# Strongly recommended. Without [trigger] the extension runs on EVERY keystroke.
[trigger]
prefixes = ["emoji", "em"]     # lowercase ascii/digits/-/_ only; "define"/"dict" are reserved
min_query_len = 1              # applies to the post-strip query; clamped to [0, 32]
always = false                 # true = also run on non-prefixed queries (discouraged)

[permissions]
network = ["api.github.com"]   # exact hosts only; wildcards are rejected; omit for none
kv = true                      # per-extension key-value storage
clipboard = true               # clipboard_write host fn (NOT needed for CopyText effects)
open_url = true                # open_url host fn (NOT needed for OpenUrl effects)

[limits]
search_timeout_ms = 100        # clamped to [10, 500]
activate_timeout_ms = 2000     # clamped to [10, 10000]

[background]                   # optional: schedules your `refresh` export
refresh_interval_secs = 600    # clamped to [60, 86400]

[[settings]]                   # optional, repeated: user-editable options
key = "tone"                   # [a-z0-9_]+; read via settings_get / setting_str
type = "select"                # string | bool | number | select
label = "Skin tone modifier"
description = "Applied to hand emoji"   # optional helper text
default = "none"
options = ["none", "light", "medium", "dark"]  # select only
# number also accepts: min / max / step;  string accepts: placeholder
```

**Permissions are self-declared and snapshotted at consent time.** They are
shown to the user before enabling/installing, and an update whose permissions
*grow* past that snapshot refuses to load until the user re-approves in
Settings. Declare the minimum you need.

## Triggers

With `[trigger]` declared, the host only calls your `search` when the query's
first token equals one of your prefixes (case-insensitive), and it strips the
prefix before you see it:

| User types | Your `search` gets |
|---|---|
| `emoji smi` | `{ "query": "smi", "raw_query": "emoji smi", "trigger": "emoji" }` |
| `emoji` / `emoji ` | `{ "query": "", … }` — the *browse state*: return default/popular results (or nothing) |
| `firefox` | *(not called at all)* |

This is the main performance lever: a gated-out extension costs the keystroke
path literally nothing. Trigger-matched results also rank in the launcher's
intent band (above apps, calc and dict) because the user explicitly asked for
you. Without `[trigger]` (always-mode) your results compete just above apps and
you pay a wasm call per keystroke.

## Exports

Your module exports up to four functions. Each takes one JSON string and
returns one JSON string. With the Rust SDK the (de)serialization is handled
for you.

### `search` (required)

```jsonc
// in
{ "query": "smile", "raw_query": "emoji smile", "trigger": "emoji" }
// out
{ "results": [ {
    "id": "grinning-face",       // opaque local id; host namespaces it
    "title": "😄 grinning face",
    "subtitle": "smile happy",    // optional
    "relevance": 87.5,            // 0-100, higher = better
    "actions": [                  // optional; first = default on Enter,
      { "id": "copy", "label": "Copy emoji" },              // rest via Alt+Enter picker
      { "id": "copy-name", "label": "Copy name", "hint": "as :shortcode:" }
    ],
    "badge": "beta",              // optional small chip on the result row
    "icon": {                     // optional; shown instead of the default glyph
      "mime": "image/png",        // png/jpeg/gif/webp only
      "data_base64": "iVBOR..."   // ≤ 32 KB base64
    }
} ] }
```

- Relevance is your only ranking input. The host maps 0-100 into a band whose
  base depends on whether the query was trigger-matched. Out-of-range values
  are clamped.
- At most 200 results are taken per query; titles/subtitles/badges are clamped to 2 KB.
- Icons are validated host-side: png/jpeg/gif/webp, at most 32 KB base64. An
  invalid icon is dropped (the result keeps the default glyph) and the error
  surfaces in Settings.

### `activate` (required for actionable results)

```jsonc
// in: the result EXACTLY as you returned it, plus the chosen action id (or null)
{ "result": { ...ExtensionResult }, "action": "copy" }
// out: declarative effects the host executes for you
{ "effects": [
  { "type": "copy_text", "text": "😄" },
  { "type": "show_toast", "message": "Copied!" }
] }
```

The full result round-trips back to you, so you never need to persist search
state. Budget: 2 s by default.

**Effects** run host-side after your call returns, in order. Because they only
ever run on an explicit keypress, they need **no permissions**:

| Effect | Fields | Notes |
|---|---|---|
| `copy_text` | `text` | system clipboard |
| `open_url` | `url` | http(s) only, default browser |
| `show_toast` | `message` | desktop notification + in-launcher toast; ≤ 512 bytes |

Most extensions can drop their `clipboard`/`open_url` permissions entirely and
just return effects. The host functions remain for side effects *outside*
activation (e.g. during `refresh`). Returning `{ "effects": [] }` (or `{}`) is
fine when you did your work via host functions.

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
write kv, return. 30 s budget. After a successful refresh the host re-runs any
open launcher query, so fresh data appears without a keystroke. Failures show
in Settings but never bench the extension; five consecutive failures pause the
schedule until reload.

```jsonc
// in:  { "reason": "load" | "scheduled" }
// out: {}
```

### The kv-as-cache pattern

The canonical recipe for network extensions:

1. `refresh` fetches over HTTP and writes results to kv, timestamped with `now_ms`.
2. `search` reads only kv, so it is always instant and always within budget.
3. `activate` may also fetch (user-triggered, 2-10 s budget) for manual refresh.

Never do network in `search`; the 150 ms keystroke budget will cancel it.

## Settings

Declare `[[settings]]` entries in the manifest and Portunus renders them in
Settings → Extensions → your card, with type-appropriate controls. Read them
at call time:

```rust
use portunus_ext_sdk::guest::{setting_str, setting_bool, setting_num};

let tone = setting_str("tone")?;   // user's value, or your manifest default
```

Values are validated against your schema on both save and load; you always see
schema-shaped data. A settings change hot-reloads your extension (a fresh
instance with the new snapshot), so treat them as constants per instance.

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
| `log_message(text)` | none | stderr + the Settings log viewer, 4 KB cap |
| `settings_get(key) -> Option<Value>` | none | your `[[settings]]` values, defaults applied |

SDK wrapper names: `kv_read`, `kv_write`, `kv_keys`, `kv_remove`, `clipboard`,
`now`, `open`, `debug`, `setting` / `setting_str` / `setting_bool` / `setting_num`.

**HTTP** has no custom host function; use Extism's built-in HTTP
(`extism_pdk::http::request` in Rust). The host derives the allowlist from
`[permissions].network`; requests to other hosts fail.

## Developer workflow

```bash
portunus ext new <name>       # scaffold: Cargo.toml, manifest, src/lib.rs, README
portunus ext dev <dir>        # symlink into the extensions dir + watch: every
                              # rebuild of extension.wasm (or manifest edit)
                              # hot-reloads just your extension
portunus ext validate <dir>   # manifest lint + wasm export check (no code runs)
portunus ext pack <dir>       # build <name>.portext + print its sha256
```

Debugging signals, in order of usefulness:
1. **Settings → Extensions → your card → Logs** - `debug(...)` output and every
   runtime error, live.
2. The error line on your card (manifest/load/last runtime error).
3. stderr of the running Portunus.

`portunus --reload-extensions` force-reloads every extension;
`portunus --reload-extension <name>` reloads one (what `ext dev` sends on
rebuild). The Rescan button in Settings does the former.

Any language with an [Extism PDK](https://extism.org/docs/concepts/pdk)
(Go/TinyGo, Zig, C, JS via the JS PDK, and more) works too: implement the same JSON
exports and import the host functions above.

## Distribution

A `.portext` is a plain zip of your extension directory with `manifest.toml`
at the root (`portunus ext pack` builds one). Users install it from Settings →
Extensions → Install, from either a local file or an https URL, optionally
verifying a sha256 you publish.

Install is two-phase: the archive is downloaded/staged/validated first
(hardened extraction - no symlinks, no path escapes, size caps), the user
reviews name/version/permissions/hash, and only then do the *exact staged
bytes* land. The install origin is recorded: URL installs get a "Check for
update" button that re-fetches from the same URL and compares versions - an
update requesting **new permissions** requires explicit re-approval before it
loads. Uninstalling from Settings removes the directory, kv data, frecency
history and logs.

## Sandbox, limits, failure behavior

- No filesystem, no process spawning, no network beyond the manifest allowlist.
- 64 MB linear memory cap; `extension.wasm` must be ≤ 32 MB.
- Each call runs under a wall-clock watchdog (cancelled via wasmtime epoch
  interruption). A trapped/timed-out call returns empty results; it never
  crashes or stalls the launcher.
- After a failed call the instance is rebuilt from disk before the next one.
  **Three consecutive failures bench the extension for the session**; the error
  shows in your card's log viewer (check there first when "my extension
  returns nothing").
- Result ids are namespaced `ext:<name>:<your-id>` by the host. Your local id is
  opaque and may contain anything, including colons.
- Frecency: results the user activates rank higher in later searches
  automatically, keyed by your stable result id.

## Versioning

`api` in the manifest is the wire-contract major. The host refuses to load
extensions targeting a different major and shows why. Additive changes (new
optional fields, new preview types, new effects) do not bump the major; field
removals or semantic changes do. v2 broke from v1: `actions` became structured
objects, `activate` returns effects, `search` input gained `raw_query`/`trigger`.

## Examples

- `examples/extensions/emoji/` - offline: triggers, structured actions,
  activate effects (no permissions), settings, browse state, preview.
- `examples/extensions/cheatsh/` - network: kv-as-cache with background
  refresh, trigger prefixes, open-url/copy effects, HTML preview.

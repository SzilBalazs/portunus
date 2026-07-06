# Portunus Extensions

Portunus can be extended with sandboxed WebAssembly modules. An extension is a
search provider: it receives the launcher query, returns results, and handles
activation (Enter) and an optional preview panel, all through a small, versioned
JSON wire contract. Extensions never ship UI; the host renders everything.

- **Runtime:** [Extism](https://extism.org/) (wasmtime). No WASI.
- **Wire API version:** `4` (see `extension-sdk/`, the single source of truth).
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

Enable it once in Settings → Extensions, then either type `my-ext ` in the
launcher or search for its "Search my-ext" entry and press Enter — the
scaffolded manifest declares one `[[commands]]` entry that answers to both.
When it's ready to ship: `portunus ext pack .` produces `my-ext.portext` and
prints its sha256.

## Anatomy

```
~/.local/share/portunus/extensions/<name>/
├── manifest.toml      # identity, commands, permissions, limits, settings
└── extension.wasm     # your compiled module
```

A hand-dropped extension is **disabled** until the user reviews its permissions
in Settings → Extensions and enables it. Installing through the Settings dialog
shows the same permission review before anything lands, so dialog installs
arrive enabled. Dropping a folder never runs code.

## manifest.toml

```toml
api = 4                        # REQUIRED: wire API major; unknown majors are rejected
name = "emoji"                 # must match the directory name; [a-zA-Z0-9_-] only
version = "0.2.0"
description = "Search and copy emoji"
author = "you"
homepage = "https://github.com/you/emoji"  # optional, shown in Settings
kinds = ["ext-emoji"]          # result kind; must start with "ext-"; defaults to "ext-<name>"
entry = "extension.wasm"       # default; no path separators

# REQUIRED: at least one [[commands]] entry. Each is a searchable launcher
# entry ("Search Emoji") the user finds by fuzzy-matching its title/keywords
# and opens. One wasm module can serve several commands - the wire's `command`
# field says which one is running.
[[commands]]
name = "search"                # [a-z0-9_-]+, unique within the extension; wire id
title = "Search Emoji"         # required: the searchable launcher entry's title
description = "Find and copy emoji by name"  # optional: entry subtitle
mode = "scope"                 # scope | action (default scope; "inline" is a legacy alias for scope)
keywords = ["emoji", "em", "smiley"]  # optional search synonyms folded into the fuzzy match
min_query_len = 1              # applies to the scope query; clamped to [0, 32]
always = false                 # run live in root search on every keystroke (discouraged)
chip = "Emoji"                 # optional: active-mode chip label; defaults to title
placeholder = "Search emoji…"  # optional: input placeholder while the mode is active
kind = "ext-emoji"             # optional: per-command result-kind override; must be in `kinds`
icon = "icon_emoji.b64"        # optional: bundled base64-PNG file (bare filename) shown on the command's entry; else a generic glyph

[permissions]
network = ["api.github.com"]   # exact hosts only; wildcards are rejected; omit for none
kv = true                      # per-extension key-value storage
clipboard = true               # clipboard_write host fn (NOT needed for CopyText effects)
open_url = true                # open_url host fn (NOT needed for OpenUrl effects)
paste = true                   # Paste activate effect (synthetic Ctrl+V into other apps)

[limits]
search_timeout_ms = 100        # clamped to [10, 500]
activate_timeout_ms = 2000     # clamped to [10, 10000]
query_timeout_ms = 10000       # optional `query` export; clamped to [500, 60000]
preview_timeout_ms = 500       # `preview` export; clamped to [100, 10000]

[background]                   # optional: schedules your `refresh` export
refresh_interval_secs = 600    # clamped to [60, 86400]

[[settings]]                   # optional, repeated: user-editable options
key = "tone"                   # [a-z0-9_]+; read via settings_get / setting_str
type = "select"                # string | bool | number | select | secret
label = "Skin tone modifier"
description = "Applied to hand emoji"   # optional helper text
default = "none"
options = ["none", "light", "medium", "dark"]  # select only
# number also accepts: min / max / step;  string accepts: placeholder

[[settings]]                   # a secret: stored in the system keyring, never config.toml
key = "api_key"
type = "secret"                # masked input; no default/options/min/max allowed
label = "API key"
description = "Your api.example.com token"
```

**Permissions are self-declared and snapshotted at consent time.** They are
shown to the user before enabling/installing, and an update whose permissions
*grow* past that snapshot refuses to load until the user re-approves in
Settings. Declare the minimum you need.

## Commands

Every extension is one or more **commands**. A command is a searchable entry in
the launcher root: its `title` ("Search Emoji") is matched by the same fuzzy
search as apps and files, alongside any `keywords` you declare (search synonyms
like `["emoji", "smiley"]`). There is no prefix or trigger syntax — the user
finds a command by typing something that fuzzy-matches its title or keywords,
then opens it.

`mode` picks what opening it does:

| Mode | Behavior |
|---|---|
| `scope` (default) | Enter opens an enterable mode with its own chip and input placeholder; every keystroke then routes to your `search`. Backspace on an empty query exits. (`inline` is accepted as a legacy alias for `scope`.) |
| `action` | One-shot: Enter calls `activate` immediately with a synthetic default result and closes. Set `opens_form = true` on the entry if the activation answers with a `show_form` effect — the launcher then stays visible (with a "Working…" pill) instead of hiding optimistically, so the form doesn't flash the window hidden-then-shown. |

Inside a scope, your `search` receives the whole typed term (`query` and
`raw_query` are identical); an empty `query` is the *browse state*:

| User did | Your `search` gets |
|---|---|
| opened "Search Emoji", typed `smi` | `{ "command": "search", "query": "smi", "raw_query": "smi" }` |
| opened "Search Emoji", empty input | `{ "command": "search", "query": "", … }` — return default/popular results (or nothing) |

Scope results rank in the launcher's intent band (above apps, calc and dict)
because the user explicitly entered your command.

**`always` commands** are the one exception that runs in root search: set
`always = true` and your `search` runs on *every keystroke* with the raw query,
like a built-in provider (results compete just above apps). It pays a wasm call
per keystroke, so reserve it for provider-style extensions (e.g. a live unit
converter) where an entered scope genuinely doesn't fit. `always` does not apply
to `action` commands.

**Multiple commands** share one wasm module; dispatch on the wire's `command`
field (the `[[commands]]` entry's `name`) in `search`/`activate`/`preview`/
`query`. See `examples/extensions/gh/manifest.toml` for a three-command
extension (two `scope` search commands, one `action` command).

## Exports

Your module exports up to five functions. Each takes one JSON string and
returns one JSON string. With the Rust SDK the (de)serialization is handled
for you.

There are two search tiers. `search` is the synchronous fast path on a tight
keystroke budget — trivial extensions only ever need it. `query` is the
optional async tier: a generous budget, a dedicated instance, and streaming, so
you can hit the network without blocking the launcher.

### `search` (required)

```jsonc
// in
{ "command": "search", "query": "smile", "raw_query": "smile" }
// out
{ "results": [ {
    "id": "grinning-face",       // opaque local id; host namespaces it
    "title": "😄 grinning face",
    "subtitle": "smile happy",    // optional
    "relevance": 87.5,            // 0-100, higher = better
    "actions": [                  // optional; first = default on Enter,
      { "id": "copy", "label": "Copy emoji" },              // rest via Alt+Enter picker
      { "id": "copy-name", "label": "Copy name", "hint": "as :shortcode:" },
      { "id": "report", "label": "New Issue…", "opens_form": true }  // keeps the window up while activate runs
    ],
    "badge": "beta",              // optional small chip on the result row
    "icon": {                     // optional; shown instead of the default glyph
      "mime": "image/png",        // png/jpeg/gif/webp only
      "data_base64": "iVBOR..."   // ≤ 32 KB base64
    }
} ] }
```

- Relevance is your only ranking input. The host maps 0-100 into a band whose
  base depends on whether the results came from an entered scope (higher) or an
  `always` command in root search (lower). Out-of-range values are clamped.
- At most 200 results are taken per query; titles/subtitles/badges are clamped to 2 KB.
- Icons are validated host-side: png/jpeg/gif/webp, at most 32 KB base64. An
  invalid icon is dropped (the result keeps the default glyph) and the error
  surfaces in Settings.

### `query` (optional) — async & streaming search

Export `query` when your results need real work (network calls, pagination,
LLM output) that can't fit `search`'s keystroke budget. The host runs it on a
dedicated instance under `[limits] query_timeout_ms` (default 10 s), off the
keystroke path — built-in results and your `search` results render instantly
while `query` is still running, shown with a per-extension loading row.

Input is identical to `search` (`command`, `query`, `raw_query`). Push partial
batches with `guest::emit` as they arrive; the return value is the final batch.

```rust
use portunus_ext_sdk::guest::{self, plugin_fn, FnResult, Json};
use portunus_ext_sdk::{QueryInput, QueryOutput};

#[plugin_fn]
pub fn query(input: Json<QueryInput>) -> FnResult<Json<QueryOutput>> {
    for page in 0..5 {
        let results = fetch_page(&input.0.query, page)?;   // blocking HTTP is fine here
        if !guest::emit(results)? { break; }               // false = cancelled, stop now
    }
    Ok(Json(QueryOutput::default()))
}
```

- **Cancellation.** A new keystroke cancels the in-flight query. `emit` returns
  `false` once that happens — bail out between your blocking calls. Anything you
  emit after cancellation is dropped by the host regardless.
- **Merging.** Streamed results merge into the list by `id`. Emitting a result
  whose `id` matches one your `search` already returned *replaces* it in place —
  the canonical "show the cached row instantly, swap in the fresh one when the
  network answers" pattern.
- **Caps & failure.** At most 200 results per query across all batches; each
  batch ≤ 512 KB. Query failures never bench your `search`; five consecutive
  failures disable just the async tier for the session (the error shows in
  Settings), and it recovers on reload.

### `activate` (required for actionable results)

```jsonc
// in: the result EXACTLY as you returned it, plus the chosen action id (or null)
{ "command": "search", "result": { ...ExtensionResult }, "action": "copy" }
// out: declarative effects the host executes for you
{ "effects": [
  { "type": "copy_text", "text": "😄" },
  { "type": "show_toast", "message": "Copied!" }
] }
```

The full result round-trips back to you, so you never need to persist search
state. Budget: 2 s by default.

**Action commands** (`mode = "action"`) skip `search` entirely: selecting
their launcher entry calls `activate` directly, with `command` set to the
command's name and a synthetic default result (empty `id`/`title`) standing
in for the "result exactly as you returned it" — there was nothing to return.
Use this for one-shot side effects like "open GitHub notifications in the
browser" (see `examples/extensions/gh/manifest.toml`'s `notifications`
command).

**Effects** run host-side after your call returns, in order. Because they only
ever run on an explicit keypress, they need **no permissions** (except
`paste`, which injects keystrokes into *another* application):

| Effect | Fields | Notes |
|---|---|---|
| `copy_text` | `text` | system clipboard |
| `open_url` | `url` | http(s) only, default browser |
| `show_toast` | `message`, `level?` | `level`: `info` (default) / `success` / `error`; in-launcher toast queue, desktop notification when the window is hidden (errors always notify); ≤ 512 bytes |
| `show_form` | `title`, `fields`, `submit_action`, `submit_label?` | open a modal form — see below |
| `hide` | — | hide the launcher after activation (the default; explicit form for clarity) |
| `keep_open` | — | keep the launcher open (toggle-style actions) |
| `refresh_results` | — | re-run the current query (after delete/toggle/mark-done actions) |
| `paste` | `text` | copy + synthetic Ctrl+V into the previously focused window; **requires `paste = true` in `[permissions]`**. Clobbers the clipboard; falls back to a "Copied — press Ctrl+V" notification on compositors without the virtual-keyboard protocol (e.g. GNOME) |

At most 16 effects run per activation; extras are dropped and logged. Window
visibility resolves as `show_form` > `hide` > `keep_open` > default (hide).

Most extensions can drop their `clipboard`/`open_url` permissions entirely and
just return effects. The host functions remain for side effects *outside*
activation (e.g. during `refresh`). Returning `{ "effects": [] }` (or `{}`) is
fine when you did your work via host functions.

#### Forms (`show_form`)

`show_form` turns an action into a multi-input flow — create an issue, add a
bookmark, rename something — without the extension shipping any UI:

```jsonc
// activate out:
{ "effects": [ { "type": "show_form",
  "title": "New issue in portunus",
  "submit_action": "create_issue",   // comes back as `action` on submit
  "submit_label": "Create",          // optional; defaults to "Submit"
  "fields": [
    { "key": "title", "label": "Title", "type": "text", "required": true },
    { "key": "body",  "label": "Body",  "type": "textarea", "placeholder": "Describe…" },
    { "key": "label", "label": "Label", "type": "select",
      "options": [ { "value": "bug", "label": "Bug" }, { "value": "feat", "label": "Feature" } ] }
  ] } ] }
```

The launcher renders the modal. **Submit** calls your `activate` again on the
same result with `action = submit_action` and the collected values:

```jsonc
// activate in (second call):
{ "command": "repos", "result": { ...same result... },
  "action": "create_issue",
  "form_values": { "title": "Crash on start", "body": "…", "label": "bug" } }
```

**Cancel (Esc) makes no call.** A submit handler may return another
`show_form` for multi-step flows — each step is user-gated, so there is no
loop risk. Field types: `text`, `textarea` (Ctrl+Enter submits), `password`,
`select`, `checkbox` (bool value), `number` (number value). `required` fields
block submit client-side. Caps: 32 fields, 64 select options, title ≤ 120
chars, submitted string values ≤ 16 KB. A failed/timed-out submit shows an
error toast and keeps the form open with the entered values. Form-opening
activations don't count toward frecency; the submit does.

**Declare form-opening entry points.** The launcher normally hides itself
~150 ms into an activation so dismissal feels instant; an activation that
answers with `show_form` would flash the window hidden-then-shown. Mark such
entry points up front — `opens_form = true` on an `action` command in the
manifest, or `"opens_form": true` on a result action — and the window stays
visible (with a "Working…" pill) while your `activate` runs.

In Rust, `ActivateOutput::form(...)`, `::copy(...)`, `::open(...)`,
`::toast(...)` and `FormField::new(key, label, kind).required().placeholder(…)`
build these without hand-writing JSON.

### `preview` (optional)

Called lazily when the user selects one of your results (`[limits]
preview_timeout_ms`, default 500 ms). Return one of the declarative content
types; the host renders it natively:

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

#### Streaming previews

`preview` may call `guest::emit_preview_update(&content)` before it returns to
replace what's rendered — re-send the FULL content each time (the host swaps it
wholesale), which is the right shape for token-by-token LLM output. Raise
`preview_timeout_ms` to give the stream room. `emit_preview_update` returns
`false` when the selection moved — stop and return.

### The kv-as-cache pattern

Two good recipes for network extensions:

- **`query` (api 3+):** hit the network directly in the async tier and stream
  results in. Optionally keep a `search` that serves a kv cache instantly, then
  emit the fresh result from `query` with the same `id` to replace it.
- **`refresh` + kv:** for data that's shared across queries or slow to warm —
  `refresh` fetches over HTTP and writes kv (timestamped with `now_ms`), and
  `search` reads only kv so it is always instant.

Never do network in `search`; the tight keystroke budget will cancel it — use
`query` or `refresh`.

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

### Secret settings (API keys)

`type = "secret"` renders a masked input and stores the value in the system
keyring (`org.freedesktop.secrets` — GNOME Keyring, KWallet, KeePassXC),
**never** in `config.toml` and never in the log store. Read it exactly like any
other setting:

```rust
let key = setting_str("api_key")?;   // the stored secret, or None if unset
```

Notes:

- Declaring any secret setting is consent-relevant: it shows a `secrets
  (keyring)` chip before enable/install, and adding the first secret setting to
  an installed extension triggers re-consent. Paired with `network`, the chips
  sit side by side — the user can see the extension can send the key somewhere.
- Secrets are write-only from the UI: they can be set, replaced, or cleared,
  never read back into the settings form.
- No Secret Service daemon → the field is disabled with a clear message and
  `settings_get` returns `None`. Handle an absent optional secret gracefully.
- **Residual risk:** your extension receives the raw value. Do not
  `log_message` it — anything you log is visible in the log viewer and on
  stderr. Portunus never writes the secret anywhere itself.

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
| `emit_results(batch) -> {cancelled}` | none | stream partial results; valid only inside `query` |
| `emit_preview(content) -> {cancelled}` | none | stream a preview update; valid only inside `preview` |

SDK wrapper names: `kv_read`, `kv_write`, `kv_keys`, `kv_remove`, `clipboard`,
`now`, `open`, `debug`, `setting` / `setting_str` / `setting_bool` / `setting_num`,
`emit`, `emit_preview_update`.

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
  **Three consecutive failures pause the extension** behind an escalating
  cooldown (30 s → 2 min → 10 min) — it retries automatically, and a Rescan
  clears the pause immediately; the error shows in your card's log viewer
  (check there first when "my extension returns nothing").
- Result ids are namespaced `ext:<name>:<your-id>` by the host. Your local id is
  opaque and may contain anything, including colons.
- Frecency: results the user activates rank higher in later searches
  automatically, keyed by your stable result id.

## Versioning

`api` in the manifest is the wire-contract major. The host refuses to load
extensions targeting a different major and shows why. Additive changes (new
optional fields, new preview types, new effects) do not bump the major; field
removals or semantic changes do. The effects added within v4 (`show_form`,
`show_toast.level`, `hide`/`keep_open`/`refresh_results`/`paste`) are
additive: older extensions keep working unchanged, but an extension returning
the new effects needs a Portunus build that knows them. **v4 broke from v3:** removed `[trigger]` in
favor of one or more `[[commands]]` entries (each a searchable launcher entry
found by fuzzy-matching its title and `keywords`), removed prefix/alias
triggering entirely (commands are opened as scopes, not invoked by a typed
prefix; `SearchInput`/`QueryInput` dropped the `trigger` field), and
`SearchInput`/`QueryInput`/`ActivateInput`/`PreviewInput` gained a `command`
field naming which one is running. No compatibility shim — migrate `[trigger]`
to `[[commands]]`, rename `prefixes` to `keywords`, and set `api = 4`.
**v3 broke from v2:** added the optional async `query` export with
`emit`-based streaming, streaming previews via `emit_preview_update`, `secret`
settings, and the `query_timeout_ms` / `preview_timeout_ms` limits. (v2 broke
from v1: `actions` became structured objects, `activate` returns effects,
`search` input gained `raw_query`.)

## Examples

- `examples/extensions/emoji/` - offline: a single scope command with search
  keywords, structured actions, activate effects (no permissions), settings,
  browse state, preview.
- `examples/extensions/cheatsh/` - network: an async `query` that streams the
  live sheet in over `search`'s instant kv cache, background refresh, search
  keywords, open-url/copy effects, HTML preview.
- `examples/extensions/gh/` - full breadth: three `[[commands]]` (two `scope`
  search commands plus an `action` command), secret setting (PAT in the
  keyring), dual-endpoint streaming `query` (repos + issues) merged over a kv
  repo cache by id, streaming Metadata→Markdown previews, clone-command copy
  effects, a `show_form` flow ("New Issue…" → form → POST → success toast +
  open), daily background cache warm.

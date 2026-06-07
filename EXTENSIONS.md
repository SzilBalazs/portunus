# Portunus Extensions

Portunus can be extended with sandboxed WebAssembly modules. An extension is a
search provider: it receives the launcher query, returns results, and handles
activation (Enter) and an optional preview panel — all through a small, versioned
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
api = 1                        # REQUIRED — wire API major; unknown majors are rejected
name = "emoji"                 # must match the directory name; [a-zA-Z0-9_-] only
version = "0.1.0"
description = "Search and copy emoji"
author = "you"
kinds = ["ext-emoji"]          # result kind; must start with "ext-"; defaults to "ext-<name>"
entry = "extension.wasm"       # default; no path separators

[permissions]
network = ["api.github.com"]   # exact hosts only — wildcards are rejected; omit for none
kv = true                      # per-extension key-value storage
clipboard = true               # clipboard_write host function

[limits]
search_timeout_ms = 100        # clamped to [10, 500]
activate_timeout_ms = 2000     # clamped to [10, 10000]
```

**Permissions are self-declared** — they bound what your extension *can* do, and
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
    "relevance": 87.5,            // 0–100, higher = better
    "actions": ["copy"]           // optional; first = default on Enter
} ] }
```

- Relevance is your only ranking input — the host maps 0–100 into its internal
  score space. Out-of-range values are clamped.
- At most 200 results are taken per query; titles/subtitles are clamped to 2 KB.
- **`search` runs on the keystroke path with a hard ~150 ms budget. Never do
  network I/O in `search`.** Overruns are cancelled and count as failures.

### `activate` (required for actionable results)

```jsonc
// in — the result EXACTLY as you returned it, plus the chosen action (or null)
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
{ "type": "markdown", "content": "## GFM markdown (raw HTML is not rendered)" }
{ "type": "metadata", "items": [ { "label": "Version", "value": "1.2.0" } ] }
{ "type": "image",    "mime": "image/png", "data_base64": "..." }   // ≤ 1 MB; png/jpeg/gif/webp only
{ "type": "list",     "items": [ { "title": "...", "subtitle": "..." } ] }
```

## Host functions

Importable from the `extism:host/user` namespace; the Rust SDK wraps them:

| Function | Permission | Notes |
|---|---|---|
| `kv_get(key) -> Option<String>` | `kv` | per-extension namespace |
| `kv_set(key, value)` | `kv` | 10 MB quota per extension |
| `clipboard_write(text)` | `clipboard` | wl-copy under the hood |

**HTTP** has no custom host function — use Extism's built-in HTTP
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

`portunus --reload-extensions` force-reloads the wasm bytes of every extension —
that command is your hot-reload loop. The Rescan button in Settings → Extensions
does the same.

Any language with an [Extism PDK](https://extism.org/docs/concepts/pdk)
(Go/TinyGo, Zig, C, JS via the JS PDK, …) works too — implement the same JSON
exports and import the host functions above.

## Sandbox, limits, failure behavior

- No filesystem, no process spawning, no network beyond the manifest allowlist.
- 64 MB linear memory cap; `extension.wasm` must be ≤ 32 MB.
- Each call runs under a wall-clock watchdog (cancelled via wasmtime epoch
  interruption). A trapped/timed-out call returns empty results — it never
  crashes or stalls the launcher.
- After a failed call the instance is rebuilt from disk before the next one.
  **Three consecutive failures bench the extension for the session**; the error
  shows in Settings → Extensions (your primary debugging signal — check there
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

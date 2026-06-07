# emoji — example Portunus extension

Searches a small emoji set by name/keyword; Enter copies the emoji to the
clipboard. Demonstrates `search`, `activate`, `preview`, and the
`clipboard_write` host function.

## Build

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
```

## Install

```bash
DEST=~/.local/share/portunus/extensions/emoji
mkdir -p "$DEST"
cp target/wasm32-unknown-unknown/release/emoji.wasm "$DEST/extension.wasm"
cp manifest.toml "$DEST/"
portunus --reload-extensions   # or restart Portunus
```

Then open Settings → Extensions, review the permissions, and enable it.
Type `smile` in the launcher.

## Iterate

```bash
cargo build --release --target wasm32-unknown-unknown \
  && cp target/wasm32-unknown-unknown/release/emoji.wasm "$DEST/extension.wasm" \
  && portunus --reload-extensions
```

See `EXTENSIONS.md` at the repository root for the full authoring guide.

# SuperMD plugin template

A SuperMD plugin is a WebAssembly component plus a `plugin.toml`
manifest, dropped into `~/.supermd/plugins/<name>/`.

## Build

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

## Install

```sh
mkdir -p ~/.supermd/plugins/my-plugin
cp target/wasm32-wasip2/release/*.wasm ~/.supermd/plugins/my-plugin/plugin.wasm
cp plugin.toml ~/.supermd/plugins/my-plugin/
```

Restart SuperMD. Fences you claimed render as live blocks; commands
you declared appear in the command palette (⌘⇧P / Ctrl+Shift+P).

## What plugins can do

Everything is a function call from the host — no processes, no
background execution, no plugin-drawn pixels. Compute times out after
2 seconds. Surfaces (declare in `plugin.toml` what you use):

- `fences = ["lang"]` + `render_block` — fenced blocks → SVG widgets.
- `[[commands]]` + `run_command` — palette commands.
- `[[inline]]` + `render_inline` — inline pattern replacements.
- `[[decorations]]` — regex → style token, no wasm at all.
- `formats = true` + `format_document` — "Format: <name>" + on-save.
- `paste = true` + `process_paste` — transform pasted text.
- `[[exports]]` + `export_document` — return files as bytes; the HOST
  shows the save dialog and writes. Plugins never see paths.

## Capabilities

Declared in the manifest, granted by the user:

- *(none)* — pure compute; installs silently.
- `capabilities = ["workspace-read"]` — the workspace root appears
  read-only at `/workspace` after a one-time consent.
- `capabilities = ["net"]` — enables `host_api::fetch` (https only,
  5 s / 2 MB / 4 fetches per call). Grants nothing by itself: every
  domain prompts its own one-time consent banner on first fetch.
  Net-capable paste plugins run asynchronously after the paste.

## Troubleshooting

- "failed: <name>" in the palette → the manifest didn't parse or the
  wasm didn't link; run SuperMD from a terminal to see the reason.
- A link error mentioning `wasi:` imports → your crate pulled in
  code needing WASI (files, clocks, randomness). Phase 1 plugins must
  be pure; remove the dependency.

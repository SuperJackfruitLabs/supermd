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

## What plugins can do (Phase 1)

Pure functions only — text in, text out. No filesystem, no network,
no processes. `render_block` receives the fence language, its source,
and the active theme palette; return SVG. `run_command` receives the
document and selection; return a replacement. Calls time out after
2 seconds.

## Troubleshooting

- "failed: <name>" in the palette → the manifest didn't parse or the
  wasm didn't link; run SuperMD from a terminal to see the reason.
- A link error mentioning `wasi:` imports → your crate pulled in
  code needing WASI (files, clocks, randomness). Phase 1 plugins must
  be pure; remove the dependency.

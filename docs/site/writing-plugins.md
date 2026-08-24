# Writing a Plugin

A SuperMD plugin is a WebAssembly component plus a `plugin.toml` manifest, in a folder under `~/.supermd/plugins/`. This page walks the whole path from template to installed plugin. You'll need Rust and about ten minutes.

## Start from the template

Copy [`plugins/template/`](https://github.com/SuperJackfruitLabs/supermd/tree/master/plugins/template) from the repo. It's a complete, buildable plugin where every surface returns "not implemented" — you fill in the ones you want.

## The manifest

`plugin.toml` declares what your plugin contributes. A complete example using several surfaces:

```toml
name = "my-plugin"
version = "0.1.0"

# Render ```mylang fences as SVG.
fences = ["mylang"]

# Command palette entries.
[[commands]]
id = "my-plugin.hello"
title = "Say Hello"

# Inline pattern replacement (regex, one match consumed whole).
[[inline]]
id = "shortcode"
pattern = ":([a-z]+):"

# Regex → style highlighting, no code involved.
[[decorations]]
pattern = "\\b(TODO|FIXME)\\b"
style = "accent"        # accent | muted | strong | highlight

formats = true          # adds "Format: my-plugin" to the palette
paste = true            # transform pasted text

# Export formats: "Export: HTML" in the palette.
[[exports]]
id = "html"
name = "HTML"
extension = "html"

# Rendered views for file types (content in, markdown out).
[[viewers]]
extensions = ["csv"]

# Status-strip text.
[[widgets]]
id = "words"

# "New: Daily Note" in the palette.
[[templates]]
id = "daily"
name = "Daily Note"

# Run on every save.
hooks = ["save"]
```

Declare only what you use — the host never calls surfaces you didn't declare.

## Implement the surfaces

Each manifest entry maps to one exported function in `src/lib.rs`:

| Manifest | Export | Signature (in, out) |
| -------- | ------ | ------------------- |
| `fences` | `render-block` | language + source + theme → SVG |
| `[[commands]]` | `run-command` | id + document + selection → replacement |
| `[[inline]]` | `render-inline` | pattern id + matched text → replacement text |
| `formats` | `format-document` | document → document |
| `paste` | `process-paste` | text → optional replacement |
| `[[exports]]` | `export-document` | document + format + theme → files (bytes) |
| `[[viewers]]` | `render-view` | filename + content → markdown |
| `[[widgets]]` | `status-text` | document → one line of text |
| `[[templates]]` | `render-template` | id + date/context → filename + content |
| `hooks` | `on-save` | path + document → optional replacement |

Everything is a pure function call: data in, data out, errors as strings. Returning an error shows a message; it never crashes the editor.

## Build and install

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
mkdir -p ~/.supermd/plugins/my-plugin
cp target/wasm32-wasip2/release/*.wasm ~/.supermd/plugins/my-plugin/plugin.wasm
cp plugin.toml ~/.supermd/plugins/my-plugin/
```

Run **Reload Plugins** from the palette. Your fences render, your commands appear.

If your plugin needs to read the workspace or fetch from the web, read [Capabilities](capabilities.md). For syntax-highlighting plugins, see [Grammar Plugins](grammars.md) — they're data-only, no Rust required.

## Troubleshooting

- **"failed: …" in the palette** — the manifest didn't parse or the wasm didn't link; run SuperMD from a terminal to see the full reason.
- **A link error mentioning `wasi:` imports** — a dependency pulled in code needing files, clocks, or randomness. Plugins are pure by default; remove the dependency or gate the feature.
- **Your surface returns an error every time** — check that the manifest declares it; undeclared surfaces are never called.

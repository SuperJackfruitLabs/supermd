# SuperMD

**[supermd.app](https://supermd.app)** · [Download](https://github.com/SuperJackfruitLabs/supermd/releases/latest)

A native, GPU-rendered Markdown editor for macOS — the writing feel of
Bear/Lettera on the engine philosophy of Zed. Plain CommonMark on disk,
always.

Built in Rust on [GPUI](https://www.gpui.rs), the UI framework behind
[Zed](https://zed.dev).

## What it does

- **Hybrid WYSIWYG** — syntax markers hide when your cursor is
  elsewhere and reveal in place when you touch them: `**bold**`,
  headings, lists (`•`), quotes, links, task checkboxes (click to
  toggle).
- **Live blocks** — tables render as real tables and whole-line images
  render inline while you edit; touch them and they dissolve back into
  raw source. Code fences read as clean panels with tree-sitter
  highlighting.
- **A real workspace** — folder sidebar with Seti UI file icons, tabs,
  outline panel, fuzzy file finder (⌘P), find in file (⌘F), pretty
  preview toggle (⌘E), image viewer tabs, light/dark following the
  system.
- **78 languages highlighted** via tree-sitter (inkjet + Helix
  queries, plus an extras registry), in fenced blocks and standalone
  files alike. Code files get a real code editor: monospace, full
  width, line-number gutter, auto-indent.
- **Themes** — six built-in (Paper, Graphite, Solarized ×2, Nord,
  Gruvbox Dark), live picker on ⌘T, custom themes as TOML files in
  `~/.supermd/themes/`. Your light and dark picks follow the system
  appearance automatically.
- **Safe by default** — autosave with atomic writes, per-session
  backups in `~/.supermd/backups`, external-change detection that never
  silently clobbers anything, and live reload of clean buffers when
  files change on disk.

The editing core (buffer, selection, undo, styling spans, display
transform, projection) is pure Rust under a test suite; the GPU shell
stays thin.

## Building

Requires Rust (stable) and macOS. No full Xcode needed — Metal shaders
compile at runtime.

```sh
cargo run            # opens the current directory as a workspace
cargo run -- <path>  # open a file or folder
cargo test
```

## Status

Early and moving fast — built as a working editor first, a product
second. macOS only for now (GPUI is cross-platform; other platforms
untested).

## License

Apache-2.0. Vendored third-party assets keep their own licenses (see
`assets/`).

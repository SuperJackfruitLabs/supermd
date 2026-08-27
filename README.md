# SuperMD

[![CI](https://github.com/SuperJackfruitLabs/supermd/actions/workflows/ci.yml/badge.svg)](https://github.com/SuperJackfruitLabs/supermd/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/SuperJackfruitLabs/supermd)](https://github.com/SuperJackfruitLabs/supermd/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/SuperJackfruitLabs/supermd/total)](https://github.com/SuperJackfruitLabs/supermd/releases)
[![Coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fgist.githubusercontent.com%2Frakeshgangwar%2F9fb5226b83eda4ae8cb0568e7bc7755f%2Fraw%2Fsupermd-coverage.json&cacheSeconds=1800)](https://github.com/SuperJackfruitLabs/supermd/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)

**Markdown that gets out of the way.** A native, GPU-rendered Markdown
editor — the writing feel of Bear/Lettera on the engine philosophy of
Zed. Plain CommonMark on disk, always.

**[supermd.app](https://supermd.app)** ·
**[Download](https://github.com/SuperJackfruitLabs/supermd/releases/latest)** ·
built in Rust on [GPUI](https://www.gpui.rs), the UI framework behind
[Zed](https://zed.dev)

![SuperMD editing a workspace — hybrid WYSIWYG text with markers hidden, a live table, syntax-highlighted code and a [[wiki link]], with the sidebar, outline panel, titlebar panel toggles and status bar](docs/assets/screenshot.png)

## Highlights

- **Hybrid WYSIWYG** — syntax markers hide when your cursor is
  elsewhere and reveal in place when you touch them: `**bold**`,
  headings, lists (`•`), quotes, links, task checkboxes (click to
  toggle).
- **Notes that know each other** — type `[[` and complete to any note
  in the workspace; follow links with ⌘-click (unresolved links create
  the note); backlinks with context and `#tags` in the knowledge panel
  (⌘3); a native force-directed **graph view** (⌘⇧G) of the whole
  workspace. Renaming or moving a note rewrites every link pointing at
  it — the graph never breaks. All computed from your plain files: no
  database.
- **Writing that keeps up** — select text and a floating toolbar
  appears (bold, italic, code, strike, link, heading, quote — every
  action a toggle, ⌘B/⌘I from the keyboard); lists continue on Enter
  and indent with Tab; Tab hops table cells while the pipes align
  themselves; paste an image and it lands in `assets/` with the link
  inserted.
- **Live blocks** — tables render as real tables and whole-line images
  render inline while you edit; touch them and they dissolve back into
  raw source. Code fences read as clean panels with tree-sitter
  highlighting.
- **Live diagrams** — ` ```mermaid ` fences render as native,
  theme-matched diagrams (merman — pure Rust, no browser). Click one
  to edit its source; click away and it's a picture again. All 35
  mermaid diagram families.
- **A real workspace** — folder sidebar with Seti UI file icons
  (gitignore-aware: no `node_modules`/`target` noise) and full
  keyboard file management (rename, create, move via fuzzy picker,
  delete to trash — open tabs follow along), tabs with VS Code-style
  preview behavior (arrow through files in the sidebar, pin with Enter
  or a double-click), outline panel, fuzzy file finder (nucleo-scored
  with match highlighting), find in file, pretty preview toggle, image
  viewer tabs, light/dark following the system.
- **Search in workspace** — ⌘⇧F streams ripgrep-powered results into a
  two-pane overlay: matches grouped by file, live preview centered on
  the hit, Enter jumps straight to the line.
- **78 languages highlighted** via tree-sitter (inkjet + Helix
  queries, plus an extras registry), in fenced blocks and standalone
  files alike. Code files get a real code editor: monospace, full
  width, line-number gutter, auto-indent.
- **Show Changes** — diff the open file against git HEAD, with
  word-level marks rendered in the editor's own typography (added words
  on a green wash, deleted words struck through in red, inline in the
  flow). Code files get line diffs with a diff-aware gutter; modified
  files get a dot in the sidebar. Pure-Rust git (gix), read-only —
  SuperMD never writes to your repo.
- **Themes** — eight built-in (Jackfruit ×2, Paper, Graphite,
  Solarized ×2, Nord, Gruvbox Dark), live picker, custom themes as
  TOML files in `~/.supermd/themes/`. Your light and dark picks follow
  the system appearance automatically — and **flux** (opt-in) follows
  the sun: crossfade to your dark theme at sunset and warm every color
  toward candle-light, sunrise/sunset computed offline from the NOAA
  solar equations. No location permission, no network.
- **Extensible** — plugins are WebAssembly components: block renderers
  (` ```dot ` graphviz, ` ```chart `), palette commands (⌘⇧P), inline
  renderers (`:tada:` → 🎉, `{{2 km + 300 m}}` calculated in place),
  decoration rules (TODO highlighting needs zero code — just a
  manifest), formatters, paste processors (CSV → table), exporters,
  file viewers (CSV tables, Jupyter notebooks), status widgets,
  templates, save hooks, and tree-sitter grammars. Sandboxed hard: no
  filesystem or network without explicit per-plugin (and per-domain)
  consent, no processes; a hung plugin is cut off in 2 s. Fourteen
  ship first-party — eight pre-installed, the rest one **Install
  Plugins…** away — and each doubles as a working example. Author your
  own from `plugins/template/` in ~20 lines of Rust.
- **Findable** — every command is declared once and reaches the menu
  bar, the ☰ menu off macOS, the ⌘/ sheet and the docs from that one
  place, so they cannot drift apart. Full File / Edit / Format / View /
  Go / Tools menus, panel toggles and a `+` in the chrome, a status bar
  carrying plugin widgets alongside flux and graph toggles, and an
  About dialog that checks for updates on demand. Panels sit on
  ⌘1/⌘2/⌘3; the shortcut map has a written scheme so new bindings have
  a home.
- **Update aware** — a quiet launch-time check against GitHub releases
  shows an "update available" pill in the titlebar when a newer version
  ships; clicking opens the download page. Nothing phones home beyond
  that one request, and failures are silent.
- **Safe by default** — autosave with atomic writes, per-session
  backups in `~/.supermd/backups`, external-change detection that never
  silently clobbers anything, and live reload of clean buffers when
  files change on disk.

The editing core (buffer, selection, undo, styling spans, display
transform, projection) is pure Rust under a test suite; the GPU shell
stays thin.

### The six shortcuts worth learning first

| Shortcut | Does |
| -------- | ---- |
| ⌘O | open a file or folder |
| ⌘P | jump to any file by fuzzy name |
| ⌘⇧F | search inside every file in the workspace |
| ⌘3 | backlinks, tags, and the local graph for the open note |
| ⌘⇧D | see what you've changed since your last git commit |
| ⌘T | pick a theme |

Everything else lives in ⌘/. On Linux and Windows, read ⌘ as Ctrl.

## Installing

Grab the build for your platform from the
[latest release](https://github.com/SuperJackfruitLabs/supermd/releases/latest):

**macOS** — download the DMG, drag **SuperMD** onto **Applications**, done. If you
launch it straight from the disk image instead, SuperMD notices and
offers to move itself. Releases are signed and notarized — no
Gatekeeper warnings. Double-click any `.md` file to open it ("Open
With → SuperMD" for other text), drop a folder on the window or Dock
icon to open a workspace, and SuperMD reopens your last workspace on
launch (File → Open Recent has the rest).

**Linux** *(new)* — install the `.deb`, or unpack the tarball and run
`./install.sh` (installs to `~/.local`, registers the .desktop entry
and markdown association). Wayland and X11 both supported.

**Windows** *(new)* — run `SuperMD-Setup-<version>.exe` (Start Menu
entry, optional `.md` association, uninstaller) or use the portable
zip. Builds are not yet code-signed, so SmartScreen shows one
"unrecognized app" prompt — More info → Run anyway.

## Building from source

Requires Rust (stable). On macOS no full Xcode is needed — Metal
shaders compile at runtime. On Linux install the build deps first:

```sh
sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig1-dev \
  libfreetype6-dev
```

```sh
cargo run            # opens an empty workspace (Open Folder… to pick one)
cargo run -- .       # open the current directory as a workspace
cargo run -- <path>  # open a file or folder
cargo test
```

Test coverage (CI enforces a 90% line-coverage floor) via
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov):

```sh
cargo install cargo-llvm-cov   # once; needs `rustup component add llvm-tools-preview`
cargo llvm-cov                 # summary table
cargo llvm-cov --html --open   # browsable line-by-line report
```

## Status

Early and moving fast — built as a working editor first, a product
second. macOS is the primary platform; Linux and Windows builds are
new — [feedback and issues](https://github.com/SuperJackfruitLabs/supermd/issues)
welcome.

## License

Apache-2.0. Vendored third-party assets keep their own licenses (see
`assets/`).

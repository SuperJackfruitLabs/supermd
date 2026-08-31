# Plugins

SuperMD is extensible with sandboxed WebAssembly plugins. Eight come pre-installed, and the rest are one keystroke away, and anyone can [write their own](writing-plugins.md).

## What plugins can add

- **Rendered blocks** — claim a fence language and draw it (the Graphviz plugin renders ` ```dot ` fences).
- **Commands** — entries in the command palette (**⌘ ⇧ P**), like *Insert Table of Contents*.
- **Inline replacements** — patterns like `:tada:` rendered as 🎉, revealed as raw text when your cursor touches them.
- **Formatters** — "Format: …" palette commands, optionally run on every save.
- **Paste transforms** — pasted CSV becomes a Markdown table; a pasted link can gain its page title.
- **Exporters** — "Export: HTML" renders your document to a file you choose.
- **File viewers** — open non-Markdown files as rendered views (CSV as tables); **⌘ E** switches to the raw source.
- **Status widgets** — word count and reading time in the corner of the editor.
- **Templates** — "New: Daily Note" creates today's journal file in your workspace.
- **Save hooks** — the table-of-contents plugin refreshes its markers every time you save.
- **Syntax highlighting** — new languages for fences and files (GraphQL ships this way).

## The bundled plugins

| Plugin | What it does |
| ------ | ------------ |
| **dot** | Renders ` ```dot ` and ` ```graphviz ` fences as themed diagrams (pure Rust, no Graphviz install needed) |
| **toc** | *Insert Table of Contents* / *Update Table of Contents* palette commands, and auto-refreshes `<!-- toc -->` markers on every save |
| **emoji** | Renders `:tada:`-style shortcodes as emoji inline (1,900+ codes); the raw text reveals when your cursor touches it |
| **tidy** | *Format: tidy* — smart quotes and dashes, collapses extra blank lines, trims trailing whitespace; also converts pasted CSV/TSV into a Markdown table |
| **todo-marks** | Highlights TODO, FIXME, and NOTE in prose — a manifest-only plugin with no code at all |
| **url-title** | Paste a bare `https://` link and it becomes `[Page Title](url)` — asks consent per website, applied only if you haven't kept typing |
| **html-export** | *Export: HTML* — your document as a single self-contained HTML file, styled with your current theme |
| **word-count** | Word count and reading time in the editor's status corner |
| **csv-view** | Opens `.csv` / `.tsv` files as rendered tables; **⌘ E** switches to the raw text |
| **daily-note** | *New: Daily Note* — creates (or reopens) `journal/<today>.md` in your workspace |
| **graphql** | Syntax highlighting for ` ```graphql ` fences and `.graphql` / `.gql` files — a [grammar plugin](grammars.md) |
| **calc** | Calculations in prose — `{{2 km + 300 m}}` shows its result inline, raw on touch |
| **chart** | ` ```chart ` fences with `label: value` lines render as themed bar/line charts |
| **ipynb-view** | Opens Jupyter notebooks (`.ipynb`) as rendered documents — code, outputs, and prose |

These live in [`plugins/`](https://github.com/SuperJackfruitLabs/supermd/tree/master/plugins) in the repo — each one doubles as a working example for the surface it uses.

## Installing plugins

The first eight in the table above come **pre-installed** — SuperMD seeds them on first launch. Delete any from your plugins folder and it stays gone; SuperMD never re-adds a plugin you removed, and never touches one you've modified.

For the rest (and to re-install anything you deleted), open the command palette and run **Install Plugins…** — pick from the catalog and it downloads from the SuperMD repository, verifies, and installs in one step.

Manual installation also works: a plugin is just a folder. Drop it into:

- macOS / Linux: `~/.supermd/plugins/`
- Windows: `%USERPROFILE%\.supermd\plugins\`

then run **Reload Plugins** from the palette (or restart). If a plugin fails to load, the reason appears at the bottom of the command palette — a broken plugin never breaks the app.

### On the Mac App Store build

The App Store version of SuperMD runs inside Apple's sandbox, which changes how plugins arrive — every plugin still works the same once installed.

- **All fourteen plugins ship inside the app.** Nothing has to be downloaded to use them.
- **There is no in-app catalog to browse.** Browse plugins on the website and click **Install** there: the link hands the plugin's name to SuperMD, which asks you to confirm — naming the plugin and anything it wants access to — before installing.
- **Import Plugin…** (Tools menu) installs a plugin archive you downloaded yourself. This route works in every build.
- **Grammar plugins are unavailable.** A plugin that adds syntax highlighting for a new language (the GraphQL plugin is the only one today) needs a compiler Apple's sandbox does not permit. GraphQL code renders as plain text there; every other plugin is unaffected.

Your plugins folder lives inside the app's container rather than at `~/.supermd/`. Reach it with **Reveal Settings Folder** from the Help menu.

## The safety model

Plugins run inside a WebAssembly sandbox with **no access to anything by default** — no files, no network, no environment, no clock.

Two capabilities exist, and both ask you first:

- **Workspace reading** — a plugin that wants to read files in your open folder triggers a one-time consent banner. Granting gives it read-only access to that folder and nothing else.
- **Network access** — a plugin that wants to fetch from the web asks per website: the first request to a domain shows a banner naming it. Grants are remembered and revocable, and requests are limited (HTTPS only, small responses, strict timeouts).

Some things plugins can **never** do, with no permission to ask for: run programs, write files anywhere they choose (exports and templates only go where *you* pick), draw their own interface, or run in the background. And a plugin that hangs or crashes is simply cut off — after two seconds it's interrupted, the document is untouched, and the app carries on.

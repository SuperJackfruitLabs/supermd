# Plugins

Fourteen plugins ship with SuperMD. Each is a WebAssembly component
with no access to anything by default — no files, no network, no
processes. This page exercises every one of them.

If a plugin is not installed, its syntax below simply stays as written.
That is the honest failure mode: plain text, never a crash.

#guide #plugins

<!-- toc -->
<!-- /toc -->

The two markers above are the `toc` plugin's. Run **Update table of
contents** from the palette (**⌘⇧P**) and the headings of this file
appear between them; it refreshes on save from then on.

## calc — arithmetic in prose

The `calc` plugin evaluates `{{ ... }}` inline and shows the result
without touching your source.

- A walk of {{2 km + 300 m}} before breakfast.
- {{1024 * 1024}} bytes in a mebibyte.
- {{ (17 + 25) / 2 }} is the mean of seventeen and twenty-five.
- Unit-aware: {{90 min in hours}}.
- Deliberately wrong, to see the error render: {{2 km + blue}}

## emoji — shortcodes

Shipping :rocket: with tests :white_check_mark: and a bug :bug: that
turned out to be a feature :sparkles:

## todo-marks — markers in prose

The words below are highlighted wherever they appear, including here in
running text.

TODO: measure the agent layer's memory before shipping it.
FIXME: the watcher's symlink filter checks the leaf only.
NOTE: the plugin host has never been tested under Pulley in CI.

## chart — charts from `label: value`

```chart
type: bar
title: Tests per release
0.0.12: 512
0.0.13: 604
0.0.14: 706
0.0.15: 729
```

```chart
type: line
title: Resident memory (MB)
launch: 84
indexed: 129
settled: 147
```

## dot — Graphviz

Covered on [[Diagrams]], which is where it belongs, but here is a small
one so this page stands alone:

```dot
digraph { rankdir=LR; note -> link -> note; }
```

## graphql — highlighting for GraphQL

```graphql
mutation CreateNote($path: String!, $body: String!) {
  createNote(path: $path, body: $body) {
    path
    createdAt
  }
}
```

## csv-view and ipynb-view — non-Markdown documents

These two render whole files rather than blocks:

- [Quarterly figures](../Data/quarterly.csv) opens as a table
- [An analysis notebook](../Data/notebook.ipynb) opens as a document

## word-count — the status strip

Look at the bottom of the window: words and reading time for this
document, updating as you type.

## tidy — punctuation and CSV paste

Run **Tidy punctuation** from the palette on this line:

"Straight quotes" become curly, -- becomes an en dash, and ... becomes
an ellipsis.

Copy two columns from a spreadsheet and paste: `tidy` turns the
clipboard into a Markdown table.

## url-title — pasted links gain their title

Paste a URL into this document. The plugin asks for permission for that
site the first time, then replaces the bare address with the page's
title. Decline and the address stays exactly as pasted.

## daily-note — today's journal

Run **Daily note** from the palette. It creates today's file under
`Notes/Daily/`, like [this one](../Notes/Daily/2026-09-06.md).

## html-export — a self-contained page

Run **Export as HTML**. The result is one file with the theme inlined —
no external stylesheet, no network fetch when it is opened.

Back to [[README]]

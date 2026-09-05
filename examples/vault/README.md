# The SuperMD example vault

A real workspace, not a mockup. Every file here is plain CommonMark on
disk — open the folder in SuperMD and edit anything. If something looks
wrong, it *is* wrong, and that is the point: this vault is meant to
break visibly when the app regresses.

Open it with `cargo run -- examples/vault`, or **File → Open Folder**.

## The guide

- [[Editing]] — markers that fold away, the selection toolbar, lists
- [[Links and notes]] — wiki links, backlinks, tags, the graph
- [[Tables]] — the live table widget
- [[Code]] — fenced blocks across a dozen languages
- [[Diagrams]] — Mermaid and Graphviz, rendered
- [[Images]] — local assets, paste-to-file, broken links
- [[Plugins]] — every bundled plugin, exercised

## Notes that link to each other

- [[Meeting 2026-09-02]] — the shape of an ordinary working note
- [[Reading list]] — tags and outbound links
- [Daily note](Notes/Daily/2026-09-06.md) — what `daily-note` creates

## Files that are not Markdown

SuperMD opens these with a viewer-plus editor: syntax highlighting, a
gutter, auto-indent, and deliberately no language server.

- [A Rust module](Code/sample.rs)
- [A TOML config](Code/config.toml)
- [A JSON manifest](Code/package.json)
- [A YAML pipeline](Code/deploy.yaml)
- [A Python script](Code/script.py)
- [A CSV table](Data/quarterly.csv) — rendered by the `csv-view` plugin
- [A Jupyter notebook](Data/notebook.ipynb) — rendered by `ipynb-view`

## Things that should be broken

Two links below point at notes that do not exist. Clicking one creates
it — that is the designed behaviour, not a bug:

- [[Ghost note]]
- [[Another missing page]]

#example #vault

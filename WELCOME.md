# Welcome to supermd

A **pixel-perfect** Markdown reader and editor, built native in Rust on [GPUI](https://gpui.rs) — the GPU-accelerated framework behind Zed.

This document is rendered by supermd itself. Everything you see below is parsed from plain CommonMark and drawn with real text runs: *italics*, **bold**, ***both at once***, ~~strikethrough~~, and `inline code`.

## Why another editor?

> The best writing tools disappear. Bear and Lettera proved Markdown can be beautiful; Zed proved native can be fast. supermd aims for both.

There are three pillars:

1. **Reading** — typography good enough to live in all day
2. **Writing** — hybrid WYSIWYG where syntax hides until you need it
3. **Code** — tree-sitter highlighting, outline, and a fuzzy finder

## Code, first-class

Fenced blocks render in a proper monospace panel:

```rust
fn main() {
    let doc = markdown::parse(include_str!("WELCOME.md"));
    println!("{} blocks parsed", doc.blocks.len());
}
```

## The roadmap

| Phase | Deliverable | Status |
| ----- | ----------- | ------ |
| 0 | Read-only renderer | Done |
| 1 | Workspace: sidebar, tabs, TOC | Done |
| 2 | Styled-source editing + autosave | Done |
| 3 | Hybrid WYSIWYG | Next |

### Near-term tasks

- [x] GPUI window with themed rendering
- [x] CommonMark block model
- [x] Tree-sitter syntax highlighting in code blocks
- [x] Folder-as-workspace sidebar
- [x] In-place styled editing with autosave (⌘E toggles preview)
- [ ] Hide syntax markers away from the cursor (Phase 3)

---

Nested structure works too:

- Lists can hold paragraphs and other blocks
  - Including *nested* lists
  - And `inline code` inside items
- > Even a quote inside a list item

*Set in San Francisco with Menlo for code — rendered at 120fps by your GPU.*

# Welcome to SuperMD

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
| 3 | Hybrid WYSIWYG (markers hide) | Done |
| 4 | Tables, images, doc projection | Done |
| 5 | Polish: ✓ checkboxes, ⌘F, watching, chrome | Done |
| 6 | Seti icons, 36 languages, image tabs | Done |
| 7 | Pre-release UX: focus mode, ⌘/, nav | Done |
| 8 | Code-first: gutter, 77 langs (inkjet) | Done |
| 9 | Themes (⌘T), full lang mapping, XML | Done |

This very table is a live widget now — click a row to edit its raw pipes.

![supermd banner](docs/assets/banner.png)

### Near-term tasks

- [x] GPUI window with themed rendering
- [x] CommonMark block model
- [x] Tree-sitter syntax highlighting in code blocks
- [x] Folder-as-workspace sidebar
- [x] In-place styled editing with autosave (⌘E toggles preview)
- [x] Hide syntax markers away from the cursor — click around this line's **bold** and `code` to watch them fold
- [x] Real tables and inline images while editing; fence lines collapse too
- [x] Click these checkboxes to toggle them — ⌘Z undoes
- [ ] Try ⌘F to find, ⌘N for a new note, and edit a file in another app to watch it reload
- [ ] In a git repo, hit ⌘⇧D to see what you've changed since the last commit
- [ ] ⌘⇧F searches the whole workspace (focus mode moved to ⌃⌘F)
- [ ] Arrow through the sidebar — files preview in one italic tab; Enter pins them

---

Nested structure works too:

- Lists can hold paragraphs and other blocks
  - Including *nested* lists
  - And `inline code` inside items
- > Even a quote inside a list item

*Set in San Francisco with Menlo for code — rendered at 120fps by your GPU.*

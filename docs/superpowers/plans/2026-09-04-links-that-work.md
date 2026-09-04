# SuperMD 0.0.15 — "Links That Work" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make navigation in SuperMD work the way every other Markdown editor's does — links open, broken links look broken, clicking a widget doesn't lose your place, and you can go back.

**Architecture:** Four separate defects add up to "links don't work". `follow_link_at` routes every link through the knowledge index, which only holds `.md` files and cannot represent a URL. Fixing it means classifying a link before resolving it. Navigation history is new state on `Workspace`, fed by the existing `EditorEvent::OpenPath` path. The scroll-jump fix replaces a `ListState::reset` that discards measured heights. Every new decision lands in a pure, tested function; the GPUI shell only drives it.

**Tech Stack:** Rust, GPUI 0.2.2 (vendored), pulldown-cmark 0.13, inline `#[cfg(test)]` tests, `cargo test`, `cargo llvm-cov`.

**Spec:** No separate design doc — scoped directly from `docs/inspiration/FEATURE-MATRIX.md` §5 (Linking and navigation) and issues #14, #17, #20, #21. The matrix is the argument; this plan is the execution.

## Global Constraints

- **Editing/policy logic is pure Rust under tests; the GPUI shell stays thin.** Every new decision goes in a pure function with inline tests.
- **CI enforces a 90% line-coverage floor** (`cargo llvm-cov`). Every task adds tests alongside its code.
- Tests live inline as `#[cfg(test)] mod tests` beside the code they cover.
- **Both configurations must pass:** `cargo test` and `cargo test --no-default-features --features mas`.
- **`docs/site/shortcuts.md` is generated.** After changing the command table, run `UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table`, commit the result, then `cargo run --example build_docs`.
- **`SuperMD never writes to the user's git repository.**
- Adding a `commands.rs` row with a keystroke requires no count bump — `every_keybinding_parses_and_binds` asserts properties, not a count — but the collision test will reject a keystroke already bound in the same context.

## File Structure

| File | Responsibility |
| --- | --- |
| `src/knowledge.rs` | Add `LinkTarget` classification and widen `resolve` to on-disk files. Pure. |
| `src/editor/mod.rs` | `follow_link_at` dispatches on `LinkTarget`; plain-click follows; `reproject` stops resetting scroll; `find_prev` propagates. |
| `src/editor/spans.rs` | New `StyleKind::BrokenLink`. |
| `src/workspace.rs` | Navigation history (`nav.rs` state driven from here) and the two actions. |
| `src/nav.rs` | **New.** Pure back/forward stack. No GPUI. |
| `src/commands.rs` | Two new rows for Back and Forward. |
| `scripts/check_private_apis.sh` | **New.** Scans a built binary for private Apple symbols. |
| `.github/workflows/` | Wire the private-API scan and the catalog refresh. |
| `docs/internal/performance.md` | **New.** Measured baseline figures. |

---

### Task 1: Classify a link before resolving it

**Files:**
- Modify: `src/knowledge.rs` (add `LinkTarget`, `classify`)

**Interfaces:**
- Consumes: `RawLink { target: String, wiki: bool, range: Range<usize>, context: String }`
- Produces: `pub enum LinkTarget { External(String), Wiki(String), Relative(String) }`; `pub fn classify(link: &RawLink) -> LinkTarget`

`follow_link_at` currently sends every link to `Index::resolve`, which treats a non-wiki target as a relative path. `https://apple.com` becomes `<workspace>/https:/apple.com` and fails. Classification has to happen before resolution, and it is pure, so it is tested on its own.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/knowledge.rs`:

```rust
    fn raw(target: &str, wiki: bool) -> RawLink {
        RawLink { target: target.into(), wiki, range: 0..1, context: String::new() }
    }

    #[test]
    fn classify_separates_external_wiki_and_relative() {
        assert_eq!(classify(&raw("https://apple.com", false)),
                   LinkTarget::External("https://apple.com".into()));
        assert_eq!(classify(&raw("http://localhost:8080", false)),
                   LinkTarget::External("http://localhost:8080".into()));
        assert_eq!(classify(&raw("Note", true)), LinkTarget::Wiki("Note".into()));
        assert_eq!(classify(&raw("./config.toml", false)),
                   LinkTarget::Relative("./config.toml".into()));
    }

    #[test]
    fn classify_treats_other_schemes_as_relative_not_external() {
        // Only http(s) is opened. A note is untrusted content; file://,
        // mailto: and supermd:// must not become one-click actions.
        for t in ["file:///etc/passwd", "mailto:a@b.c", "supermd://install-plugin?name=x"] {
            assert!(matches!(classify(&raw(t, false)), LinkTarget::Relative(_)), "{t}");
        }
    }

    #[test]
    fn a_wiki_link_is_wiki_even_if_it_looks_like_a_url() {
        assert_eq!(classify(&raw("https://x", true)), LinkTarget::Wiki("https://x".into()));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test classify_separates_external`
Expected: FAIL — `cannot find function 'classify'`.

- [ ] **Step 3: Implement**

Add near `RawLink` in `src/knowledge.rs`:

```rust
/// What a link points at. Classification happens before resolution
/// because the index can only answer for workspace files — a URL has no
/// path to look up, and joining it onto the workspace root produces
/// nonsense like `<root>/https:/apple.com`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// An http(s) URL, opened by the platform.
    External(String),
    /// `[[Note]]` — resolved by stem against the index.
    Wiki(String),
    /// A path relative to the containing file.
    Relative(String),
}

/// Sort a link into one of the three kinds.
///
/// Only http and https are treated as external. Other schemes stay
/// relative and therefore fail to resolve, which is deliberate: a
/// document is untrusted content, and `file://` or `supermd://` must not
/// become a one-click action.
pub fn classify(link: &RawLink) -> LinkTarget {
    if link.wiki {
        return LinkTarget::Wiki(link.target.clone());
    }
    let lower = link.target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        LinkTarget::External(link.target.clone())
    } else {
        LinkTarget::Relative(link.target.clone())
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test knowledge::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/knowledge.rs
git commit -m "feat: classify a link before resolving it"
```

---

### Task 2: Resolve links to any workspace file, not only notes

**Files:**
- Modify: `src/knowledge.rs:222` (`Index::resolve`)

**Interfaces:**
- Consumes: `LinkTarget` from Task 1.
- Produces: `Index::resolve` signature unchanged; behaviour widened to on-disk files inside the workspace.

Issue #21. `resolve` gates on `self.notes.contains_key`, and `Index::scan` only inserts `.md`, so `[config](./config.toml)` computes the right path and throws it away.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/knowledge.rs`:

```rust
    #[test]
    fn relative_links_resolve_to_any_file_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("note.md"), "see [c](./config.toml)").unwrap();
        std::fs::write(root.join("config.toml"), "x = 1").unwrap();
        let index = Index::scan(root);

        let link = RawLink {
            target: "./config.toml".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        assert_eq!(
            index.resolve(&root.join("note.md"), &link),
            Some(normalize(&root.join("config.toml"))),
            "a non-Markdown file that exists should resolve"
        );
    }

    #[test]
    fn relative_links_to_missing_files_do_not_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("note.md"), "x").unwrap();
        let index = Index::scan(root);
        let link = RawLink {
            target: "./nope.png".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        // Unlike a wiki link, a relative link to a missing file is not
        // created — inventing `nope.png` would be nonsense.
        assert_eq!(index.resolve(&root.join("note.md"), &link), None);
    }

    #[test]
    fn relative_links_cannot_escape_the_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("secret.txt"), "s").unwrap();
        std::fs::write(root.join("note.md"), "x").unwrap();
        let index = Index::scan(&root);
        let link = RawLink {
            target: "../secret.txt".into(), wiki: false, range: 0..1,
            context: String::new(),
        };
        assert_eq!(index.resolve(&root.join("note.md"), &link), None,
                   "a link must not reach outside the opened folder");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test relative_links_resolve_to_any_file`
Expected: FAIL — `resolve` returns `None` for `config.toml`.

- [ ] **Step 3: Implement**

Replace the `else` branch of `Index::resolve` in `src/knowledge.rs`:

```rust
        } else {
            let base = from.parent()?;
            let resolved = normalize(&base.join(&link.target));
            // The index answers "what links to what" and holds only .md.
            // Opening is a different question: any file inside the
            // workspace that exists on disk is a valid target, which is
            // what makes `[config](./config.toml)` work.
            if !resolved.starts_with(&self.root) {
                return None; // must not escape the opened folder
            }
            (self.notes.contains_key(&resolved) || resolved.is_file()).then_some(resolved)
        }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test knowledge::`
Expected: PASS, including the pre-existing wiki and `.md` relative cases.

**If `relative_links_cannot_escape_the_workspace_root` behaves oddly on
macOS:** tempdirs sit behind the `/private` symlink, so `self.root` and the
joined path can disagree on prefix even when they name the same place.
`Index::head_text` in `src/git.rs` already hit this and canonicalises both
sides before comparing. If the check misfires, do the same here — compare
canonicalised paths, and fall back to allowing the link when either side
fails to canonicalise, so an unreadable path does not silently break
navigation.

- [ ] **Step 5: Commit**

```bash
git add src/knowledge.rs
git commit -m "fix: resolve links to any workspace file, not only notes"
```

---

### Task 3: Open external links

**Files:**
- Modify: `src/editor/mod.rs:660` (`follow_link_at`)

**Interfaces:**
- Consumes: `classify`, `LinkTarget`.
- Produces: `follow_link_at` returns `true` for a handled external link.

Issue #20.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/editor/mod.rs`:

```rust
    #[gpui::test]
    fn external_links_are_handled_rather_than_falling_through(cx: &mut TestAppContext) {
        let (_fx, editor, cx) = open_editor(cx, "n.md", "see [apple](https://apple.com)\n");
        // Offset 12 sits inside the link text.
        let handled = editor.update(cx, |ed, cx| ed.follow_link_at(12, cx));
        assert!(handled, "an https link must be handled, not passed to the index");
    }

    #[gpui::test]
    fn non_http_schemes_are_not_opened(cx: &mut TestAppContext) {
        let (_fx, editor, cx) =
            open_editor(cx, "n.md", "see [x](supermd://install-plugin?name=evil)\n");
        let handled = editor.update(cx, |ed, cx| ed.follow_link_at(9, cx));
        assert!(!handled, "only http(s) is opened from a document");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test external_links_are_handled`
Expected: FAIL — returns `false`.

- [ ] **Step 3: Implement**

In `follow_link_at`, immediately after the `link` binding and before the `KnowledgeState` lookup:

```rust
        // An external link never touches the index — classify first.
        if let crate::knowledge::LinkTarget::External(url) =
            crate::knowledge::classify(&link)
        {
            cx.open_url(&url);
            return true;
        }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --bins editor::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/editor/mod.rs
git commit -m "fix: open external hyperlinks"
```

---

### Task 4: A plain click follows a link

**Files:**
- Modify: `src/editor/mod.rs` (`on_line_mouse_down`)

**Interfaces:**
- Consumes: `follow_link_at`.
- Produces: `pub fn click_follows_link(has_modifier: bool, on_link: bool, revealed: bool) -> bool`

Today following requires ⌘-click. Every note-focused editor follows on a plain click, and SuperMD renders links rather than showing their syntax — so a plain click on rendered link text should navigate. The rule is pure so the interaction is testable without a window.

**The reveal caveat:** when the cursor is already inside the link, its syntax is showing and the user is editing it. A plain click there must place the caret, not navigate — otherwise the link becomes uneditable.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/editor/mod.rs`:

```rust
    #[test]
    fn plain_click_follows_a_rendered_link_but_not_a_revealed_one() {
        // (modifier, on a link, link syntax revealed) -> follows?
        assert!(click_follows_link(false, true, false), "plain click on rendered link");
        assert!(click_follows_link(true, true, false), "cmd-click still follows");
        // Revealed means the cursor is inside it and the user is editing.
        assert!(!click_follows_link(false, true, true), "plain click edits a revealed link");
        assert!(click_follows_link(true, true, true), "cmd-click follows even when revealed");
        assert!(!click_follows_link(false, false, false), "not on a link");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test plain_click_follows_a_rendered_link`
Expected: FAIL — `cannot find function 'click_follows_link'`.

- [ ] **Step 3: Implement**

Add near `follow_link_at` in `src/editor/mod.rs`:

```rust
/// Whether a click should navigate rather than place the caret.
///
/// A rendered link follows on a plain click, matching every note-focused
/// editor. A link whose syntax is revealed is one the cursor is already
/// inside, so a plain click there edits it — otherwise the link could
/// never be corrected. ⌘-click always follows, as before.
pub fn click_follows_link(has_modifier: bool, on_link: bool, revealed: bool) -> bool {
    on_link && (has_modifier || !revealed)
}
```

Then in `on_line_mouse_down`, replace the ⌘-click block:

```rust
        if !event.modifiers.shift {
            if let Some(offset) = self.offset_at_point(event.position) {
                let on_link = crate::knowledge::Index::link_at(
                    &self.core.buffer.text(), offset,
                ).is_some();
                let revealed = self
                    .core
                    .buffer
                    .line_of_byte(offset)
                    == self.core.buffer.line_of_byte(self.core.selection.head);
                if click_follows_link(event.modifiers.platform, on_link, revealed)
                    && self.follow_link_at(offset, cx)
                {
                    return;
                }
            }
        }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --bins editor::`
Expected: PASS.

- [ ] **Step 5: Verify by hand**

Open a note with a link. Click it once — it navigates. Put the caret inside the link so the syntax reveals, then click within it — the caret moves and nothing navigates.

- [ ] **Step 6: Commit**

```bash
git add src/editor/mod.rs
git commit -m "feat: a plain click follows a link"
```

---

### Task 5: Broken links look broken

**Files:**
- Modify: `src/editor/spans.rs` (add `StyleKind::BrokenLink`)
- Modify: `src/editor/mod.rs:1928` (style mapping)

**Interfaces:**
- Consumes: `Index::resolve`, `classify`.
- Produces: `StyleKind::BrokenLink` variant.

A `[[Ghost]]` that resolves to nothing renders identically to one that works. Zed validates links; Obsidian styles unresolved ones. Without this, the two link fixes above are invisible when they fail.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/editor/spans.rs`:

```rust
    #[test]
    fn broken_link_is_a_distinct_style_kind() {
        // The variant must exist and not equal Link, so the renderer can
        // colour it differently.
        assert_ne!(StyleKind::BrokenLink, StyleKind::Link);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test broken_link_is_a_distinct_style_kind`
Expected: FAIL — no variant `BrokenLink`.

- [ ] **Step 3: Implement**

Add to `StyleKind` in `src/editor/spans.rs`, after `Link`:

```rust
    /// A link whose target does not resolve — an unresolved `[[wiki]]`
    /// or a relative path with no file behind it. Rendered muted and
    /// dotted so a typo is visible before the click.
    BrokenLink,
```

Add to the match in `src/editor/mod.rs` beside `StyleKind::Link`:

```rust
                    StyleKind::BrokenLink => {
                        a.color = t.fg_muted;
                        a.underline = true;
                    }
```

- [ ] **Step 4: Run both configurations**

```bash
cargo test
cargo test --no-default-features --features mas
```
Expected: PASS. A non-exhaustive match elsewhere will fail to compile — fix every site the compiler names.

- [ ] **Step 5: Commit**

```bash
git add src/editor/spans.rs src/editor/mod.rs
git commit -m "feat: style unresolved links so a typo is visible"
```

---

### Task 6: Back and forward navigation

**Files:**
- Create: `src/nav.rs`
- Modify: `src/main.rs` (add `mod nav;`), `src/workspace.rs`, `src/commands.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub struct History`; `History::visit(&mut self, PathBuf)`; `History::back(&mut self) -> Option<PathBuf>`; `History::forward(&mut self) -> Option<PathBuf>`; `History::can_back(&self) -> bool`; `History::can_forward(&self) -> bool`

Once links work you can go somewhere and not return. Every editor studied has this. The stack is pure and lives in its own file.

- [ ] **Step 1: Write the failing tests**

Create `src/nav.rs` containing only the doc comment and tests:

```rust
//! Back/forward history for followed links. Pure: the workspace drives
//! it and owns the tabs, this owns only the order things were visited.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf { PathBuf::from(s) }

    #[test]
    fn back_and_forward_walk_the_visit_order() {
        let mut h = History::default();
        h.visit(p("/a"));
        h.visit(p("/b"));
        h.visit(p("/c"));
        assert_eq!(h.back(), Some(p("/b")));
        assert_eq!(h.back(), Some(p("/a")));
        assert_eq!(h.back(), None, "cannot go before the first visit");
        assert_eq!(h.forward(), Some(p("/b")));
        assert_eq!(h.forward(), Some(p("/c")));
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn visiting_after_going_back_truncates_the_forward_branch() {
        let mut h = History::default();
        h.visit(p("/a"));
        h.visit(p("/b"));
        h.back();
        h.visit(p("/z"));
        assert_eq!(h.forward(), None, "the old forward entry is gone");
        assert_eq!(h.back(), Some(p("/a")));
    }

    #[test]
    fn revisiting_the_current_file_does_not_stack_duplicates() {
        let mut h = History::default();
        h.visit(p("/a"));
        h.visit(p("/a"));
        assert_eq!(h.back(), None, "re-opening the same file is not a move");
    }

    #[test]
    fn can_flags_track_the_ends() {
        let mut h = History::default();
        assert!(!h.can_back() && !h.can_forward());
        h.visit(p("/a"));
        h.visit(p("/b"));
        assert!(h.can_back() && !h.can_forward());
        h.back();
        assert!(h.can_forward());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test nav::`
Expected: FAIL — `cannot find type 'History'`.

- [ ] **Step 3: Implement**

Prepend to `src/nav.rs`:

```rust
use std::path::PathBuf;

/// A browser-style visit stack: entries plus a cursor into them.
#[derive(Default)]
pub struct History {
    entries: Vec<PathBuf>,
    /// Index of the current entry. Meaningless when `entries` is empty.
    at: usize,
}

impl History {
    /// Record a newly opened file. Re-opening the current file is not a
    /// move, so it does not stack. Visiting after going back discards
    /// the forward branch, which is what every browser does.
    pub fn visit(&mut self, path: PathBuf) {
        if self.entries.get(self.at) == Some(&path) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.at + 1);
            self.at += 1;
        }
        self.entries.push(path);
        self.at = self.entries.len() - 1;
    }

    pub fn can_back(&self) -> bool { self.at > 0 }

    pub fn can_forward(&self) -> bool {
        !self.entries.is_empty() && self.at + 1 < self.entries.len()
    }

    pub fn back(&mut self) -> Option<PathBuf> {
        if !self.can_back() {
            return None;
        }
        self.at -= 1;
        self.entries.get(self.at).cloned()
    }

    pub fn forward(&mut self) -> Option<PathBuf> {
        if !self.can_forward() {
            return None;
        }
        self.at += 1;
        self.entries.get(self.at).cloned()
    }
}
```

Add `mod nav;` to `src/main.rs` beside the other module declarations.

- [ ] **Step 4: Run the tests**

Run: `cargo test nav::`
Expected: PASS — 4 tests.

- [ ] **Step 5: Wire it into the workspace**

Add to the `Workspace` struct in `src/workspace.rs`:

```rust
    /// Back/forward across followed links.
    history: crate::nav::History,
```

Initialise it with `history: crate::nav::History::default(),` in the constructor beside the other fields.

Record every open by adding this as the first line of the body of `pub fn open_path` (`src/workspace.rs:770`), before the `is_dir` branch:

```rust
        if path.is_file() {
            self.history.visit(path.to_path_buf());
        }
```

Add the actions to the `actions!` block:

```rust
        NavigateBack,
        NavigateForward,
```

Add the handlers beside the other navigation methods:

```rust
    fn navigate_back(&mut self, _: &NavigateBack, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.history.back() {
            // open_path records a visit; going back must not.
            self.open_path_without_history(&path, window, cx);
        }
    }

    fn navigate_forward(
        &mut self,
        _: &NavigateForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.history.forward() {
            self.open_path_without_history(&path, window, cx);
        }
    }

    /// Open a file without recording it — used by back and forward, which
    /// are moves through history rather than new visits.
    fn open_path_without_history(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_path_preview(path, true, window, cx);
    }
```

Register both beside the other `.on_action(...)` calls in `render`:

```rust
            .on_action(cx.listener(Self::navigate_back))
            .on_action(cx.listener(Self::navigate_forward))
```

- [ ] **Step 6: Add the command table rows**

In `src/commands.rs`, in the Go section beside `FollowLink`:

```rust
    ws::NavigateBack => { id: "nav_back", label: "Back", keys: ["cmd-["],
        ctx: None, menu: Some((Go, 2)), help: Some(General) },
    ws::NavigateForward => { id: "nav_forward", label: "Forward", keys: ["cmd-]"],
        ctx: None, menu: Some((Go, 2)), help: Some(General) },
```

`cmd-[` and `cmd-]` are unbound; tab switching uses `cmd-shift-[` and `cmd-shift-]`, so the pairing reads consistently.

- [ ] **Step 7: Regenerate the shortcut docs**

```bash
UPDATE_DOCS=1 cargo test shortcut_docs_match_the_table
cargo run --example build_docs
```

- [ ] **Step 8: Run both configurations**

```bash
cargo test
cargo test --no-default-features --features mas
```
Expected: PASS, including `shortcut_docs_match_the_table` and the command-collision test.

- [ ] **Step 9: Commit**

```bash
git add src/nav.rs src/main.rs src/workspace.rs src/commands.rs docs/site/shortcuts.md site/docs
git commit -m "feat: back and forward through followed links"
```

---

### Task 7: Clicking a widget keeps your place

**Files:**
- Modify: `src/editor/mod.rs:457` (`reproject`)

**Interfaces:**
- Consumes: `ListState::logical_scroll_top`, `ListState::scroll_to`.
- Produces: `reproject` preserves the scroll anchor.

Issue #17. `ListState::reset` sets `logical_scroll_top = None` **and splices every item, discarding measured heights**. `scroll_to_reveal_item` then seeks a tree whose heights are all zero, computes `goal_top = 0`, and pins to item 0 — the top of the file.

`scroll_to` sets the anchor directly and does not depend on measured heights, so capturing the anchor before the reset and restoring it after is the smaller, safer fix.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/editor/mod.rs`:

```rust
    /// Clicking a table far down a document must not scroll to the top.
    #[gpui::test]
    fn revealing_a_widget_keeps_the_scroll_position(cx: &mut TestAppContext) {
        // A long document with a table near the end.
        let mut text = String::new();
        for i in 0..300 {
            text.push_str(&format!("line {i}\n\n"));
        }
        text.push_str("| a | b |\n| - | - |\n| 1 | 2 |\n");
        let (_fx, editor, cx) = open_editor(cx, "long.md", &text);
        cx.run_until_parked();

        // Scroll to the table and let the projection settle.
        editor.update(cx, |ed, _| {
            let last = ed.projection.len().saturating_sub(1);
            ed.list_state.scroll_to_reveal_item(last);
        });
        cx.run_until_parked();
        let before = editor.read_with(cx, |ed, _| ed.list_state.logical_scroll_top().item_ix);
        assert!(before > 0, "precondition: we are not at the top");

        // Put the cursor in the table, which reveals it and changes the
        // projection — the path that used to reset the scroll.
        editor.update(cx, |ed, cx| {
            let offset = ed.core.buffer.text().find("| a |").unwrap();
            ed.core.set_cursor(offset);
            cx.notify();
        });
        cx.run_until_parked();

        let after = editor.read_with(cx, |ed, _| ed.list_state.logical_scroll_top().item_ix);
        assert!(after > 0, "revealing a widget must not jump to the top (was {after})");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test revealing_a_widget_keeps_the_scroll_position`
Expected: FAIL — `after` is 0.

- [ ] **Step 3: Implement**

Replace the body of `reproject` in `src/editor/mod.rs`:

```rust
    fn reproject(&mut self) {
        if self.diff.is_some() {
            return; // list is showing the diff doc, not the projection
        }
        let items = self.compute_projection();
        if items != self.projection {
            // reset() clears logical_scroll_top AND discards every
            // measured height, so a following scroll_to_reveal_item
            // computes goal_top = 0 and pins to item 0. Capture the
            // anchor first and restore it with scroll_to, which sets the
            // anchor directly and needs no measurements.
            let anchor = self.list_state.logical_scroll_top();
            self.projection = items;
            self.list_state.reset(self.projection.len());
            let clamped = ListOffset {
                item_ix: anchor.item_ix.min(self.projection.len().saturating_sub(1)),
                offset_in_item: anchor.offset_in_item,
            };
            self.list_state.scroll_to(clamped);
            self.reveal_cursor();
        }
    }
```

`ListOffset` is already imported in this file.

- [ ] **Step 4: Run the tests**

Run: `cargo test --bins editor::`
Expected: PASS, including the pre-existing projection tests.

- [ ] **Step 5: Verify by hand**

Open a long document with a table near the end, scroll to it, click it. The table reveals its source and the view stays put.

- [ ] **Step 6: Commit**

```bash
git add src/editor/mod.rs
git commit -m "fix: revealing a widget no longer scrolls to the top"
```

---

### Task 8: ⌘⇧G opens the graph

**Files:**
- Modify: `src/editor/mod.rs:1551` (`find_prev`)

**Interfaces:**
- Consumes: nothing.
- Produces: `find_prev` propagates when the find bar is closed.

Issue #14. `cmd-shift-g` is bound twice: `ws::ToggleGraph` (global) and `ed::FindPrev` (Editor context). With a document open the editor binding wins, `cycle_find` returns immediately because `self.find` is `None`, and the key is swallowed. `toggle_bold` already solves the same collision for ⌘B by propagating.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/editor/mod.rs`:

```rust
    #[test]
    fn find_prev_only_consumes_the_key_when_the_find_bar_is_open() {
        // Mirrors cmd_b_is_shared_across_contexts_on_purpose: a shared
        // chord must fall through when this handler has nothing to do,
        // or the global binding is unreachable.
        assert!(!find_prev_should_consume(false), "closed find bar must propagate");
        assert!(find_prev_should_consume(true), "open find bar consumes the key");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test find_prev_only_consumes_the_key`
Expected: FAIL — `cannot find function 'find_prev_should_consume'`.

- [ ] **Step 3: Implement**

Add beside `find_prev` in `src/editor/mod.rs`:

```rust
/// Whether ⌘⇧G belongs to the editor's find bar or should fall through
/// to the global Graph View binding. Pure so the collision rule is
/// recorded in a test rather than in a comment.
pub fn find_prev_should_consume(find_open: bool) -> bool {
    find_open
}
```

Replace `find_prev`:

```rust
    fn find_prev(&mut self, _: &FindPrev, _: &mut Window, cx: &mut Context<Self>) {
        if !find_prev_should_consume(self.find.is_some()) {
            cx.propagate(); // ⌘⇧G belongs to Graph View when find is closed
            return;
        }
        self.cycle_find(false, cx);
    }
```

- [ ] **Step 4: Add the collision exception test**

Add to `mod tests` in `src/commands.rs`, mirroring `cmd_b_is_shared_across_contexts_on_purpose`:

```rust
    #[test]
    fn cmd_shift_g_is_shared_across_contexts_on_purpose() {
        // ToggleGraph (global) and FindPrev (Editor) both bind ⌘⇧G; the
        // editor handler propagates when the find bar is closed so the
        // graph still opens. This records the exception.
        let holders: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.keys.contains(&"cmd-shift-g"))
            .map(|c| c.id)
            .collect();
        assert_eq!(holders, vec!["graph", "find_prev"]);
    }
```

If the ids differ, run `cargo test cmd_shift_g_is_shared` and use the ids the failure prints.

- [ ] **Step 5: Run both configurations**

```bash
cargo test
cargo test --no-default-features --features mas
```
Expected: PASS.

- [ ] **Step 6: Verify by hand**

Open a document and press ⌘⇧G — the graph opens. Press ⌘F, then ⌘⇧G — it steps to the previous match instead.

- [ ] **Step 7: Commit**

```bash
git add src/editor/mod.rs src/commands.rs
git commit -m "fix: cmd-shift-g reaches Graph View when the find bar is closed"
```

---

### Task 9: CI refuses a binary containing private Apple APIs

**Files:**
- Create: `scripts/check_private_apis.sh`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: a built binary path.
- Produces: exit 0 when clean, 1 with the offending symbols listed.

App Review rejected 0.0.14 twice; the private-API rejection was found by a scan we could have run ourselves. `altool --validate-app` does **not** check this, so it surfaces only after upload. Encoding it as a script means the vendored gpui patch cannot silently regress.

- [ ] **Step 1: Write the script**

```bash
cat > scripts/check_private_apis.sh <<'SH'
#!/bin/bash
# Fail if a binary references private Apple APIs.
#
# App Review rejects these on sight, and its scan is static — a linked
# symbol is enough, whether or not the code can run. altool
# --validate-app does NOT check, so without this the failure only appears
# after upload. See vendor/gpui/PATCH.md.
#
# Usage: scripts/check_private_apis.sh <binary>
set -uo pipefail
BIN="${1:?usage: check_private_apis.sh <binary>}"
fail=0

# Private CoreGraphics / SkyLight, as undefined symbols.
syms=$(nm -u "$BIN" 2>/dev/null | sed 's/^ *//' | grep -E '^_(CGS|SLS)' | sort -u)
if [ -n "$syms" ]; then
    echo "private symbols:"; echo "$syms" | sed 's/^/  /'; fail=1
fi

# Private selectors are dispatched at runtime and never appear as
# undefined symbols — only a string scan finds them.
sels=$(strings -a "$BIN" 2>/dev/null | grep -E '^_(windowResize[A-Za-z]*Cursor|updateProxyLayer|setCornerMask)$' | sort -u)
if [ -n "$sels" ]; then
    echo "private selectors:"; echo "$sels" | sed 's/^/  /'; fail=1
fi

if [ "$fail" = 0 ]; then
    echo "clean: no private Apple APIs in $BIN"
fi
exit $fail
SH
chmod +x scripts/check_private_apis.sh
```

- [ ] **Step 2: Prove it passes on the current binary**

```bash
cargo build --release
bash scripts/check_private_apis.sh target/release/supermd
```
Expected: `clean: no private Apple APIs in target/release/supermd`, exit 0.

- [ ] **Step 3: Prove it actually detects something**

A check that cannot fail is worthless. Confirm the detector fires:

```bash
printf '_CGSSetWindowBackgroundBlurRadius\n' > /tmp/fake_bin
bash scripts/check_private_apis.sh /tmp/fake_bin; echo "exit=$?"
rm -f /tmp/fake_bin
```
Expected: reports the selector or symbol and `exit=1`.

- [ ] **Step 4: Wire it into CI**

In `.github/workflows/ci.yml`, in the `test` job after `Build the App Store configuration`:

```yaml
      - name: Refuse private Apple APIs
        if: runner.os == 'macOS'
        run: |
          cargo build --release --no-default-features --features mas
          bash scripts/check_private_apis.sh target/release/supermd
```

- [ ] **Step 5: Commit**

```bash
git add scripts/check_private_apis.sh .github/workflows/ci.yml
git commit -m "ci: refuse a binary that references private Apple APIs"
```

---

### Task 10: The plugin catalogue refreshes itself on release

**Files:**
- Modify: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: `scripts/update_catalog_hashes.sh`.
- Produces: a committed catalogue pointing at the release just published.

`update_catalog_hashes.sh` is a manual post-release step. It is currently stale — `plugins/catalog.json` points at v0.0.13 assets while v0.0.14 is out — so the website's Download buttons serve last release's plugins.

- [ ] **Step 1: Add the job**

In `.github/workflows/release.yml`, after `publish`:

```yaml
  catalog:
    name: Refresh the plugin catalogue
    needs: [publish]
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v5
        with:
          ref: master
      - name: Point the catalogue at this release
        run: bash scripts/update_catalog_hashes.sh "$GITHUB_REF_NAME"
      - name: Commit if anything changed
        run: |
          if git diff --quiet plugins/catalog.json; then
            echo "catalogue already current"; exit 0
          fi
          git config user.name "github-actions[bot]"
          git config user.email "github-actions[bot]@users.noreply.github.com"
          git add plugins/catalog.json
          git commit -m "chore: point the plugin catalogue at $GITHUB_REF_NAME"
          git push origin master
```

It runs after `publish` because the script downloads the release assets to hash them — they must exist first.

- [ ] **Step 2: Verify the workflow parses**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('release.yml parses')"
```

- [ ] **Step 3: Refresh the catalogue by hand for the current release**

The automation only helps from the next tag onward; the stale entries are live now.

```bash
bash scripts/update_catalog_hashes.sh v0.0.14
python3 -c "
import json; d=json.load(open('plugins/catalog.json'))
assert all('v0.0.14' in p['download'] for p in d['plugins'])
assert all(len(p['sha256'])==64 for p in d['plugins'])
print('catalogue points at v0.0.14')"
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml plugins/catalog.json
git commit -m "ci: refresh the plugin catalogue on release"
```

---

### Task 11: Measure what "lightweight" means

**Files:**
- Create: `docs/internal/performance.md`

**Interfaces:**
- Consumes: nothing.
- Produces: a recorded baseline.

We claim lightweight against competitors with published figures — Obsidian 180–250 MB resident, 213 MB installed — and have measured nothing. This is the guard rail for the agent work in 0.1.0, which is exactly the kind of feature that could quietly make the claim untrue.

- [ ] **Step 1: Build a large workspace**

```bash
python3 - <<'PY'
import os, random
root = os.path.expanduser("~/supermd-perf-vault")
os.makedirs(root, exist_ok=True)
words = "note link graph markdown editor plugin theme render index".split()
for i in range(2000):
    d = os.path.join(root, f"folder{i//100}")
    os.makedirs(d, exist_ok=True)
    body = [f"# Note {i}", ""]
    for _ in range(40):
        body.append(" ".join(random.choice(words) for _ in range(12)))
    body.append(f"See [[Note {random.randint(0,1999)}]] and [[Note {random.randint(0,1999)}]].")
    open(os.path.join(d, f"note{i}.md"), "w").write("\n".join(body))
print("wrote 2000 notes to", root)
PY
```

- [ ] **Step 2: Measure cold start and resident memory**

```bash
cargo build --release
/usr/bin/time -l ./target/release/supermd ~/supermd-perf-vault 2>&1 | tail -20 &
sleep 15
ps -o rss=,command= -p "$(pgrep -n supermd)" | awk '{printf "resident: %.1f MB\n", $1/1024}'
pkill -f "target/release/supermd"
```

Record the peak resident size from `/usr/bin/time -l` and the `ps` figure. Repeat three times and take the median — first runs pay for page cache.

- [ ] **Step 3: Write the results down**

Create `docs/internal/performance.md` with the measured numbers, the machine, the OS version, the vault size, and the date. Include the comparison figures (Obsidian 180–250 MB resident, 213 MB installed; SuperMD 18.3 MB installed per the App Store listing) and state plainly which of ours are measured versus quoted.

- [ ] **Step 4: Clean up**

```bash
rm -rf ~/supermd-perf-vault
```

- [ ] **Step 5: Commit**

`docs/internal/` is gitignored, so there is nothing to commit. Instead, if the resident figure is competitive, add one sentence to `site/index.html`'s note line quoting it, and commit that:

```bash
git add site/index.html
git commit -m "docs: quote the measured memory footprint"
```

If the figure is *not* competitive, do not publish it — record it in `docs/internal/performance.md` and raise it as a finding. An unflattering measurement is still worth having.

---

### Task 12: Ship 0.0.15

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`

- [ ] **Step 1: Full verification**

```bash
cargo test
cargo test --no-default-features --features mas
cargo llvm-cov --summary-only
```
Expected: both suites green; coverage at or above the 90% floor.

- [ ] **Step 2: Bump the version**

`Cargo.toml` to `0.0.15`, then `cargo update -p supermd --precise 0.0.15` so the lockfile follows. **Commit before tagging** — cargo-deb reads the manifest and the DMG takes the tag, which would mask a mismatch.

```bash
git add Cargo.toml Cargo.lock
git commit -m "release: v0.0.15"
```

- [ ] **Step 3: Tag and push**

```bash
git tag -a v0.0.15 -m "v0.0.15 — links that work"
git push origin master
git push origin v0.0.15
```

- [ ] **Step 4: Watch the release workflow**

Expect DMG, Linux, Windows and the new catalogue job to succeed, and the `mas` job to skip for want of signing secrets.

- [ ] **Step 5: Build and submit the App Store update**

Follow `docs/mac-app-store.md`. This release also realigns the versions: GitHub v0.0.14 and App Store 0.0.14 are currently different binaries, and 0.0.15 is built from one commit for both.

---

## Deferred from this release

Named so nobody adds them mid-flight:

- **Table row/column commands** and **ordered-list renumbering** — real gaps, but a different theme (writing ergonomics) that would blur this release.
- **Find and replace** — the same class of absence as the link bugs, and a larger build than everything above combined. First candidate for 0.0.16.
- **Dark-mode App Store screenshots** — deferred by decision; tooling and pitfalls are in `docs/screenshots.md`.
- **Universal binary for Intel** — needs its own decision; it changes the store listing's stated compatibility.

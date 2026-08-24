# Git Diff Viewer ("Show Changes") Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A read-only, in-place diff view of the current file against git HEAD — styled word-level prose diffs for Markdown, line diffs in code-mode — plus sidebar modified-dots.

**Architecture:** A pure engine (`src/diff.rs`) merges HEAD text and buffer text into one synthetic document plus a change map; a thin read-only git wrapper (`src/git.rs`) supplies the baseline and status set; the view layer adds an `EditorView::Diff` mode that pushes the merged text through the existing styling/layout pipeline with one extra overlay layer for change washes.

**Tech Stack:** Rust, GPUI 0.2.2, `similar` (diffing), `git2` + vendored libgit2 (baseline/status).

**Spec:** `docs/superpowers/specs/2026-08-24-git-diff-viewer-design.md`

## Global Constraints

- Read-only feature: never write to the buffer, disk, or repository from diff code paths.
- All `Change` ranges: byte ranges into merged text, on `char` boundaries, non-overlapping, sorted by start (same discipline as `spans.rs`).
- Size cap: reuse `MAX_STYLED_BYTES` (1 MB) from `src/editor/spans.rs`; over cap → line-level marks only, never nothing.
- All git errors degrade to "no baseline" / empty status set — no crashes, no dialogs.
- TDD: every task writes the failing test first and watches it fail.
- Commit after every green task.

---

### Task 1: Diff engine — line-level merge

**Files:**
- Create: `src/diff.rs`
- Modify: `src/main.rs` (add `mod diff;`), `Cargo.toml` (add `similar = "2"`)

**Interfaces:**
- Produces: `diff::{ChangeKind, Change, DiffDoc, diff_doc(old: &str, new: &str) -> DiffDoc}` exactly as in the spec's Component 1. Tasks 2, 6, 7 consume these.

- [ ] **Step 1: Add dependency**

Run: `cargo add similar@2`

- [ ] **Step 2: Write failing tests** (in `src/diff.rs` under `#[cfg(test)]`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_docs_have_no_changes() {
        let d = diff_doc("a\nb\n", "a\nb\n");
        assert_eq!(d.text, "a\nb\n");
        assert!(d.changes.is_empty());
    }

    #[test]
    fn pure_insertion_marks_added() {
        let d = diff_doc("a\nc\n", "a\nb\nc\n");
        assert_eq!(d.text, "a\nb\nc\n");
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Added);
        assert_eq!(&d.text[d.changes[0].range.clone()], "b\n");
    }

    #[test]
    fn pure_deletion_splices_deleted_run() {
        let d = diff_doc("a\nb\nc\n", "a\nc\n");
        assert_eq!(d.text, "a\nb\nc\n"); // deleted line spliced back
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Deleted);
        assert_eq!(&d.text[d.changes[0].range.clone()], "b\n");
    }

    #[test]
    fn changes_sorted_and_non_overlapping() {
        let d = diff_doc("a\nX\nc\nY\ne\n", "a\nb\nc\nd\ne\n");
        let mut last = 0;
        for c in &d.changes {
            assert!(c.range.start >= last, "sorted, non-overlapping");
            assert!(d.text.is_char_boundary(c.range.start));
            assert!(d.text.is_char_boundary(c.range.end));
            last = c.range.end;
        }
    }

    /// Strip Deleted ranges → new; strip Added ranges → old.
    fn assert_reconstruction(old: &str, new: &str) {
        let d = diff_doc(old, new);
        let strip = |kind: ChangeKind| {
            let mut out = String::new();
            let mut pos = 0;
            for c in &d.changes {
                if c.kind == kind {
                    out.push_str(&d.text[pos..c.range.start]);
                    pos = c.range.end;
                }
            }
            out.push_str(&d.text[pos..]);
            out
        };
        assert_eq!(strip(ChangeKind::Deleted), new, "strip deleted == new");
        assert_eq!(strip(ChangeKind::Added), old, "strip added == old");
    }

    #[test]
    fn reconstruction_invariants_hold() {
        for (old, new) in [
            ("", ""),
            ("a\n", ""),
            ("", "a\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("a\nc\n", "a\nb\nc\n"),
            ("x\ny\n", "p\nq\n"),
            ("one two three\n", "one 2 three\n"),
            ("no trailing newline", "still no newline"),
        ] {
            assert_reconstruction(old, new);
        }
    }
}
```

- [ ] **Step 3: Run tests, watch them fail to compile** (`cargo test diff::` — types don't exist)

- [ ] **Step 4: Implement line-level engine**

```rust
//! Pure diff engine: merges old and new text into one document with a
//! change map. No git, no GPUI. See the design spec for invariants.

use std::ops::Range;

pub const MAX_DIFF_BYTES: usize = crate::editor::spans::MAX_STYLED_BYTES;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind { Added, Deleted }

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    pub range: Range<usize>,
    pub kind: ChangeKind,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DiffDoc {
    pub text: String,
    pub changes: Vec<Change>,
}

pub fn diff_doc(old: &str, new: &str) -> DiffDoc {
    let mut doc = DiffDoc::default();
    let diff = similar::TextDiff::from_lines(old, new);
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let s = change.value();
            let start = doc.text.len();
            doc.text.push_str(s);
            match change.tag() {
                similar::ChangeTag::Equal => {}
                similar::ChangeTag::Insert => doc.changes.push(Change {
                    range: start..doc.text.len(),
                    kind: ChangeKind::Added,
                }),
                similar::ChangeTag::Delete => doc.changes.push(Change {
                    range: start..doc.text.len(),
                    kind: ChangeKind::Deleted,
                }),
            }
        }
    }
    coalesce(&mut doc.changes);
    doc
}

/// Merge adjacent same-kind changes so the map stays minimal.
fn coalesce(changes: &mut Vec<Change>) {
    let mut out: Vec<Change> = Vec::with_capacity(changes.len());
    for c in changes.drain(..) {
        match out.last_mut() {
            Some(last) if last.kind == c.kind && last.range.end == c.range.start => {
                last.range.end = c.range.end;
            }
            _ => out.push(c),
        }
    }
    *changes = out;
}
```

Note: `similar` emits Delete runs before Insert runs within a replace
op, so deleted text lands before added text in the merged doc —
matching "deleted runs spliced at their original positions".
`MAX_STYLED_BYTES` must be `pub` in `spans.rs` (it is `pub(crate)`
today at most — make it `pub(crate)` if private and import via
`crate::editor::spans::MAX_STYLED_BYTES`).

- [ ] **Step 5: Run tests to green** (`cargo test diff::`)

- [ ] **Step 6: Full suite** (`cargo test`) — no regressions.

- [ ] **Step 7: Commit** — `feat: diff engine with line-level merge and change map`

---

### Task 2: Diff engine — word-level refinement + size cap

**Files:**
- Modify: `src/diff.rs`

**Interfaces:**
- Produces: same public API; replace runs are now refined to word granularity when total input ≤ `MAX_DIFF_BYTES`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn one_word_edit_marks_one_word() {
    let d = diff_doc("the quick brown fox\n", "the swift brown fox\n");
    // word refinement: only "quick"/"swift" marked, not the whole line
    let deleted: Vec<&str> = d.changes.iter()
        .filter(|c| c.kind == ChangeKind::Deleted)
        .map(|c| &d.text[c.range.clone()]).collect();
    let added: Vec<&str> = d.changes.iter()
        .filter(|c| c.kind == ChangeKind::Added)
        .map(|c| &d.text[c.range.clone()]).collect();
    assert_eq!(deleted, vec!["quick"]);
    assert_eq!(added, vec!["swift"]);
}

#[test]
fn word_refined_doc_keeps_invariants() {
    assert_reconstruction("the quick brown fox\n", "the swift brown fox\n");
    assert_reconstruction("alpha beta\ngamma delta\n", "alpha b\ngamma delta epsilon\n");
    assert_reconstruction("héllo wörld\n", "héllo mönde\n"); // multibyte
}

#[test]
fn oversized_input_skips_word_refinement() {
    let old = "a ".repeat(600_000); // >1MB combined
    let new = old.replacen("a ", "b ", 1);
    let d = diff_doc(&old, &new);
    assert!(!d.changes.is_empty()); // line-level marks still present
}
```

(`assert_reconstruction` from Task 1 is reused — move it above the test fns if needed.)

- [ ] **Step 2: Run, watch `one_word_edit_marks_one_word` fail** (whole line marked)

- [ ] **Step 3: Implement refinement**

In `diff_doc`, handle ops with tag `Replace` specially when
`old.len() + new.len() <= MAX_DIFF_BYTES`: collect the deleted run and
inserted run as strings, word-diff them, and emit interleaved:

```rust
pub fn diff_doc(old: &str, new: &str) -> DiffDoc {
    let refine = old.len() + new.len() <= MAX_DIFF_BYTES;
    let mut doc = DiffDoc::default();
    let diff = similar::TextDiff::from_lines(old, new);
    for op in diff.ops() {
        if refine && op.tag() == similar::DiffTag::Replace {
            let del: String = diff.iter_changes(op)
                .filter(|c| c.tag() == similar::ChangeTag::Delete)
                .map(|c| c.value()).collect();
            let ins: String = diff.iter_changes(op)
                .filter(|c| c.tag() == similar::ChangeTag::Insert)
                .map(|c| c.value()).collect();
            emit_word_diff(&mut doc, &del, &ins);
            continue;
        }
        for change in diff.iter_changes(op) {
            // ... unchanged Task 1 body ...
        }
    }
    coalesce(&mut doc.changes);
    doc
}

fn emit_word_diff(doc: &mut DiffDoc, del: &str, ins: &str) {
    let wd = similar::TextDiff::from_unicode_words(del, ins);
    for op in wd.ops() {
        for change in wd.iter_changes(op) {
            let start = doc.text.len();
            doc.text.push_str(change.value());
            match change.tag() {
                similar::ChangeTag::Equal => {}
                similar::ChangeTag::Insert => doc.changes.push(Change {
                    range: start..doc.text.len(), kind: ChangeKind::Added }),
                similar::ChangeTag::Delete => doc.changes.push(Change {
                    range: start..doc.text.len(), kind: ChangeKind::Deleted }),
            }
        }
    }
}
```

Note: word-interleaving inside a replace run reorders shared words
relative to the line-level splice, but the reconstruction invariants
still hold because Equal words appear exactly once and Delete/Insert
words carry their kind. The invariant tests are the oracle — if
`from_unicode_words` ordering breaks them, switch to diffing the two
runs with `similar::utils::diff_unicode_words` and emitting
delete-then-insert per contiguous changed region.

- [ ] **Step 4: Run tests to green**, then full suite.

- [ ] **Step 5: Commit** — `feat: word-level diff refinement with size cap`

---

### Task 3: Git baseline module

**Files:**
- Create: `src/git.rs`
- Modify: `src/main.rs` (add `mod git;`), `Cargo.toml`

**Interfaces:**
- Produces: `git::{Baseline, head_text(path: &Path) -> Baseline, modified_paths(root: &Path) -> HashSet<PathBuf>}` per spec Component 2. Tasks 6 and 8 consume these. `modified_paths` returns paths **relative to `root`**.

- [ ] **Step 1: Add dependency**

Run: `cargo add git2 --no-default-features` then enable vendoring:
in `Cargo.toml`: `git2 = { version = "0.20", default-features = false, features = ["vendored-libgit2"] }`
(no https/ssh features needed — local repo access only).

- [ ] **Step 2: Write failing tests** (in `src/git.rs`; helper builds a tempdir repo)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn repo_with_commit(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        let mut index = repo.index().unwrap();
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
        dir
    }

    #[test]
    fn head_text_returns_committed_content() {
        let dir = repo_with_commit(&[("notes.md", "hello\n")]);
        std::fs::write(dir.path().join("notes.md"), "hello world\n").unwrap();
        match head_text(&dir.path().join("notes.md")) {
            Baseline::Text(t) => assert_eq!(t, "hello\n"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn untracked_file_reports_untracked() {
        let dir = repo_with_commit(&[("a.md", "x\n")]);
        std::fs::write(dir.path().join("new.md"), "fresh\n").unwrap();
        assert!(matches!(head_text(&dir.path().join("new.md")), Baseline::Untracked));
    }

    #[test]
    fn fresh_repo_without_commits_reports_untracked() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "x\n").unwrap();
        assert!(matches!(head_text(&dir.path().join("a.md")), Baseline::Untracked));
    }

    #[test]
    fn outside_repo_reports_not_in_repo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "x\n").unwrap();
        assert!(matches!(head_text(&dir.path().join("a.md")), Baseline::NotInRepo));
    }

    #[test]
    fn binary_blob_reports_binary() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
        let mut index = repo.index().unwrap();
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();
        assert!(matches!(head_text(&dir.path().join("blob.bin")), Baseline::Binary));
    }

    #[test]
    fn modified_paths_reports_dirty_and_untracked_only() {
        let dir = repo_with_commit(&[("clean.md", "c\n"), ("dirty.md", "d\n")]);
        std::fs::write(dir.path().join("dirty.md"), "changed\n").unwrap();
        std::fs::write(dir.path().join("new.md"), "n\n").unwrap();
        let set = modified_paths(dir.path());
        assert!(set.contains(std::path::Path::new("dirty.md")));
        assert!(set.contains(std::path::Path::new("new.md")));
        assert!(!set.contains(std::path::Path::new("clean.md")));
    }

    #[test]
    fn modified_paths_outside_repo_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(modified_paths(dir.path()).is_empty());
    }
}
```

- [ ] **Step 3: Run, watch fail to compile.**

- [ ] **Step 4: Implement**

```rust
//! Read-only git access: HEAD baselines and workspace status. All
//! errors degrade to "no baseline" / empty set — never a crash.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Baseline {
    Text(String),
    NotInRepo,
    Untracked,
    Binary,
}

pub fn head_text(path: &Path) -> Baseline {
    let Some(parent) = path.parent() else { return Baseline::NotInRepo };
    let Ok(repo) = git2::Repository::discover(parent) else {
        return Baseline::NotInRepo;
    };
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return Baseline::NotInRepo; // bare repo
    };
    let Ok(canon) = path.canonicalize() else { return Baseline::Untracked };
    let Ok(canon_workdir) = workdir.canonicalize() else { return Baseline::NotInRepo };
    let Ok(rel) = canon.strip_prefix(&canon_workdir) else {
        return Baseline::NotInRepo;
    };
    let Ok(head) = repo.head() else { return Baseline::Untracked }; // unborn
    let Ok(tree) = head.peel_to_tree() else { return Baseline::Untracked };
    let Ok(entry) = tree.get_path(rel) else { return Baseline::Untracked };
    let Ok(obj) = entry.to_object(&repo) else { return Baseline::Untracked };
    let Some(blob) = obj.as_blob() else { return Baseline::Untracked };
    match std::str::from_utf8(blob.content()) {
        Ok(s) => Baseline::Text(s.to_string()),
        Err(_) => Baseline::Binary,
    }
}

pub fn modified_paths(root: &Path) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    let Ok(repo) = git2::Repository::discover(root) else { return set };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else { return set };
    let workdir = repo.workdir().map(Path::to_path_buf);
    for entry in statuses.iter() {
        if let Some(p) = entry.path() {
            // statuses are workdir-relative; re-relativize to `root`
            // when root sits below the repo workdir.
            let abs = match &workdir {
                Some(w) => w.join(p),
                None => continue,
            };
            if let Ok(rel) = abs.strip_prefix(root.canonicalize().unwrap_or_else(|_| root.into())) {
                set.insert(rel.to_path_buf());
            } else if let Ok(rel) = abs.strip_prefix(root) {
                set.insert(rel.to_path_buf());
            }
        }
    }
    set
}
```

(macOS tempdirs live under `/private` symlinks — canonicalize both
sides before `strip_prefix`, as above.)

- [ ] **Step 5: Run to green, full suite.**

- [ ] **Step 6: Commit** — `feat: read-only git baseline and status module`

---

### Task 4: Theme diff colors

**Files:**
- Modify: `src/theme.rs`

**Interfaces:**
- Produces: `Theme { diff_added_bg, diff_added_fg, diff_deleted_bg, diff_deleted_fg: Hsla }` (same color type the rest of `Theme` uses) and the four optional `[colors]` keys in `ThemeFile`. Tasks 6–8 consume `theme.diff_*`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn builtin_themes_have_diff_colors() {
    // light and dark defaults exist and differ between appearances
    assert_ne!(Theme::light().diff_added_bg, Theme::dark().diff_added_bg);
    assert_ne!(Theme::light().diff_deleted_bg, Theme::dark().diff_deleted_bg);
}

#[test]
fn theme_file_diff_keys_optional_and_parsed() {
    let toml = r#"
name = "T"
appearance = "light"
[colors]
diff_added_bg = "#112233"
"#;
    let t = LoadedTheme::from_toml(toml).unwrap();
    assert_eq!(t.theme.diff_added_bg, parse_hex("#112233").unwrap());
    // unspecified keys fall back to appearance defaults
    assert_eq!(t.theme.diff_deleted_bg, Theme::light().diff_deleted_bg);
}
```

(Adapt constructor names to the existing `LoadedTheme::from_toml` /
`parse_hex` signatures in `theme.rs`.)

- [ ] **Step 2: Run, watch fail.**

- [ ] **Step 3: Implement** — add the four fields to `Theme`; defaults per spec:
light: added_bg `0xe6f0dc`, added_fg `0x3d6b2f`, deleted_bg `0xf7e3e0`, deleted_fg `0xa04b3d`;
dark: added_bg `0x2c3a26`, added_fg `0xa8c897`, deleted_bg `0x3d2723`, deleted_fg `0xd18b7f`.
Add four `Option<String>` keys to `ThemeFile`'s colors struct, resolved in the same fallback pattern the other optional colors use.

- [ ] **Step 4: Run to green, full suite.**

- [ ] **Step 5: Commit** — `feat: diff wash colors in theme system`

---

### Task 5: EditorView refactor (Edit / Preview / Diff)

**Files:**
- Modify: `src/workspace.rs` (Tab enum, all `preview` match sites, ⌘E toggle, escape handling)

**Interfaces:**
- Produces: `pub enum EditorView { Edit, Preview, Diff }`; `Tab::Editor { editor, view: EditorView }`. Task 6 consumes `EditorView::Diff`.
- Consumes: nothing new.

Pure refactor — behavior identical after it: ⌘E toggles `Edit ↔ Preview`; `Diff` exists but nothing enters it yet.

- [ ] **Step 1: Write/adjust failing test** — the existing workspace test covering the preview toggle changes shape:

```rust
#[test]
fn preview_toggles_between_edit_and_preview() {
    // adapt the existing preview-toggle test to assert on EditorView
    // ... open a markdown file, assert view == EditorView::Edit,
    // dispatch TogglePreview, assert EditorView::Preview, dispatch
    // again, assert EditorView::Edit.
}
```

If no such entity-level test exists, add one following the pattern of
existing workspace tests (they construct the workspace with a test
App context).

- [ ] **Step 2: Refactor** — replace `preview: bool` with `view: EditorView` at the definition and every use site (`rg 'preview' src/workspace.rs`). `TogglePreview` maps `Edit|Diff → Preview`, `Preview → Edit`.

- [ ] **Step 3: Run full suite to green** (refactor is done only when everything compiles and passes).

- [ ] **Step 4: Commit** — `refactor: editor tab view enum (Edit/Preview/Diff)`

---

### Task 6: Diff view — state, action, rendering overlay, empty states

**Files:**
- Modify: `src/editor/mod.rs` (DiffState, compute, overlay painting), `src/workspace.rs` (ShowChanges action, header strip, empty states), `src/main.rs` (action + `cmd-shift-d` binding, View menu item), `src/workspace.rs` SHORTCUTS table
- Test: engine-adjacent logic in `src/editor/mod.rs` tests; view transitions in workspace tests

**Interfaces:**
- Consumes: `diff::diff_doc`, `git::{head_text, Baseline}`, `EditorView::Diff`, `theme.diff_*`.
- Produces: `Editor::compute_diff(&mut self, cx)` storing `Option<DiffState>`; `DiffState { doc: DiffDoc, spans: Vec<(Range<usize>, StyleKind)>, missing: Option<git::Baseline>, adds: usize, dels: usize }`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn diff_state_counts_adds_and_dels() {
    // pure: build DiffState::from_texts("a\nb\n", "a\nc\n", path_is_markdown)
    let s = DiffState::from_texts("a\nb\n", "a\nc\n", true);
    assert_eq!((s.adds, s.dels), (1, 1));
    assert!(!s.doc.changes.is_empty());
    assert!(!s.spans.is_empty()); // styled markdown spans over merged text
}

#[test]
fn show_changes_outside_repo_sets_missing_state() {
    // workspace-level: open file in a non-repo tempdir, dispatch
    // ShowChanges, assert view == EditorView::Diff and the editor's
    // diff state carries missing == Some(Baseline::NotInRepo).
}
```

`DiffState::from_texts(old, new, markdown: bool)` is a pure
constructor (runs `diff_doc`, then `markdown_spans` over the merged
text for markdown, or the fence/`Syntax` path used for code files) so
the counting/styling logic tests without git or GPUI.

- [ ] **Step 2: Run, watch fail.**

- [ ] **Step 3: Implement state + action flow**

- `ShowChanges` action registered like `TogglePreview`; bound to `cmd-shift-d`; View menu item "Show Changes"; SHORTCUTS row ("Show Changes (diff vs git HEAD)", "⌘⇧D").
- On dispatch: if `view == Diff` → back to `Edit`. Else call `git::head_text(path)`; `Baseline::Text(old)` → `DiffState::from_texts(&old, &buffer_text, is_markdown)`; other variants → `DiffState` with `missing: Some(variant)` and empty doc. Set `view = Diff`.
- Escape in Diff → `Edit` (extend existing escape handling).
- Recompute `DiffState` after save/flush and after watcher-driven reload while in Diff.

- [ ] **Step 4: Implement rendering**

In the diff branch of the editor's render path:
- If `missing.is_some()` or `doc.changes.is_empty()`: centered message per spec ("Not in a git repository." / "Not tracked in git yet." / "No text baseline at HEAD." / "No uncommitted changes.") in `muted` color — same construction as existing empty finder-preview state.
- Else: render `doc.text` through the preview pipeline with `DiffState.spans`; the line renderer receives the changes intersecting each line (slice the sorted change list against the line's byte range, the way `display_line` slices style spans) and paints per shaped sub-range: Added → `diff_added_bg` wash + `diff_added_fg`; Deleted → `diff_deleted_bg` wash + `diff_deleted_fg` + strikethrough (GPUI `StrikethroughStyle` on the text run). Block widgets (tables/images) are not projected in Diff — lines render as raw source.
- Header strip above content: left `Changes vs HEAD · +{adds} −{dels}`, right `esc to close`, styled like the find bar.
- All editing input inert in Diff (same guard as Preview).

- [ ] **Step 5: Run tests to green, full suite, manual smoke** (`cargo run` in a repo, edit a file, ⌘⇧D).

- [ ] **Step 6: Commit** — `feat: Show Changes diff view with styled word-level marks`

---

### Task 7: Code-mode diff rendering

**Files:**
- Modify: `src/editor/mod.rs`

**Interfaces:**
- Consumes: `DiffState`, `is_code_mode()`, existing gutter rendering.
- Produces: gutter behavior for diff: merged lines numbered sequentially skipping fully-deleted lines, which show `-`.

- [ ] **Step 1: Write failing test** (pure helper)

```rust
#[test]
fn diff_gutter_numbers_skip_deleted_lines() {
    let doc = diff_doc("a\nb\nc\n", "a\nc\n"); // "b\n" deleted
    let labels = diff_gutter_labels(&doc);
    assert_eq!(labels, vec!["1".to_string(), "-".to_string(), "2".to_string()]);
}
```

`pub fn diff_gutter_labels(doc: &DiffDoc) -> Vec<String>` lives in
`src/diff.rs`: walk merged lines; a line whose full byte range lies
inside a `Deleted` change gets `"-"`, others get the next new-file
number.

- [ ] **Step 2: Run, watch fail. Implement. Green.**

- [ ] **Step 3: Wire into code-mode diff render** — in Diff view for code files, gutter uses `diff_gutter_labels`; changed lines get full-width line washes (Added/Deleted bg over the line) in addition to word-level marks. Manual smoke on a `.rs` file.

- [ ] **Step 4: Full suite. Commit** — `feat: code-mode diff rendering with diff-aware gutter`

---

### Task 8: Sidebar modified dots

**Files:**
- Modify: `src/workspace.rs` (status set + refresh + row dot)

**Interfaces:**
- Consumes: `git::modified_paths`.
- Produces: `Workspace.git_modified: HashSet<PathBuf>` (workspace-root-relative), refreshed at open, watcher drain (only when events arrived), and after save/flush.

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn refresh_git_status_populates_modified_set() {
    // build tempdir repo (reuse the repo_with_commit pattern from
    // src/git.rs tests via a local helper), modify a file, construct
    // workspace over it, call refresh_git_status, assert the relative
    // path is in workspace.git_modified.
}
```

- [ ] **Step 2: Run, watch fail. Implement:**

- `fn refresh_git_status(&mut self)` → `self.git_modified = git::modified_paths(&self.root)`; call at workspace construction, at the end of each watcher drain tick that delivered events, and in the save/flush notification path.
- File row render: after the filename, `div().flex_1()`, then when `self.git_modified.contains(rel_path)` a `div().size(px(5.)).rounded_full().bg(theme.accent)` aligned center — mirroring how the chevron slot is laid out.

- [ ] **Step 3: Green, full suite, manual smoke** (edit a file, dot appears; commit outside, dot clears on next event).

- [ ] **Step 4: Commit** — `feat: sidebar dots for uncommitted changes`

---

### Task 9: Docs + finishing gate

**Files:**
- Modify: `README.md` (feature list), `WELCOME.md` (mention ⌘⇧D)

- [ ] **Step 1: README bullet** — add to "What it does": `**Show Changes** — ⌘⇧D diffs the open file against git HEAD, word-level marks rendered in the editor's own typography; modified files get a dot in the sidebar. Read-only: SuperMD never writes to your repo.` Adjust WELCOME.md shortcuts list.

- [ ] **Step 2: Full suite + `cargo build --release` smoke.**

- [ ] **Step 3: Commit** — `docs: Show Changes in README and welcome`

- [ ] **Step 4: Finishing gate** — REQUIRED SUB-SKILL: superpowers:finishing-a-development-branch.

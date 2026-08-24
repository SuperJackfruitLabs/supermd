# Workspace Hygiene, Project Search & Preview Tabs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ignore-aware file listing, finder match highlighting, streaming project-wide search (⌘⇧F), git2→gix migration, and VS Code-style preview tabs.

**Architecture:** The `ignore` crate becomes the single walking/filter layer for sidebar, finder, watcher, and search. Project search is a pure engine (`src/search.rs`) on ripgrep's `grep-searcher`, streamed from the background executor into a finder-family overlay. `src/git.rs` keeps its tested public API and swaps internals to `gix`. Preview tabs are one workspace-level `Option<usize>` slot with promotion rules.

**Tech Stack:** `ignore`, `grep-searcher` + `grep-regex`, `gix`, `nucleo-matcher` (already present), GPUI 0.2.2.

**Spec:** `docs/superpowers/specs/2026-08-24-workspace-hygiene-search-design.md`

## Global Constraints

- Search cap: 500 matches, UI shows "(capped)" when hit. Debounce 120 ms.
- Smart case everywhere: query with no uppercase → case-insensitive.
- `git.rs` public API frozen: `Baseline{Text,NotInRepo,Untracked,Binary}`, `head_text(&Path) -> Baseline`, `modified_paths(&Path) -> HashSet<PathBuf>` (root-relative paths).
- git2 and vendored libgit2 must be fully removed by the end of Task 5.
- Keybindings: ⌘⇧F = project search, ⌃⌘F = focus mode; update SHORTCUTS table + menus with each change.
- TDD; commit per task; full suite green before each commit.

---

### Task 1: Ignore-aware walking

**Files:**
- Modify: `src/files.rs` (`FileTree::load`, `all_files`), `src/workspace.rs` (watcher drain filter), `Cargo.toml`

**Interfaces:**
- Produces: `files::workspace_walk(root: &Path) -> ignore::Walk` — the canonical WalkBuilder config (gitignore on, hidden off, `.git` excluded implicitly); used by Task 3's search. `FileTree::load`/`all_files` filter through the same rules.

- [ ] **Step 1:** `cargo add ignore`

- [ ] **Step 2: Failing tests** (in `src/files.rs` tests)

```rust
#[test]
fn listing_respects_gitignore_and_hides_dotfiles() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join("target/junk.txt"), "x").unwrap();
    std::fs::write(dir.path().join(".hidden.md"), "x").unwrap();
    std::fs::write(dir.path().join("kept.md"), "x").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
    let mut tree = FileTree::new(dir.path().to_path_buf());
    let names: Vec<String> = tree.visible().into_iter().map(|(_, e)| e.name).collect();
    assert!(names.contains(&"kept.md".to_string()));
    assert!(!names.contains(&"target".to_string()));
    assert!(!names.iter().any(|n| n.starts_with('.')));
}

#[test]
fn all_files_respects_ignore_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("node_modules")).unwrap();
    std::fs::write(dir.path().join("node_modules/dep.js"), "x").unwrap();
    std::fs::write(dir.path().join("a.md"), "x").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
    let tree = FileTree::new(dir.path().to_path_buf());
    let files = tree.all_files(1000);
    assert!(files.iter().any(|p| p.ends_with("a.md")));
    assert!(!files.iter().any(|p| p.to_string_lossy().contains("node_modules")));
}
```

Note: `.gitignore` applies even without `git init` when the walker sets
`require_git(false)` — configure `workspace_walk` accordingly so
non-repo folders still honor a `.gitignore` and tests need no repo.

- [ ] **Step 3:** Run, watch fail (ignored entries currently listed).

- [ ] **Step 4: Implement**

```rust
/// Canonical workspace walker: gitignore (even without git), global
/// gitignore off, hidden files skipped, .git skipped.
pub fn workspace_walk(root: &Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .git_global(false)
        .git_exclude(true)
        .build()
}

/// True if `path` (inside `root`) survives the workspace ignore rules.
pub fn is_visible(root: &Path, path: &Path) -> bool { ... }
```

`FileTree::load(dir)` builds its single-level listing via
`WalkBuilder::new(dir).max_depth(Some(1))` plus the same flags, with
`add_custom_ignore_filename` unnecessary; sort as today (dirs first,
then names). `all_files` uses `workspace_walk(root)` with the existing
limit. `is_visible` walks the parent chain against a
`ignore::gitignore::GitignoreBuilder` rooted at `root` — used by the
watcher: in `on_fs_events`, drop paths where `!is_visible` (and hidden
components) before refresh/reload logic, so `target/` churn is silent.

- [ ] **Step 5:** Green, full suite, commit `feat: ignore-aware file walking`.

---

### Task 2: Finder match highlighting

**Files:**
- Modify: `src/finder.rs`

**Interfaces:**
- Produces: `Finder` rows render matched characters in accent color. Internal: `rescore` keeps `Vec<(usize, Vec<u32>)>` (candidate ix, match char-indices) via `Pattern::indices`.

- [ ] **Step 1: Failing test** — pure scoring helper extracted:

```rust
#[test]
fn match_indices_returned_for_highlighting() {
    let (order, indices) = score_candidates("rm", &["readme.md".into(), "zzz.txt".into()]);
    assert_eq!(order, vec![0]);
    let hit: Vec<u32> = indices[0].clone();
    assert!(!hit.is_empty()); // 'r' and 'm' positions in "readme.md"
}
```

`fn score_candidates(query: &str, rels: &[String]) -> (Vec<usize>, Vec<Vec<u32>>)`
extracted from `rescore` (same Pattern/Matcher config, but calling
`pattern.indices(haystack, &mut matcher, &mut vec)` instead of
`score`; sort by score desc, truncate MAX_RESULTS).

- [ ] **Step 2:** Run/fail/implement/green.

- [ ] **Step 3:** Row render: split the filename text into per-char runs —
matched chars `t.accent` + semibold, others unchanged. (Rows are plain
`div` children; build a `Vec<AnyElement>` of styled spans from the
index set — mirror the SHORTCUTS-dialog styling pattern.)

- [ ] **Step 4:** Full suite, manual smoke (⌘P, verify highlights), commit `feat: finder match highlighting via nucleo indices`.

---

### Task 3: Project search engine

**Files:**
- Create: `src/search.rs`; Modify: `src/main.rs` (`mod search;`), `Cargo.toml`

**Interfaces:**
- Consumes: `files::workspace_walk`.
- Produces:

```rust
pub struct SearchMatch {
    pub path: PathBuf,              // workspace-relative
    pub line_number: u64,           // 1-based
    pub line_text: String,          // trimmed, see below
    pub ranges: Vec<Range<usize>>,  // byte ranges of hits in line_text
}
pub const SEARCH_CAP: usize = 500;
pub fn search_workspace(
    root: &Path, query: &str,
    cancelled: &std::sync::atomic::AtomicBool,
    tx: std::sync::mpsc::Sender<Vec<SearchMatch>>,
) -> bool; // true if capped
```

- [ ] **Step 1:** `cargo add grep-searcher grep-regex grep-matcher`

- [ ] **Step 2: Failing tests**

```rust
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.md"), "Alpha beta\ngamma ALPHA\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "let alpha = 1;\n").unwrap();
    std::fs::create_dir(dir.path().join("skip")).unwrap();
    std::fs::write(dir.path().join("skip/c.md"), "alpha\n").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "skip/\n").unwrap();
    dir
}

#[test]
fn smart_case_lowercase_matches_all_cases() {
    let dir = fixture();
    let (tx, rx) = std::sync::mpsc::channel();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    search_workspace(dir.path(), "alpha", &cancelled, tx);
    let all: Vec<SearchMatch> = rx.iter().flatten().collect();
    assert_eq!(all.len(), 3); // a.md ×2 lines... (Alpha, ALPHA), b.rs ×1
    assert!(all.iter().all(|m| !m.path.starts_with("skip")));
    assert!(all.iter().all(|m| !m.ranges.is_empty()));
}

#[test]
fn smart_case_uppercase_is_exact() {
    let dir = fixture();
    let (tx, rx) = std::sync::mpsc::channel();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    search_workspace(dir.path(), "ALPHA", &cancelled, tx);
    let all: Vec<SearchMatch> = rx.iter().flatten().collect();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].line_number, 2);
}

#[test]
fn cancellation_stops_stream() {
    let dir = fixture();
    let (tx, rx) = std::sync::mpsc::channel();
    let cancelled = std::sync::atomic::AtomicBool::new(true); // pre-cancelled
    search_workspace(dir.path(), "alpha", &cancelled, tx);
    assert_eq!(rx.iter().flatten().count(), 0);
}

#[test]
fn empty_query_returns_nothing() {
    let dir = fixture();
    let (tx, rx) = std::sync::mpsc::channel();
    let c = std::sync::atomic::AtomicBool::new(false);
    assert!(!search_workspace(dir.path(), "", &c, tx));
    assert_eq!(rx.iter().count(), 0);
}
```

- [ ] **Step 3:** Run, fail (module absent).

- [ ] **Step 4: Implement** — matcher:
`RegexMatcherBuilder::new().fixed_strings(true).case_insensitive(no_upper).build(query)`;
per walked file (files only) run `Searcher::new()` with
`SinkFn`-style closure (implement `grep_searcher::Sink` on a small
struct) collecting one `SearchMatch` per matched line:
`line_text` = the line trimmed of trailing newline, truncated to 240
bytes on a char boundary (append "…"); `ranges` from
`grep_matcher::Matcher::find_iter` over the (untruncated) line,
clipped to the truncation window. Batch per file (send one
`Vec<SearchMatch>` per file with hits). Check `cancelled` before each
file and inside the sink (return `Ok(false)` to stop). Stop the walk
at `SEARCH_CAP` total matches and return `true`.

- [ ] **Step 5:** Green, full suite, commit `feat: streaming project search engine on ripgrep crates`.

---

### Task 4: Search overlay + keybindings

**Files:**
- Modify: `src/workspace.rs` (SearchState, overlay render, actions), `src/main.rs` (bindings, menu), `src/finder.rs` only if preview helper needs `pub(crate)`.

**Interfaces:**
- Consumes: `search::{search_workspace, SearchMatch, SEARCH_CAP}`, finder's `PreviewContent`/`load_preview` pattern, `TextInput`.
- Produces: workspace actions `ToggleSearch`, `SearchUp`, `SearchDown`, `SearchConfirm`, `SearchDismiss`; `ToggleFocusMode` rebound.

- [ ] **Step 1: State + actions** — add to workspace `actions!` list; state:

```rust
struct SearchState {
    input: Entity<crate::input::TextInput>,
    matches: Vec<search::SearchMatch>,   // streamed in, grouped later
    selected: usize,
    capped: bool,
    searching: bool,
    generation: u64,                      // stale-result guard
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    debounce: Option<gpui::Task<()>>,
    _subscription: gpui::Subscription,    // input changes → debounce restart
}
```

On query change: cancel previous (`cancelled.store(true)`), bump
`generation`, spawn 120 ms `cx.background_executor().timer` then run
`search_workspace` on `cx.background_executor().spawn`, polling the
mpsc receiver from an async loop that updates the entity (matches +
notify) — same spawn/update pattern as the watcher drain loop.

- [ ] **Step 2: Render** — 680×440 fixed two-pane overlay (copy the
finder dialog skeleton): left = input + scrollable match list
(uniform_list; rows: file-header rows when `path` differs from the
previous match, then match rows "  {line_number}  {line_text}" with
`ranges` washed `find_match_bg`), right = preview via the finder's
preview machinery, selected match line highlighted; footer line shows
"N matches" / "500+ (capped)" / "Type to search the workspace" / "No
matches". `.occlude()` like finder; key context "Search".

- [ ] **Step 3: Keys/menus** — main.rs:

```rust
KeyBinding::new("cmd-shift-f", workspace::ToggleSearch, None),
KeyBinding::new("ctrl-cmd-f", ToggleFocusMode, None),  // was cmd-shift-f
KeyBinding::new("up", workspace::SearchUp, Some("Search")),
KeyBinding::new("down", workspace::SearchDown, Some("Search")),
KeyBinding::new("enter", workspace::SearchConfirm, Some("Search")),
KeyBinding::new("escape", workspace::SearchDismiss, Some("Search")),
```

Menu: View → "Search in Workspace…". SHORTCUTS: "⌘ ⇧ F Search in
workspace", "⌃ ⌘ F Focus mode" (replace old row). Confirm → open
permanent tab + `Editor::scroll_to_line(line_number - 1)`.

- [ ] **Step 4:** Full suite, manual smoke (search this repo for "Baseline"), commit `feat: project-wide search overlay (cmd-shift-f)`.

---

### Task 5: git2 → gix

**Files:**
- Modify: `src/git.rs` (internals + test fixtures), `Cargo.toml`

**Interfaces:**
- Public API frozen (Global Constraints). Test helper changes to git CLI:

```rust
fn sh_git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
        .args(args).current_dir(dir)
        .status().unwrap().success();
    assert!(ok, "git {args:?} failed");
}
// repo_with_commit: sh_git(&["init","-q"]); write files; sh_git(&["add","-A"]); sh_git(&["commit","-qm","init"]);
```

- [ ] **Step 1:** Rewrite the two fixture helpers to the CLI (keep every
`#[test]` assertion identical). Run: still green on git2 — proves the
fixtures are backend-neutral.

- [ ] **Step 2:** `cargo remove git2 && cargo add gix --no-default-features --features status,index,blob-diff,revision`
(then `cargo check`; add/remove gix features until `discover`,
`head_commit`, tree `lookup_entry`, blob data, and `status` compile —
record the final list in the commit message).

- [ ] **Step 3: Swap internals** (tests now RED — git2 gone):

```rust
pub fn head_text(path: &Path) -> Baseline {
    let Some(parent) = path.parent() else { return Baseline::NotInRepo };
    let Ok(repo) = gix::discover(parent) else { return Baseline::NotInRepo };
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return Baseline::NotInRepo;
    };
    let (Ok(canon), Ok(canon_workdir)) = (path.canonicalize(), workdir.canonicalize())
    else { return Baseline::Untracked };
    let Ok(rel) = canon.strip_prefix(&canon_workdir) else { return Baseline::NotInRepo };
    let Ok(commit) = repo.head_commit() else { return Baseline::Untracked };
    let Ok(tree) = commit.tree() else { return Baseline::Untracked };
    let Ok(Some(entry)) = tree.lookup_entry_by_path(rel) else {
        return Baseline::Untracked;
    };
    let Ok(obj) = entry.object() else { return Baseline::Untracked };
    match std::str::from_utf8(&obj.data) {
        Ok(s) => Baseline::Text(s.to_string()),
        Err(_) => Baseline::Binary,
    }
}
```

`modified_paths`: `repo.status(gix::progress::Discard)` →
`.into_index_worktree_iter(Vec::new())` (API per gix docs; adjust to
the compiling form), untracked included, ignored excluded (gix status
defaults exclude ignored); map each item's `rel_path()` to
`workdir.join(rel)` and re-relativize to canonicalized `root` exactly
as the git2 version did.

- [ ] **Step 4:** All 7 git tests green unchanged; `grep -r git2 Cargo.toml src/` empty; full suite; commit `refactor: pure-Rust git via gix, fixtures via git CLI`.

---

### Task 6: Preview tabs

**Files:**
- Modify: `src/workspace.rs`

**Interfaces:**
- Produces: `Workspace.preview_tab: Option<usize>`;
`fn open_path_preview(&mut self, path: &Path, window, cx)`;
`open_path` stays the permanent-open (and clears `preview_tab` if it
lands on that index). Pure helper for the state rule:

```rust
/// (slot_to_replace, make_new_tab) given existing tab paths, the
/// preview slot, and the target's existing-tab index if any.
pub(crate) fn preview_plan(
    preview: Option<usize>, existing_ix: Option<usize>,
) -> PreviewPlan;   // enum { ActivateExisting(usize), ReplacePreview(usize), PushNew }
```

- [ ] **Step 1: Failing tests** (pure):

```rust
#[test]
fn preview_plan_rules() {
    use PreviewPlan::*;
    assert_eq!(preview_plan(None, Some(2)), ActivateExisting(2));
    assert_eq!(preview_plan(Some(1), Some(1)), ActivateExisting(1));
    assert_eq!(preview_plan(Some(1), None), ReplacePreview(1));
    assert_eq!(preview_plan(None, None), PushNew);
}
```

- [ ] **Step 2:** Run/fail/implement/green.

- [ ] **Step 3: Wire** —
`open_path_preview`: flush the tab being replaced, build the new tab
entity (same construction as `open_path`'s file branch), apply the
plan (`ReplacePreview` writes `self.tabs[ix]` and activates;
`PushNew` pushes + sets `preview_tab`). Promotion (clear
`preview_tab`): `open_path` on the previewed path; sidebar Enter
(`sidebar_open`); double-click on file row (`ClickEvent::click_count(&self) -> usize`
exists in gpui 0.2.2 — single click (count 1) →
`open_path_preview`, count ≥2 → `open_path`); double-click on the tab
strip tab; editor dirty transition (in the save-notify path the
workspace already touches, or check `editor.save.is_dirty()` on
render — pick the observer the code makes natural and note it in the
commit). Sidebar ↑/↓ (`sidebar_move`): after moving selection, if the
selected row is a file, `open_path_preview` it. Closing a tab adjusts
`preview_tab` indices (decrement if greater, clear if equal).
Tab title render: `.italic()` when `Some(ix) == preview_tab`.

- [ ] **Step 4:** Full suite; manual smoke: arrow through sidebar (previews replace one italic tab), Enter pins, double-click pins, editing pins, ⌘P still opens permanent.

- [ ] **Step 5:** Commit `feat: preview tabs for sidebar browsing`.

---

### Task 7: Docs + release v0.0.4

**Files:**
- Modify: `README.md`, `WELCOME.md`

- [ ] **Step 1:** README: add search bullet ("**Search in workspace** —
⌘⇧F streams ripgrep-powered results into a two-pane overlay"),
mention ignore-aware sidebar/finder, preview tabs, and swap the git
mention to "pure-Rust git (gix)". WELCOME: update focus-mode chord to
⌃⌘F, add ⌘⇧F line. SHORTCUTS table verified consistent with main.rs
bindings (constraint from workspace.rs:51 comment).

- [ ] **Step 2:** Full suite + `cargo build --release` + manual smoke of all five features together.

- [ ] **Step 3:** Commit `docs: workspace search, preview tabs, gix in README/WELCOME`; push master.

- [ ] **Step 4:** Tag `v0.0.4`, push tag, watch the release workflow
(build → sign → notarize → staple → publish). Report the release URL.

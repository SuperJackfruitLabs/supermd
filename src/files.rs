//! Folder-as-workspace: a lazily loaded, expandable view of the file system.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Belt-and-braces fallback for folders without a .gitignore.
const IGNORED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".build",
    "dist",
    "out",
    "__pycache__",
    ".venv",
];

/// macOS puts these behind a per-folder consent prompt. Indexing them for
/// Markdown is wasted work, and walking one raises a modal the user has to
/// answer for no benefit — opening the Home folder used to raise three.
///
/// Only skipped directly under the workspace root, which is where the
/// protected folders live. A `Pictures` folder nested inside a notes
/// repository is ordinary and still indexed.
const PROTECTED_ROOT_DIRS: &[&str] = &["Music", "Pictures", "Movies"];

/// Whether the walk should descend into `path`, given the workspace root.
/// Pure so the rule is testable without touching a real protected folder.
pub fn should_descend(root: &Path, path: &Path, is_dir: bool) -> bool {
    if !is_dir {
        return true;
    }
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return true;
    };
    if IGNORED_DIRS.contains(&name.as_str()) {
        return false;
    }
    // Depth 1 only: the parent of a protected folder is the root itself.
    let at_root = path.parent() == Some(root);
    !(at_root && PROTECTED_ROOT_DIRS.contains(&name.as_str()))
}

/// The walker the knowledge index (and search/`⌘P`) uses: gitignore rules
/// (even without git), hidden files skipped, well-known build dirs
/// skipped. A gitignored draft is deliberately not part of the note graph.
fn index_walk_builder(root: &Path) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(true)
        .git_ignore(true)
        .require_git(false)
        .git_global(false)
        .git_exclude(true)
        .filter_entry({
            let root = root.to_path_buf();
            move |e| {
                should_descend(&root, e.path(), e.file_type().is_some_and(|t| t.is_dir()))
            }
        });
    b
}

/// Is this path build output or VCS internals -- the churn the watcher
/// exists to ignore? This is deliberately NOT "is it gitignored": a
/// gitignored note is a note, and the sidebar shows it dimmed, so an
/// edit to one still has to refresh the tree.
///
/// `should_descend` already names these directories (`IGNORED_DIRS`,
/// and the protected folders at the root); asking it about each
/// component on the way down is the same rule, not a second list.
pub fn is_build_noise(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        // Outside the workspace: not ours to call noise.
        return false;
    };
    let mut acc = root.to_path_buf();
    let count = rel.components().count();
    for (ix, comp) in rel.components().enumerate() {
        acc.push(comp);
        // Every component but the last is a directory by construction;
        // the last may be gone already, so ask the disk.
        let is_dir = ix + 1 < count || acc.is_dir();
        if !should_descend(root, &acc, is_dir) {
            return true;
        }
    }
    false
}

/// The walker the sidebar uses. Shows gitignored files — a file tree
/// should show what is on disk — but still skips hidden files and the
/// build directories that would bury everything else.
///
/// `ignore(false)` and `parents(false)` are part of that promise:
/// `git_ignore(false)` alone still let a `.ignore`/`.rgignore` file (or
/// one above the root) delete rows outright, which is the opposite of
/// showing them dimmed. `FsEntry::ignored` still comes from
/// `IndexMatcher`, so those rows arrive dimmed rather than absent.
fn tree_walk_builder(root: &Path) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(true)
        .ignore(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .filter_entry({
            let root = root.to_path_buf();
            move |e| {
                should_descend(&root, e.path(), e.file_type().is_some_and(|t| t.is_dir()))
            }
        });
    b
}

/// Canonical workspace walker: gitignore rules (even without git),
/// hidden files skipped, well-known build dirs skipped. Used by the
/// knowledge index and search/`⌘P` — a gitignored note stays out of both.
pub fn workspace_walk(root: &Path) -> ignore::Walk {
    index_walk_builder(root).build()
}

/// The index walk's own ignore rules, asked about single paths instead
/// of driven over a tree.
///
/// Built from `index_walk_builder`, so this is the *same* rule
/// `Index::scan` walks with — nested `.gitignore`s, `.git/info/exclude`,
/// `.ignore`/`.rgignore`, and ignore files above the root included. The
/// hand-rolled root-only gitignore this replaced admitted notes the
/// scan had excluded: the watcher re-indexed them on the next save and
/// the sidebar drew them undimmed, so the one affordance saying "this
/// file is outside the index" was wrong exactly where it mattered.
///
/// A matcher is a snapshot of the ignore files it has already loaded,
/// so build one per batch and drop it; a long-lived one stops noticing
/// edits to a `.gitignore`.
pub struct IndexMatcher {
    root: PathBuf,
    /// `None` only if the builder handed back no matcher at all, which
    /// it does not for a single root; visibility then falls back to the
    /// component rules alone.
    inner: Option<ignore::IncrementalIgnore>,
}

pub fn index_matcher(root: &Path) -> IndexMatcher {
    IndexMatcher {
        root: root.to_path_buf(),
        inner: index_walk_builder(root).build_matchers().pop(),
    }
}

impl IndexMatcher {
    /// True if `path` (inside the root) survives the workspace ignore
    /// rules — the question `Index::scan`'s walk answers by arriving
    /// at the file or not.
    pub fn allows(&mut self, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return false;
        };
        // `should_descend` is the walker's `filter_entry`, and a
        // matcher cannot apply that (it needs a real directory entry),
        // so run it here over each component on the way down — plus the
        // dotfile rule, which also covers paths that no longer exist.
        let mut acc = self.root.clone();
        let count = rel.components().count();
        for (ix, comp) in rel.components().enumerate() {
            acc.push(comp);
            let is_dir = ix + 1 < count || acc.is_dir();
            if let std::path::Component::Normal(name) = comp {
                if name.to_string_lossy().starts_with('.') {
                    return false;
                }
            }
            if !should_descend(&self.root, &acc, is_dir) {
                return false;
            }
        }
        let Some(inner) = self.inner.as_mut() else {
            return true;
        };
        // `matched` wants a root-relative path with no `..`; `normalize`
        // is the crate's own way of getting one.
        let rel = inner.normalize(path).unwrap_or_else(|| rel.to_path_buf());
        !inner.matched(&rel, path.is_dir()).is_ignore()
    }
}

/// True if `path` (inside `root`) survives the workspace ignore rules.
/// One-shot form of `IndexMatcher::allows`; prefer the matcher when
/// asking about more than one path.
pub fn is_visible(root: &Path, path: &Path) -> bool {
    index_matcher(root).allows(path)
}

#[derive(Clone, Debug)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Excluded by a `.gitignore`. Listed in the tree, dimmed, and kept
    /// out of the knowledge index.
    pub ignored: bool,
}

pub struct FileTree {
    pub root: PathBuf,
    children: HashMap<PathBuf, Vec<FsEntry>>,
    expanded: HashSet<PathBuf>,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut expanded = HashSet::new();
        expanded.insert(root.clone());
        Self {
            root,
            children: HashMap::new(),
            expanded,
        }
    }

    pub fn root_name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
        }
    }

    pub fn is_expanded(&self, dir: &Path) -> bool {
        self.expanded.contains(dir)
    }

    fn load(&mut self, dir: &Path) {
        if self.children.contains_key(dir) {
            return;
        }
        let root = self.root.clone();
        let mut entries: Vec<FsEntry> = {
            let mut b = tree_walk_builder(dir);
            b.max_depth(Some(1));
            b.build()
                .flatten()
                .filter(|e| e.path() != dir)
                .filter_map(|e| {
                    let is_dir = e.file_type()?.is_dir();
                    let path = e.into_path();
                    Some(FsEntry {
                        name: path.file_name()?.to_string_lossy().into_owned(),
                        path,
                        is_dir,
                        ignored: false,
                    })
                })
                .collect()
        };
        // One matcher for the whole listing: it caches each directory's
        // ignore files, which a per-entry `is_visible` would re-read.
        let mut matcher = index_matcher(&root);
        for entry in &mut entries {
            entry.ignored = !matcher.allows(&entry.path);
        }
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.children.insert(dir.to_path_buf(), entries);
    }

    /// The rows currently visible in the sidebar, with their indent depth.
    pub fn visible(&mut self) -> Vec<(usize, FsEntry)> {
        let mut out = Vec::new();
        let root = self.root.clone();
        self.walk(&root, 0, &mut out);
        out
    }

    fn walk(&mut self, dir: &Path, depth: usize, out: &mut Vec<(usize, FsEntry)>) {
        self.load(dir);
        let entries = self.children.get(dir).cloned().unwrap_or_default();
        for entry in entries {
            out.push((depth, entry.clone()));
            if entry.is_dir && self.is_expanded(&entry.path) {
                self.walk(&entry.path, depth + 1, out);
            }
        }
    }

    /// Drop cached listings so the next render re-reads the disk.
    pub fn refresh(&mut self) {
        self.children.clear();
    }

    /// Expand every ancestor directory of `path` (so the row for a just-
    /// opened file is present in the visible tree).
    pub fn expand_to(&mut self, path: &Path) {
        for ancestor in path.ancestors().skip(1) {
            if !ancestor.starts_with(&self.root) || ancestor == self.root {
                break;
            }
            self.expanded.insert(ancestor.to_path_buf());
        }
    }

    /// All files under the root (for the fuzzy finder). Bounded to keep
    /// pathological folders from stalling the UI.
    pub fn all_files(&self, limit: usize) -> Vec<PathBuf> {
        workspace_walk(&self.root)
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .map(|e| e.into_path())
            .take(limit)
            .collect()
    }
}

/// Files the image viewer tab opens (everything else goes to the editor).
pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico"
            )
        })
}

/// First free untitled name: "Untitled.md", then "Untitled 2.md", …
pub fn pick_untitled(existing: &[String]) -> String {
    let taken = |name: &str| existing.iter().any(|e| e == name);
    if !taken("Untitled.md") {
        return "Untitled.md".into();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("Untitled {n}.md");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_folders_are_skipped_only_at_the_workspace_root() {
        let root = Path::new("/w");
        // macOS prompts for each of these; indexing them finds nothing
        // useful, and opening the Home folder used to raise three modals.
        for name in ["Music", "Pictures", "Movies"] {
            assert!(
                !should_descend(root, &root.join(name), true),
                "{name} at the root should be skipped"
            );
        }
        // Nested ones are ordinary folders and must still be walked — a
        // notes repository may legitimately contain Pictures/.
        assert!(should_descend(root, Path::new("/w/notes/Pictures"), true));
        assert!(should_descend(root, Path::new("/w/project/Music"), true));
    }

    /// The pure rule above is only useful if the walker actually consults
    /// it — this covers the filter_entry wiring, where a regression would
    /// silently reintroduce the consent prompts.
    #[test]
    fn workspace_walk_skips_protected_root_folders_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["Music", "Pictures", "Movies", "notes", "notes/Pictures", "target"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in [
            "top.md",
            "Music/a.md",
            "Pictures/b.md",
            "Movies/c.md",
            "notes/d.md",
            "notes/Pictures/e.md",
            "target/f.md",
        ] {
            std::fs::write(root.join(f), "# x").unwrap();
        }

        let found: Vec<String> = workspace_walk(root)
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
            .map(|e| {
                e.path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        assert!(found.contains(&"top.md".to_string()));
        assert!(found.contains(&"notes/d.md".to_string()));
        // Nested Pictures is an ordinary folder and stays indexed.
        assert!(found.contains(&"notes/Pictures/e.md".to_string()));
        // Protected roots and build dirs never contribute.
        for skipped in ["Music/a.md", "Pictures/b.md", "Movies/c.md", "target/f.md"] {
            assert!(!found.contains(&skipped.to_string()), "{skipped} should be skipped");
        }
    }

    #[test]
    fn build_dirs_are_skipped_at_any_depth_and_files_always_descend() {
        let root = Path::new("/w");
        assert!(!should_descend(root, Path::new("/w/target"), true));
        assert!(!should_descend(root, Path::new("/w/a/b/node_modules"), true));
        assert!(should_descend(root, Path::new("/w/src"), true));
        // A *file* named Music is not a protected folder.
        assert!(should_descend(root, &root.join("Music"), false));
    }

    /// The sidebar shows what the index excludes, dimmed. A .ignore
    /// file made rows vanish instead, which is the one affordance
    /// telling the user a file is outside the index.
    #[test]
    fn ignore_files_are_dimmed_in_the_sidebar_not_omitted() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".ignore"), "drafts/\n").unwrap();
        std::fs::create_dir(root.path().join("drafts")).unwrap();
        std::fs::write(root.path().join("drafts/x.md"), "# x\n").unwrap();
        let mut tree = FileTree::new(root.path().to_path_buf());
        let rows = tree.visible();
        let names: Vec<&str> = rows.iter().map(|(_, e)| e.name.as_str()).collect();
        assert!(names.contains(&"drafts"), "present: {names:?}");
        let entry = rows.iter().find(|(_, e)| e.name == "drafts").expect("the row is there");
        assert!(entry.1.ignored, "and dimmed");
    }

    /// Build noise silences the watcher; a gitignored note does not.
    /// The old gate conflated them, so writing to a gitignored file
    /// from a terminal left the sidebar stale until something else
    /// happened.
    #[test]
    fn only_build_noise_silences_the_watcher() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".gitignore"), "drafts/\n").unwrap();
        assert!(
            !is_build_noise(root.path(), &root.path().join("drafts/secret.md")),
            "a gitignored note still refreshes the sidebar"
        );
        assert!(
            is_build_noise(root.path(), &root.path().join("target/debug/x.o")),
            "build output does not"
        );
        assert!(is_build_noise(root.path(), &root.path().join(".git/index")));
    }

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
        assert!(names.contains(&"kept.md".to_string()), "{names:?}");
        assert!(!names.contains(&"target".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n.starts_with('.')), "{names:?}");
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
        assert!(files.iter().any(|p| p.ends_with("a.md")), "{files:?}");
        assert!(
            !files.iter().any(|p| p.to_string_lossy().contains("node_modules")),
            "{files:?}"
        );
    }

    #[test]
    fn is_visible_rejects_ignored_and_hidden_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        assert!(is_visible(dir.path(), &dir.path().join("src/main.rs")));
        assert!(!is_visible(dir.path(), &dir.path().join("target/debug/app")));
        assert!(!is_visible(dir.path(), &dir.path().join(".git/HEAD")));
        assert!(!is_visible(dir.path(), &dir.path().join("a/.hidden")));
    }

    /// The watcher and the sidebar ask `is_visible`; `Index::scan`
    /// walks with `index_walk_builder`. They have to be one rule: a
    /// nested `.gitignore` excluded a note from the scan, `is_visible`
    /// let the watcher re-admit it the moment the user saved, and the
    /// sidebar drew it undimmed while it sat in the graph.
    #[test]
    fn is_visible_honours_the_same_ignore_files_the_index_walk_does() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join(".gitignore"), "private.md\n").unwrap();
        std::fs::write(notes.join("private.md"), "secret\n").unwrap();
        std::fs::write(notes.join("public.md"), "fine\n").unwrap();
        // `.ignore` and `.git/info/exclude` count for the walker too.
        std::fs::write(dir.path().join(".ignore"), "drafts/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("drafts")).unwrap();
        std::fs::write(dir.path().join("drafts/d.md"), "draft\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".git/info")).unwrap();
        std::fs::write(dir.path().join(".git/info/exclude"), "excluded.md\n").unwrap();
        std::fs::write(dir.path().join("excluded.md"), "x\n").unwrap();

        let walked: Vec<PathBuf> = workspace_walk(dir.path())
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .map(|e| e.into_path())
            .collect();
        for path in [
            notes.join("private.md"),
            dir.path().join("drafts/d.md"),
            dir.path().join("excluded.md"),
        ] {
            assert!(!walked.contains(&path), "the index walk skips {path:?}");
            assert!(!is_visible(dir.path(), &path), "so must is_visible: {path:?}");
        }
        assert!(walked.contains(&notes.join("public.md")));
        assert!(is_visible(dir.path(), &notes.join("public.md")));
    }

    /// And the sidebar dims exactly what the index drops.
    #[test]
    fn the_tree_marks_a_nested_gitignored_file_as_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join(".gitignore"), "private.md\n").unwrap();
        std::fs::write(notes.join("private.md"), "secret\n").unwrap();
        std::fs::write(notes.join("public.md"), "fine\n").unwrap();

        let mut tree = FileTree::new(dir.path().to_path_buf());
        tree.toggle(&notes);
        let rows = tree.visible();
        let find = |name: &str| {
            rows.iter()
                .find(|(_, e)| e.name == name)
                .unwrap_or_else(|| panic!("{name} listed: {rows:?}"))
                .1
                .ignored
        };
        assert!(find("private.md"), "dimmed, like the index drops it");
        assert!(!find("public.md"));
    }

    #[test]
    fn is_visible_rejects_paths_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_visible(dir.path(), Path::new("/elsewhere/file.md")));
    }

    #[test]
    fn is_visible_allows_paths_with_parent_components() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_visible(dir.path(), &dir.path().join("a/../b.md")));
    }

    #[test]
    fn is_visible_rejects_gitignored_custom_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
        assert!(!is_visible(dir.path(), &dir.path().join("build/x.md")));
        assert!(is_visible(dir.path(), &dir.path().join("src/x.md")));
    }

    #[test]
    fn root_name_uses_last_component_or_display() {
        assert_eq!(FileTree::new(PathBuf::from("/tmp/proj")).root_name(), "proj");
        assert_eq!(FileTree::new(PathBuf::from("/")).root_name(), "/");
    }

    #[test]
    fn toggle_flips_expansion() {
        let mut tree = FileTree::new(PathBuf::from("/root"));
        assert!(tree.is_expanded(Path::new("/root")));
        tree.toggle(Path::new("/root"));
        assert!(!tree.is_expanded(Path::new("/root")));
        tree.toggle(Path::new("/root"));
        assert!(tree.is_expanded(Path::new("/root")));
    }

    #[test]
    fn visible_sorts_dirs_first_then_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("zeta")).unwrap();
        std::fs::create_dir(dir.path().join("Alpha")).unwrap();
        std::fs::write(dir.path().join("b.md"), "x").unwrap();
        std::fs::write(dir.path().join("A.md"), "x").unwrap();
        let mut tree = FileTree::new(dir.path().to_path_buf());
        let names: Vec<String> = tree.visible().into_iter().map(|(_, e)| e.name).collect();
        assert_eq!(names, vec!["Alpha", "zeta", "A.md", "b.md"]);
    }

    #[test]
    fn nested_expansion_and_refresh_reread_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/inner.md"), "x").unwrap();
        let mut tree = FileTree::new(dir.path().to_path_buf());
        // Collapsed subdir hides its contents.
        assert!(!tree.visible().iter().any(|(_, e)| e.name == "inner.md"));
        tree.toggle(&dir.path().join("sub"));
        let rows = tree.visible();
        let inner = rows.iter().find(|(_, e)| e.name == "inner.md");
        assert_eq!(inner.map(|(depth, _)| *depth), Some(1));
        // Cached listing ignores new files until refresh.
        std::fs::write(dir.path().join("late.md"), "x").unwrap();
        assert!(!tree.visible().iter().any(|(_, e)| e.name == "late.md"));
        tree.refresh();
        assert!(tree.visible().iter().any(|(_, e)| e.name == "late.md"));
    }

    #[test]
    fn expand_to_opens_all_ancestors() {
        let mut tree = FileTree::new(PathBuf::from("/root"));
        tree.expand_to(Path::new("/root/a/b/c.md"));
        assert!(tree.is_expanded(Path::new("/root/a")));
        assert!(tree.is_expanded(Path::new("/root/a/b")));
        assert!(!tree.is_expanded(Path::new("/root/a/b/c.md")));
    }

    #[test]
    fn expand_to_ignores_paths_outside_root() {
        let mut tree = FileTree::new(PathBuf::from("/root"));
        tree.expand_to(Path::new("/elsewhere/x/y.md"));
        assert!(!tree.is_expanded(Path::new("/elsewhere/x")));
    }

    #[test]
    fn image_paths_detected_case_insensitively() {
        for name in ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.webp", "f.svg", "g.bmp", "h.ico"] {
            assert!(is_image_path(Path::new(name)), "{name}");
        }
        for name in ["x.md", "y.rs", "z.pngx", "noext"] {
            assert!(!is_image_path(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn untitled_picks_first_free_name() {
        assert_eq!(pick_untitled(&[]), "Untitled.md");
        assert_eq!(pick_untitled(&["Untitled.md".into()]), "Untitled 2.md");
        assert_eq!(
            pick_untitled(&["Untitled.md".into(), "Untitled 2.md".into()]),
            "Untitled 3.md"
        );
    }

    /// Test-only stand-in for the brief's `children(root, dir)`: drives
    /// the real (private) `FileTree::load` and reads back what it cached,
    /// rather than exposing a new public listing function.
    fn children(root: &Path, dir: &Path) -> Vec<FsEntry> {
        let mut tree = FileTree::new(root.to_path_buf());
        tree.load(dir);
        tree.children.get(dir).cloned().unwrap_or_default()
    }

    /// A file tree should show what is on disk. Gitignored files were
    /// invisible in the sidebar, which is right for the note index and
    /// wrong for a file browser — a draft you deliberately kept out of
    /// git simply vanished.
    #[test]
    fn the_tree_shows_gitignored_files_and_marks_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "drafts/\n").unwrap();
        std::fs::create_dir(dir.path().join("drafts")).unwrap();
        std::fs::write(dir.path().join("drafts/secret.md"), "x").unwrap();
        std::fs::write(dir.path().join("visible.md"), "x").unwrap();

        let entries = children(dir.path(), dir.path());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"drafts"), "the ignored folder is listed: {names:?}");
        assert!(names.contains(&"visible.md"));

        let drafts = entries.iter().find(|e| e.name == "drafts").unwrap();
        assert!(drafts.ignored, "and is marked so the sidebar can dim it");
        let visible = entries.iter().find(|e| e.name == "visible.md").unwrap();
        assert!(!visible.ignored);
    }

    /// Build output stays collapsed. Showing `target/` or
    /// `node_modules/` would bury the actual notes.
    #[test]
    fn build_directories_are_still_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/x.md"), "x").unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules/y.md"), "x").unwrap();
        std::fs::write(dir.path().join("real.md"), "x").unwrap();

        let names: Vec<String> =
            children(dir.path(), dir.path()).into_iter().map(|e| e.name).collect();
        assert!(names.contains(&"real.md".to_string()));
        assert!(!names.contains(&"target".to_string()), "{names:?}");
        assert!(!names.contains(&"node_modules".to_string()), "{names:?}");
    }

    /// The knowledge index is unchanged: a gitignored note is still not
    /// part of the note graph.
    #[test]
    fn the_index_still_skips_gitignored_notes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "drafts/\n").unwrap();
        std::fs::create_dir(dir.path().join("drafts")).unwrap();
        std::fs::write(dir.path().join("drafts/secret.md"), "# Secret\n").unwrap();
        std::fs::write(dir.path().join("open.md"), "# Open\n").unwrap();

        let index = crate::knowledge::Index::scan(dir.path());
        let names: Vec<String> =
            index.note_names().iter().map(|(n, _)| n.clone()).collect();
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case("open")));
        assert!(
            !names.iter().any(|n| n.eq_ignore_ascii_case("secret")),
            "the index keeps excluding ignored notes: {names:?}"
        );
    }
}

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

fn walk_builder(root: &Path) -> ignore::WalkBuilder {
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

/// Canonical workspace walker: gitignore rules (even without git),
/// hidden files skipped, well-known build dirs skipped.
pub fn workspace_walk(root: &Path) -> ignore::Walk {
    walk_builder(root).build()
}

/// True if `path` (inside `root`) survives the workspace ignore rules.
/// Checks hidden/well-known components plus the root .gitignore — used
/// to drop watcher events from ignored paths.
pub fn is_visible(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    for comp in rel.components() {
        if let std::path::Component::Normal(name) = comp {
            let n = name.to_string_lossy();
            if n.starts_with('.') || IGNORED_DIRS.contains(&n.as_ref()) {
                return false;
            }
        }
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    builder.add(root.join(".gitignore"));
    let Ok(gitignore) = builder.build() else {
        return true;
    };
    let mut acc = root.to_path_buf();
    let count = rel.components().count();
    for (ix, comp) in rel.components().enumerate() {
        acc.push(comp);
        let is_dir = ix + 1 < count || acc.is_dir();
        if gitignore.matched(&acc, is_dir).is_ignore() {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
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
        let mut entries: Vec<FsEntry> = {
            let mut b = walk_builder(dir);
            b.max_depth(Some(1));
            b.build()
                .flatten()
                .filter(|e| e.path() != dir)
                .filter_map(|e| {
                    let is_dir = e.file_type()?.is_dir();
                    Some(FsEntry {
                        name: e.file_name().to_string_lossy().into_owned(),
                        path: e.into_path(),
                        is_dir,
                    })
                })
                .collect()
        };
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
}

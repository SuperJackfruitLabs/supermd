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

fn walk_builder(root: &Path) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.hidden(true)
        .git_ignore(true)
        .require_git(false)
        .git_global(false)
        .git_exclude(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_some_and(|t| t.is_dir()) && IGNORED_DIRS.contains(&name.as_ref()))
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

//! Folder-as-workspace: a lazily loaded, expandable view of the file system.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

    /// Drop cached listings so the next render re-reads the disk.
    pub fn refresh(&mut self) {
        self.children.clear();
    }

    fn load(&mut self, dir: &Path) {
        if self.children.contains_key(dir) {
            return;
        }
        let mut entries: Vec<FsEntry> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                let is_dir = entry.file_type().ok()?.is_dir();
                if is_dir && IGNORED_DIRS.contains(&name.as_str()) {
                    return None;
                }
                Some(FsEntry {
                    path: entry.path(),
                    name,
                    is_dir,
                })
            })
            .collect();
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

    /// All files under the root (for the fuzzy finder). Bounded to keep
    /// pathological folders from stalling the UI.
    pub fn all_files(&self, limit: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            if out.len() >= limit {
                break;
            }
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => {
                        if !IGNORED_DIRS.contains(&name.as_str()) {
                            stack.push(entry.path());
                        }
                    }
                    Ok(t) if t.is_file() => out.push(entry.path()),
                    _ => {}
                }
            }
        }
        out
    }
}

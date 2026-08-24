//! Read-only git access: HEAD baselines and workspace status. All
//! errors degrade to "no baseline" / empty set — never a crash, and
//! never a write to the repository.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Baseline {
    /// Blob content at HEAD for this path.
    Text(String),
    /// No repository above this file.
    NotInRepo,
    /// Repo exists, but the path is absent from the HEAD tree (also
    /// covers a fresh repo with no commits).
    Untracked,
    /// Blob exists at HEAD but is not valid UTF-8.
    Binary,
}

/// HEAD content for `path`, discovered from its parent directory.
pub fn head_text(path: &Path) -> Baseline {
    let Some(parent) = path.parent() else {
        return Baseline::NotInRepo;
    };
    let Ok(repo) = git2::Repository::discover(parent) else {
        return Baseline::NotInRepo;
    };
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return Baseline::NotInRepo; // bare repo
    };
    // macOS tempdirs sit behind /private symlinks — canonicalize both
    // sides before comparing.
    let Ok(canon) = path.canonicalize() else {
        return Baseline::Untracked;
    };
    let Ok(canon_workdir) = workdir.canonicalize() else {
        return Baseline::NotInRepo;
    };
    let Ok(rel) = canon.strip_prefix(&canon_workdir) else {
        return Baseline::NotInRepo;
    };
    let Ok(head) = repo.head() else {
        return Baseline::Untracked; // unborn HEAD
    };
    let Ok(tree) = head.peel_to_tree() else {
        return Baseline::Untracked;
    };
    let Ok(entry) = tree.get_path(rel) else {
        return Baseline::Untracked;
    };
    let Ok(obj) = entry.to_object(&repo) else {
        return Baseline::Untracked;
    };
    let Some(blob) = obj.as_blob() else {
        return Baseline::Untracked;
    };
    match std::str::from_utf8(blob.content()) {
        Ok(s) => Baseline::Text(s.to_string()),
        Err(_) => Baseline::Binary,
    }
}

/// Workspace-relative paths with uncommitted changes (modified or
/// untracked, gitignored files excluded). Empty when `root` is not in
/// a repository.
pub fn modified_paths(root: &Path) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    let Ok(repo) = git2::Repository::discover(root) else {
        return set;
    };
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return set;
    };
    let (Ok(canon_root), Ok(canon_workdir)) = (root.canonicalize(), workdir.canonicalize())
    else {
        return set;
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return set;
    };
    for entry in statuses.iter() {
        if let Some(p) = entry.path() {
            // Status paths are workdir-relative; re-relativize to the
            // workspace root, which may sit below the repo workdir.
            let abs = canon_workdir.join(p);
            if let Ok(rel) = abs.strip_prefix(&canon_root) {
                set.insert(rel.to_path_buf());
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit_all(repo: &git2::Repository) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        let parent = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parents)
            .unwrap();
    }

    fn repo_with_commit(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        commit_all(&repo);
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
        commit_all(&repo);
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

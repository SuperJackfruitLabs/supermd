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
    let Ok(repo) = gix::discover(parent) else {
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
    let Ok(commit) = repo.head_commit() else {
        return Baseline::Untracked; // unborn HEAD
    };
    let Ok(tree) = commit.tree() else {
        return Baseline::Untracked;
    };
    let Ok(Some(entry)) = tree.lookup_entry_by_path(rel) else {
        return Baseline::Untracked;
    };
    let Ok(obj) = entry.object() else {
        return Baseline::Untracked;
    };
    if obj.kind != gix::object::Kind::Blob {
        return Baseline::Untracked;
    }
    match std::str::from_utf8(&obj.data) {
        Ok(s) => Baseline::Text(s.to_string()),
        Err(_) => Baseline::Binary,
    }
}

/// Workspace-relative paths with uncommitted changes (modified or
/// untracked, gitignored files excluded). Empty when `root` is not in
/// a repository.
pub fn modified_paths(root: &Path) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    let Ok(repo) = gix::discover(root) else {
        return set;
    };
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return set;
    };
    let (Ok(canon_root), Ok(canon_workdir)) = (root.canonicalize(), workdir.canonicalize())
    else {
        return set;
    };
    let Ok(status) = repo.status(gix::progress::Discard) else {
        return set;
    };
    let Ok(iter) = status
        .untracked_files(gix::status::UntrackedFiles::Files)
        .into_iter(None)
    else {
        return set;
    };
    for item in iter.flatten() {
        let rel_path = gix::path::from_bstr(item.location()).into_owned();
        // Status paths are workdir-relative; re-relativize to the
        // workspace root, which may sit below the repo workdir.
        let abs = canon_workdir.join(rel_path);
        if let Ok(rel) = abs.strip_prefix(&canon_root) {
            set.insert(rel.to_path_buf());
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Author fixtures with the system git CLI so tests stay
    /// backend-neutral (no git2/gix API in fixture setup).
    fn sh_git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn commit_all(dir: &Path) {
        sh_git(dir, &["add", "-A"]);
        sh_git(dir, &["commit", "-qm", "commit"]);
    }

    fn repo_with_commit(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        sh_git(dir.path(), &["init", "-q"]);
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        commit_all(dir.path());
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
        sh_git(dir.path(), &["init", "-q"]);
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
        sh_git(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
        commit_all(dir.path());
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

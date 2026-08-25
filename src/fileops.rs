//! Workspace file operations: the policy layer over std::fs that the
//! sidebar drives. Every function refuses to overwrite and returns
//! plain-sentence errors fit for inline display.

use std::path::{Path, PathBuf};

/// Test seam: when set, workspace deletes go through this instead of
/// the OS trash (the CatalogFetcher pattern).
pub struct TrashFn(pub std::sync::Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>);
impl gpui::Global for TrashFn {}

/// A single path segment for a new or renamed entry.
pub fn validate_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is empty".to_string());
    }
    if name == "." || name == ".." {
        return Err(format!("{name:?} is not a valid name"));
    }
    if name.contains('/') || name.contains('\\') {
        return Err("names cannot contain slashes".to_string());
    }
    Ok(())
}

/// Refuse to clobber an existing sibling.
fn guard_free(target: &Path) -> Result<(), String> {
    if target.exists() {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Err(format!("{name:?} already exists here"))
    } else {
        Ok(())
    }
}

/// Rename within the same directory. Returns the new path.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    validate_name(new_name)?;
    let parent = path.parent().ok_or("cannot rename the root")?;
    let target = parent.join(new_name.trim());
    guard_free(&target)?;
    std::fs::rename(path, &target).map_err(|e| format!("cannot rename: {e}"))?;
    Ok(target)
}

/// Move a file or folder into `dest_dir`. Returns the new path.
pub fn move_entry(path: &Path, dest_dir: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or("cannot move the root")?;
    if dest_dir.starts_with(path) {
        return Err("a folder cannot move inside itself".to_string());
    }
    let target = dest_dir.join(name);
    guard_free(&target)?;
    std::fs::rename(path, &target).map_err(|e| format!("cannot move: {e}"))?;
    Ok(target)
}

/// Create an empty file named `name` in `dir`.
pub fn create_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let target = dir.join(name.trim());
    guard_free(&target)?;
    std::fs::write(&target, "").map_err(|e| format!("cannot create: {e}"))?;
    Ok(target)
}

/// Create a folder named `name` in `dir`.
pub fn create_dir(dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name)?;
    let target = dir.join(name.trim());
    guard_free(&target)?;
    std::fs::create_dir(&target).map_err(|e| format!("cannot create: {e}"))?;
    Ok(target)
}

/// Send a file or folder to the OS trash.
pub fn delete(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| format!("cannot delete {}: {e}", path.display()))
}

/// Map an open tab's path through a rename or move: `old → new` moves
/// the exact path and everything under it (folder renames). None when
/// unaffected.
pub fn retarget(open_path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    let rest = open_path.strip_prefix(old).ok()?;
    Some(new.join(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn names_validate_as_single_segments() {
        assert!(validate_name("notes.md").is_ok());
        assert!(validate_name(".hidden").is_ok());
        assert!(validate_name("no slash/here.md").is_err());
        assert!(validate_name("no\\backslash").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
    }

    #[test]
    fn rename_moves_within_the_directory() {
        let dir = tempdir();
        let old = dir.path().join("a.md");
        std::fs::write(&old, "hi").unwrap();
        let new = rename(&old, "b.md").unwrap();
        assert_eq!(new, dir.path().join("b.md"));
        assert!(!old.exists() && new.exists());
    }

    #[test]
    fn rename_refuses_overwrite_and_bad_names() {
        let dir = tempdir();
        let a = dir.path().join("a.md");
        std::fs::write(&a, "").unwrap();
        std::fs::write(dir.path().join("b.md"), "").unwrap();
        let err = rename(&a, "b.md").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(rename(&a, "x/y.md").is_err());
        assert!(a.exists(), "failed rename leaves the file alone");
    }

    #[test]
    fn move_entry_relocates_and_guards() {
        let dir = tempdir();
        let file = dir.path().join("a.md");
        std::fs::write(&file, "hi").unwrap();
        let dest = dir.path().join("sub");
        std::fs::create_dir(&dest).unwrap();
        let new = move_entry(&file, &dest).unwrap();
        assert_eq!(new, dest.join("a.md"));
        assert!(new.exists());

        // Overwrite refusal.
        std::fs::write(&file, "again").unwrap();
        let err = move_entry(&file, &dest).unwrap_err();
        assert!(err.contains("already exists"), "{err}");

        // A folder cannot move into its own descendant.
        let outer = dir.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        let err = move_entry(&outer, &inner).unwrap_err();
        assert!(err.contains("inside itself"), "{err}");
    }

    #[test]
    fn create_file_and_dir_refuse_overwrite() {
        let dir = tempdir();
        let f = create_file(dir.path(), "new.md").unwrap();
        assert!(f.exists());
        assert!(create_file(dir.path(), "new.md").is_err());
        let d = create_dir(dir.path(), "folder").unwrap();
        assert!(d.is_dir());
        assert!(create_dir(dir.path(), "folder").is_err());
        assert!(create_file(dir.path(), "bad/name.md").is_err());
    }

    #[test]
    fn retarget_follows_renames_of_files_and_ancestors() {
        let p = |s: &str| PathBuf::from(s);
        // Exact file.
        assert_eq!(
            retarget(&p("/w/a.md"), &p("/w/a.md"), &p("/w/b.md")),
            Some(p("/w/b.md"))
        );
        // Under a renamed folder.
        assert_eq!(
            retarget(&p("/w/docs/x/n.md"), &p("/w/docs"), &p("/w/notes")),
            Some(p("/w/notes/x/n.md"))
        );
        // Unrelated, and prefix-similar names that are not ancestors.
        assert_eq!(retarget(&p("/w/other.md"), &p("/w/a.md"), &p("/w/b.md")), None);
        assert_eq!(retarget(&p("/w/docsier/n.md"), &p("/w/docs"), &p("/w/notes")), None);
    }
}

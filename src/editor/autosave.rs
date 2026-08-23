//! Autosave policy and session backups. The policy is a pure state
//! machine driven by injected time; the fs helpers do real IO and are
//! tested against temp dirs.

use std::time::{Duration, Instant};

pub const DEBOUNCE: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct SavePolicy {
    dirty: bool,
    last_edit: Option<Instant>,
}

impl SavePolicy {
    pub fn record_edit(&mut self, now: Instant) {
        self.dirty = true;
        self.last_edit = Some(now);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn should_flush(&self, now: Instant) -> bool {
        match (self.dirty, self.last_edit) {
            (true, Some(at)) => now.duration_since(at) >= DEBOUNCE,
            _ => false,
        }
    }

    pub fn take_flush_now(&mut self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.last_edit = None;
    }
}

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub struct BackupRegistry {
    dir: PathBuf,
    seen: HashSet<PathBuf>,
    counter: u64,
}

impl BackupRegistry {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, seen: HashSet::new(), counter: 0 }
    }

    /// Default location: ~/.supermd/backups
    pub fn default_dir() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".supermd")
            .join("backups")
    }

    pub fn backup_if_needed(&mut self, source: &Path) -> io::Result<Option<PathBuf>> {
        if self.seen.contains(source) {
            return Ok(None);
        }
        let result = self.copy_backup(source)?;
        self.seen.insert(source.to_path_buf());
        Ok(result)
    }

    pub fn force_backup(&mut self, source: &Path) -> io::Result<Option<PathBuf>> {
        self.seen.insert(source.to_path_buf());
        self.copy_backup(source)
    }

    fn copy_backup(&mut self, source: &Path) -> io::Result<Option<PathBuf>> {
        if !source.exists() {
            return Ok(None);
        }
        std::fs::create_dir_all(&self.dir)?;
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.counter += 1;
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into());
        let dest = self.dir.join(format!("{stamp}-{:03}-{name}", self.counter));
        std::fs::copy(source, &dest)?;
        Ok(Some(dest))
    }
}

pub fn atomic_write(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = path.with_extension("supermd-tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

pub fn disk_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub fn has_conflict(expected: Option<SystemTime>, path: &Path) -> bool {
    match (expected, disk_mtime(path)) {
        (Some(expected), Some(actual)) => actual != expected,
        (None, Some(_)) => true, // we never saw a file, but one exists now
        (_, None) => false,      // nothing on disk => nothing to clobber
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_policy_never_flushes() {
        let policy = SavePolicy::default();
        assert!(!policy.is_dirty());
        assert!(!policy.should_flush(Instant::now()));
    }

    #[test]
    fn flushes_only_after_debounce_idle() {
        let mut policy = SavePolicy::default();
        let start = Instant::now();
        policy.record_edit(start);
        assert!(policy.is_dirty());
        assert!(!policy.should_flush(start + Duration::from_millis(500)));
        assert!(policy.should_flush(start + Duration::from_millis(1001)));
    }

    #[test]
    fn new_edit_restarts_debounce() {
        let mut policy = SavePolicy::default();
        let start = Instant::now();
        policy.record_edit(start);
        policy.record_edit(start + Duration::from_millis(900));
        assert!(!policy.should_flush(start + Duration::from_millis(1500)));
        assert!(policy.should_flush(start + Duration::from_millis(1901)));
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut policy = SavePolicy::default();
        let start = Instant::now();
        policy.record_edit(start);
        policy.mark_saved();
        assert!(!policy.is_dirty());
        assert!(!policy.should_flush(start + Duration::from_secs(10)));
    }

    #[test]
    fn take_flush_now_reports_dirty_once_meaningfully() {
        let mut policy = SavePolicy::default();
        assert!(!policy.take_flush_now());
        policy.record_edit(Instant::now());
        assert!(policy.take_flush_now());
        // still dirty until mark_saved — the caller saves, then marks
        assert!(policy.is_dirty());
        policy.mark_saved();
        assert!(!policy.take_flush_now());
    }

    use std::fs;
    use std::path::Path;

    #[test]
    fn backup_copies_original_once_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "original").unwrap();

        let mut reg = BackupRegistry::new(backups.path().to_path_buf());
        let first = reg.backup_if_needed(&file).unwrap();
        let backup_path = first.expect("first write must back up");
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "original");

        fs::write(&file, "changed").unwrap();
        assert!(reg.backup_if_needed(&file).unwrap().is_none());
        // the original backup is untouched
        assert_eq!(fs::read_to_string(&backup_path).unwrap(), "original");
    }

    #[test]
    fn backup_of_missing_file_is_none() {
        let backups = tempfile::tempdir().unwrap();
        let mut reg = BackupRegistry::new(backups.path().to_path_buf());
        assert!(reg
            .backup_if_needed(Path::new("/nonexistent/x.md"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn force_backup_copies_again() {
        let dir = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "v1").unwrap();

        let mut reg = BackupRegistry::new(backups.path().to_path_buf());
        let p1 = reg.backup_if_needed(&file).unwrap().unwrap();
        fs::write(&file, "v2-external").unwrap();
        let p2 = reg.force_backup(&file).unwrap().unwrap();
        assert_ne!(p1, p2);
        assert_eq!(fs::read_to_string(&p2).unwrap(), "v2-external");
    }

    #[test]
    fn atomic_write_replaces_contents() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.md");
        atomic_write(&file, "hello").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
        atomic_write(&file, "goodbye").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "goodbye");
        // no stray temp files left behind
        let entries: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn conflict_detection() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.md");
        fs::write(&file, "a").unwrap();
        let mtime = disk_mtime(&file);
        assert!(mtime.is_some());
        assert!(!has_conflict(mtime, &file));

        // externally modified => conflict (set mtime forward explicitly
        // to avoid flaky sub-second granularity)
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        let f = fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(later).unwrap();
        assert!(has_conflict(mtime, &file));

        // missing file, expected mtime => no conflict (nothing to clobber)
        assert!(!has_conflict(mtime, Path::new("/nonexistent/y.md")));
    }
}

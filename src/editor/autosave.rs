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

/// Backups older than this are pruned at session start.
const GC_MAX_AGE_SECS: u64 = 30 * 86_400;
/// At most this many backup files are kept.
const GC_MAX_FILES: usize = 200;

/// Which backup files to delete: everything older than `max_age`,
/// then the oldest beyond `max_files`. Entries are (name, stamp).
pub fn plan_gc(
    entries: &[(String, u64)],
    now: u64,
    max_files: usize,
    max_age: u64,
) -> Vec<String> {
    let mut doomed: Vec<String> = Vec::new();
    let mut survivors: Vec<(String, u64)> = Vec::new();
    for (name, stamp) in entries {
        if now.saturating_sub(*stamp) > max_age {
            doomed.push(name.clone());
        } else {
            survivors.push((name.clone(), *stamp));
        }
    }
    if survivors.len() > max_files {
        survivors.sort_by_key(|(_, stamp)| *stamp); // oldest first
        for (name, _) in survivors.iter().take(survivors.len() - max_files) {
            doomed.push(name.clone());
        }
    }
    doomed
}

impl BackupRegistry {
    pub fn new(dir: PathBuf) -> Self {
        let registry = Self { dir, seen: HashSet::new(), counter: 0 };
        registry.gc();
        registry
    }

    /// Prune old backups once per session; failures are ignored (a GC
    /// problem must never affect saving).
    fn gc(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let listed: Vec<(String, u64)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                // Backup names start with the unix stamp; fall back to
                // the file's mtime for anything else in the directory.
                let stamp = name
                    .split('-')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| {
                        e.metadata()
                            .ok()?
                            .modified()
                            .ok()?
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .ok()
                            .map(|d| d.as_secs())
                    })?;
                Some((name, stamp))
            })
            .collect();
        for name in plan_gc(&listed, now, GC_MAX_FILES, GC_MAX_AGE_SECS) {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
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
        // Git already preserves committed content: when the file is
        // byte-identical to HEAD, a backup adds nothing.
        if let crate::git::Baseline::Text(head) = crate::git::head_text(source) {
            if std::fs::read_to_string(source).map(|now| now == head).unwrap_or(false) {
                return Ok(None);
            }
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

/// Reload-from-disk policy: only clean buffers whose file actually
/// changed reload; dirty buffers always keep the user's edits.
pub fn should_reload(dirty: bool, mtime_changed: bool) -> bool {
    !dirty && mtime_changed
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
    fn reload_only_when_clean_and_changed() {
        assert!(should_reload(false, true));
        assert!(!should_reload(true, true)); // dirty buffers keep user edits
        assert!(!should_reload(false, false));
        assert!(!should_reload(true, false));
    }

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
    fn default_dir_is_under_home() {
        let dir = BackupRegistry::default_dir();
        assert!(
            dir.ends_with(Path::new(".supermd").join("backups")),
            "unexpected default dir: {dir:?}"
        );
    }

    #[test]
    fn unexpected_file_on_disk_is_a_conflict() {
        // We never observed a file, but one exists now: saving would
        // clobber it, so that's a conflict.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.md");
        fs::write(&file, "surprise").unwrap();
        assert!(has_conflict(None, &file));
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
fn sh_git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    #[test]
    fn backup_skipped_when_git_head_has_identical_content() {
        let repo = tempfile::tempdir().unwrap();
        sh_git(repo.path(), &["init", "-q"]);
        let file = repo.path().join("notes.md");
        std::fs::write(&file, "committed\n").unwrap();
        sh_git(repo.path(), &["add", "-A"]);
        sh_git(repo.path(), &["commit", "-qm", "c"]);

        let backups = tempfile::tempdir().unwrap();
        let mut registry = BackupRegistry::new(backups.path().to_path_buf());
        // Content matches HEAD: git already preserves it, no copy.
        assert!(registry.backup_if_needed(&file).unwrap().is_none());
        assert_eq!(std::fs::read_dir(backups.path()).unwrap().count(), 0);

        // Dirty vs HEAD: a real backup is made (a fresh registry, since
        // backup_if_needed is once-per-session per file).
        std::fs::write(&file, "edited\n").unwrap();
        let mut registry = BackupRegistry::new(backups.path().to_path_buf());
        assert!(registry.backup_if_needed(&file).unwrap().is_some());
        assert_eq!(std::fs::read_dir(backups.path()).unwrap().count(), 1);
    }

    #[test]
    fn backup_still_made_outside_repos_and_for_untracked_files() {
        let plain = tempfile::tempdir().unwrap();
        let file = plain.path().join("free.md");
        std::fs::write(&file, "text\n").unwrap();
        let backups = tempfile::tempdir().unwrap();
        let mut registry = BackupRegistry::new(backups.path().to_path_buf());
        assert!(registry.backup_if_needed(&file).unwrap().is_some());
    }

    #[test]
    fn gc_plan_prunes_by_age_and_count() {
        let now = 100_000_000u64;
        let day = 86_400u64;
        // (name, stamp): two ancient, three recent
        let entries = vec![
            ("old-a".to_string(), now - 40 * day),
            ("old-b".to_string(), now - 31 * day),
            ("new-a".to_string(), now - day),
            ("new-b".to_string(), now - 2 * day),
            ("new-c".to_string(), now - 3 * day),
        ];
        // age rule alone
        let doomed = plan_gc(&entries, now, 10, 30 * day);
        assert_eq!(doomed, vec!["old-a".to_string(), "old-b".to_string()]);
        // count rule: keep the 2 newest of the survivors
        let doomed = plan_gc(&entries, now, 2, 30 * day);
        assert!(doomed.contains(&"old-a".to_string()) && doomed.contains(&"old-b".to_string()));
        assert!(doomed.contains(&"new-c".to_string()), "oldest survivor beyond cap");
        assert_eq!(doomed.len(), 3);
    }

    #[test]
    fn gc_runs_at_registry_creation() {
        let backups = tempfile::tempdir().unwrap();
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let old = now - 60 * 86_400;
        std::fs::write(backups.path().join(format!("{old}-001-ancient.md")), "x").unwrap();
        std::fs::write(backups.path().join(format!("{now}-002-fresh.md")), "y").unwrap();
        let _registry = BackupRegistry::new(backups.path().to_path_buf());
        let names: Vec<String> = std::fs::read_dir(backups.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].contains("fresh"));
    }
}

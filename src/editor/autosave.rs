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
}

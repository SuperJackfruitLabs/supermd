//! Project-wide text search: literal smart-case matching streamed over
//! the ignore-aware workspace walk. Pure logic — the overlay drives it
//! from the background executor.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use grep_matcher::Matcher as _;
use grep_searcher::{Searcher, Sink, SinkMatch};

pub const SEARCH_CAP: usize = 500;
const MAX_LINE_BYTES: usize = 240;

#[derive(Clone, Debug)]
pub struct SearchMatch {
    /// Workspace-relative path.
    pub path: PathBuf,
    /// 1-based.
    pub line_number: u64,
    /// The matched line, trailing newline stripped, truncated to
    /// `MAX_LINE_BYTES` on a char boundary.
    pub line_text: String,
    /// Hit byte ranges within `line_text` (clipped to the truncation).
    pub ranges: Vec<Range<usize>>,
}

struct FileSink<'a> {
    matcher: &'a grep_regex::RegexMatcher,
    rel: &'a Path,
    out: &'a mut Vec<SearchMatch>,
    total_before: usize,
    cancelled: &'a AtomicBool,
}

impl Sink for FileSink<'_> {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if self.cancelled.load(Ordering::Relaxed)
            || self.total_before + self.out.len() >= SEARCH_CAP
        {
            return Ok(false);
        }
        let raw = String::from_utf8_lossy(mat.bytes());
        let line = raw.trim_end_matches(['\n', '\r']);
        let mut ranges = Vec::new();
        self.matcher
            .find_iter(line.as_bytes(), |m| {
                ranges.push(m.start()..m.end());
                true
            })
            .ok();
        let mut text = line.to_string();
        if text.len() > MAX_LINE_BYTES {
            let mut cut = MAX_LINE_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
            ranges.retain(|r| r.end <= cut);
        }
        self.out.push(SearchMatch {
            path: self.rel.to_path_buf(),
            line_number: mat.line_number().unwrap_or(0),
            line_text: text,
            ranges,
        });
        Ok(true)
    }
}

/// Blocking streaming search over the ignore-aware workspace walk.
/// Sends one batch per file with hits; returns true if the result set
/// was capped at `SEARCH_CAP`.
pub fn search_workspace(
    root: &Path,
    query: &str,
    cancelled: &AtomicBool,
    tx: Sender<Vec<SearchMatch>>,
) -> bool {
    if query.is_empty() {
        return false;
    }
    let no_upper = !query.chars().any(|c| c.is_uppercase());
    let Ok(matcher) = grep_regex::RegexMatcherBuilder::new()
        .fixed_strings(true)
        .case_insensitive(no_upper)
        .line_terminator(Some(b'\n'))
        .build(query)
    else {
        return false;
    };
    let mut searcher = grep_searcher::SearcherBuilder::new()
        .line_number(true)
        .build();
    let mut total = 0usize;
    for entry in crate::files::workspace_walk(root).flatten() {
        if cancelled.load(Ordering::Relaxed) || total >= SEARCH_CAP {
            break;
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);
        let mut hits = Vec::new();
        let sink = FileSink {
            matcher: &matcher,
            rel,
            out: &mut hits,
            total_before: total,
            cancelled,
        };
        searcher.search_path(&matcher, path, sink).ok();
        if !hits.is_empty() {
            total += hits.len();
            if tx.send(hits).is_err() {
                break;
            }
        }
    }
    total >= SEARCH_CAP
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::channel;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "Alpha beta\ngamma ALPHA\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "let alpha = 1;\n").unwrap();
        std::fs::create_dir(dir.path().join("skip")).unwrap();
        std::fs::write(dir.path().join("skip/c.md"), "alpha\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "skip/\n").unwrap();
        dir
    }

    #[test]
    fn smart_case_lowercase_matches_all_cases() {
        let dir = fixture();
        let (tx, rx) = channel();
        let cancelled = AtomicBool::new(false);
        search_workspace(dir.path(), "alpha", &cancelled, tx);
        let all: Vec<SearchMatch> = rx.iter().flatten().collect();
        assert_eq!(all.len(), 3, "{all:?}");
        assert!(all.iter().all(|m| !m.path.starts_with("skip")), "{all:?}");
        assert!(all.iter().all(|m| !m.ranges.is_empty()));
        assert!(all.iter().all(|m| m.path.is_relative()));
    }

    #[test]
    fn smart_case_uppercase_is_exact() {
        let dir = fixture();
        let (tx, rx) = channel();
        let cancelled = AtomicBool::new(false);
        search_workspace(dir.path(), "ALPHA", &cancelled, tx);
        let all: Vec<SearchMatch> = rx.iter().flatten().collect();
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!(all[0].line_number, 2);
        let m = &all[0];
        assert_eq!(&m.line_text[m.ranges[0].clone()], "ALPHA");
    }

    #[test]
    fn cancellation_stops_stream() {
        let dir = fixture();
        let (tx, rx) = channel();
        let cancelled = AtomicBool::new(true);
        search_workspace(dir.path(), "alpha", &cancelled, tx);
        assert_eq!(rx.iter().flatten().count(), 0);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let dir = fixture();
        let (tx, rx) = channel();
        let c = AtomicBool::new(false);
        assert!(!search_workspace(dir.path(), "", &c, tx));
        assert_eq!(rx.iter().count(), 0);
    }

    #[test]
    fn overlong_lines_truncate_on_char_boundary() {
        let dir = tempfile::tempdir().unwrap();
        // 6 + 233 = 239 bytes, then a 2-byte char straddling the
        // 240-byte cut, then a second match beyond the cut.
        let line = format!("needle{}é{}needle\n", "a".repeat(233), "b".repeat(20));
        std::fs::write(dir.path().join("long.txt"), &line).unwrap();
        let (tx, rx) = channel();
        let c = AtomicBool::new(false);
        search_workspace(dir.path(), "needle", &c, tx);
        let all: Vec<SearchMatch> = rx.iter().flatten().collect();
        assert_eq!(all.len(), 1, "{all:?}");
        let m = &all[0];
        assert!(m.line_text.ends_with('…'), "{:?}", m.line_text);
        assert!(m.line_text.len() <= MAX_LINE_BYTES + '…'.len_utf8());
        assert!(m.line_text.is_char_boundary(m.line_text.len() - '…'.len_utf8()));
        // The hit past the truncation point is dropped; the leading one stays.
        assert_eq!(m.ranges, vec![0..6]);
    }

    #[test]
    fn query_with_line_terminator_is_rejected() {
        let dir = fixture();
        let (tx, rx) = channel();
        let c = AtomicBool::new(false);
        assert!(!search_workspace(dir.path(), "alpha\nbeta", &c, tx));
        assert_eq!(rx.iter().count(), 0);
    }

    #[test]
    fn closed_receiver_stops_search_without_panic() {
        let dir = fixture();
        let (tx, rx) = channel();
        drop(rx);
        let c = AtomicBool::new(false);
        assert!(!search_workspace(dir.path(), "alpha", &c, tx));
    }

    #[test]
    fn cap_stops_early_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let body = "hit\n".repeat(SEARCH_CAP + 50);
        std::fs::write(dir.path().join("big.txt"), body).unwrap();
        let (tx, rx) = channel();
        let c = AtomicBool::new(false);
        assert!(search_workspace(dir.path(), "hit", &c, tx), "capped");
        assert_eq!(rx.iter().flatten().count(), SEARCH_CAP);
    }
}

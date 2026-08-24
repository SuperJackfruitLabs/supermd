//! Pure diff engine: merges old and new text into one document with a
//! change map. No git, no GPUI. Invariants: stripping Deleted ranges
//! from the merged text reproduces the new text; stripping Added
//! ranges reproduces the old text.

use std::ops::Range;

pub const MAX_DIFF_BYTES: usize = crate::editor::spans::MAX_STYLED_BYTES;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind {
    Added,
    Deleted,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// Byte range into `DiffDoc::text` (the merged document).
    pub range: Range<usize>,
    pub kind: ChangeKind,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DiffDoc {
    /// The new text with deleted runs spliced back in at their
    /// original positions.
    pub text: String,
    /// Non-overlapping, sorted by start. Empty == no changes.
    pub changes: Vec<Change>,
}

pub fn diff_doc(old: &str, new: &str) -> DiffDoc {
    let refine = old.len() + new.len() <= MAX_DIFF_BYTES;
    let mut doc = DiffDoc::default();
    let diff = similar::TextDiff::from_lines(old, new);
    for op in diff.ops() {
        if refine && op.tag() == similar::DiffTag::Replace {
            let del: String = diff
                .iter_changes(op)
                .filter(|c| c.tag() == similar::ChangeTag::Delete)
                .map(|c| c.value())
                .collect();
            let ins: String = diff
                .iter_changes(op)
                .filter(|c| c.tag() == similar::ChangeTag::Insert)
                .map(|c| c.value())
                .collect();
            emit_word_diff(&mut doc, &del, &ins);
            continue;
        }
        for change in diff.iter_changes(op) {
            let start = doc.text.len();
            doc.text.push_str(change.value());
            let kind = match change.tag() {
                similar::ChangeTag::Equal => continue,
                similar::ChangeTag::Insert => ChangeKind::Added,
                similar::ChangeTag::Delete => ChangeKind::Deleted,
            };
            doc.changes.push(Change { range: start..doc.text.len(), kind });
        }
    }
    coalesce(&mut doc.changes);
    doc
}

/// Word-granular diff of a replaced run: unchanged words appear once,
/// changed words as deleted-then-added pairs.
fn emit_word_diff(doc: &mut DiffDoc, del: &str, ins: &str) {
    let wd = similar::TextDiff::from_unicode_words(del, ins);
    for op in wd.ops() {
        for change in wd.iter_changes(op) {
            let start = doc.text.len();
            doc.text.push_str(change.value());
            let kind = match change.tag() {
                similar::ChangeTag::Equal => continue,
                similar::ChangeTag::Insert => ChangeKind::Added,
                similar::ChangeTag::Delete => ChangeKind::Deleted,
            };
            doc.changes.push(Change { range: start..doc.text.len(), kind });
        }
    }
}

/// Merge adjacent same-kind changes so the map stays minimal.
fn coalesce(changes: &mut Vec<Change>) {
    let mut out: Vec<Change> = Vec::with_capacity(changes.len());
    for c in changes.drain(..) {
        match out.last_mut() {
            Some(last) if last.kind == c.kind && last.range.end == c.range.start => {
                last.range.end = c.range.end;
            }
            _ => out.push(c),
        }
    }
    *changes = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_docs_have_no_changes() {
        let d = diff_doc("a\nb\n", "a\nb\n");
        assert_eq!(d.text, "a\nb\n");
        assert!(d.changes.is_empty());
    }

    #[test]
    fn pure_insertion_marks_added() {
        let d = diff_doc("a\nc\n", "a\nb\nc\n");
        assert_eq!(d.text, "a\nb\nc\n");
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Added);
        assert_eq!(&d.text[d.changes[0].range.clone()], "b\n");
    }

    #[test]
    fn pure_deletion_splices_deleted_run() {
        let d = diff_doc("a\nb\nc\n", "a\nc\n");
        assert_eq!(d.text, "a\nb\nc\n");
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Deleted);
        assert_eq!(&d.text[d.changes[0].range.clone()], "b\n");
    }

    #[test]
    fn changes_sorted_and_non_overlapping() {
        let d = diff_doc("a\nX\nc\nY\ne\n", "a\nb\nc\nd\ne\n");
        let mut last = 0;
        for c in &d.changes {
            assert!(c.range.start >= last, "sorted, non-overlapping");
            assert!(d.text.is_char_boundary(c.range.start));
            assert!(d.text.is_char_boundary(c.range.end));
            last = c.range.end;
        }
        assert!(!d.changes.is_empty());
    }

    fn assert_reconstruction(old: &str, new: &str) {
        let d = diff_doc(old, new);
        let strip = |kind: ChangeKind| {
            let mut out = String::new();
            let mut pos = 0;
            for c in &d.changes {
                if c.kind == kind {
                    out.push_str(&d.text[pos..c.range.start]);
                    pos = c.range.end;
                }
            }
            out.push_str(&d.text[pos..]);
            out
        };
        assert_eq!(strip(ChangeKind::Deleted), new, "strip deleted == new for {old:?} -> {new:?}");
        assert_eq!(strip(ChangeKind::Added), old, "strip added == old for {old:?} -> {new:?}");
    }

    #[test]
    fn one_word_edit_marks_one_word() {
        let d = diff_doc("the quick brown fox\n", "the swift brown fox\n");
        let deleted: Vec<&str> = d
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Deleted)
            .map(|c| &d.text[c.range.clone()])
            .collect();
        let added: Vec<&str> = d
            .changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Added)
            .map(|c| &d.text[c.range.clone()])
            .collect();
        assert_eq!(deleted, vec!["quick"]);
        assert_eq!(added, vec!["swift"]);
    }

    #[test]
    fn word_refined_doc_keeps_invariants() {
        assert_reconstruction("the quick brown fox\n", "the swift brown fox\n");
        assert_reconstruction("alpha beta\ngamma delta\n", "alpha b\ngamma delta epsilon\n");
        assert_reconstruction("héllo wörld\n", "héllo mönde\n");
    }

    #[test]
    fn oversized_input_skips_word_refinement() {
        let old = "a ".repeat(600_000);
        let new = old.replacen("a ", "b ", 1);
        let d = diff_doc(&old, &new);
        assert!(!d.changes.is_empty());
    }

    #[test]
    fn reconstruction_invariants_hold() {
        for (old, new) in [
            ("", ""),
            ("a\n", ""),
            ("", "a\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("a\nc\n", "a\nb\nc\n"),
            ("x\ny\n", "p\nq\n"),
            ("one two three\n", "one 2 three\n"),
            ("no trailing newline", "still no newline"),
        ] {
            assert_reconstruction(old, new);
        }
    }
}

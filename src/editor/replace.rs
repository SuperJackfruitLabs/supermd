//! Replace, as one contiguous edit. Pure; the shell applies the result.

use std::ops::Range;

/// One contiguous replacement, plus what to select afterwards.
#[derive(Debug, PartialEq, Eq)]
pub struct ReplaceEdit {
    /// Bytes of the document to replace.
    pub range: Range<usize>,
    pub replacement: String,
    /// Selection after the edit, in post-edit coordinates.
    pub select: Range<usize>,
    /// How many matches were rewritten.
    pub count: usize,
}

/// Rewrite every match as a single edit spanning first match to last.
///
/// One edit rather than one per match: ⌘Z has to take back the whole
/// Replace All, and stepping back through a hundred replacements is not
/// undo. The span is trimmed to the affected region so untouched text
/// stays outside the undo entry.
///
/// The replacement is never re-scanned, so replacing `a` with `aa`
/// terminates.
pub fn replace_all(text: &str, query: &str, with: &str) -> Option<ReplaceEdit> {
    let matches = crate::editor::find::find_matches(text, query);
    let first = matches.first()?.start;
    let last = matches.last()?.end;

    let mut out = String::with_capacity(last - first);
    let mut at = first;
    for m in &matches {
        out.push_str(&text[at..m.start]);
        out.push_str(with);
        at = m.end;
    }
    out.push_str(&text[at..last]);

    Some(ReplaceEdit {
        range: first..last,
        select: first..first + out.len(),
        replacement: out,
        count: matches.len(),
    })
}

/// Rewrite one already-located match.
pub fn replace_one(_text: &str, at: Range<usize>, with: &str) -> ReplaceEdit {
    ReplaceEdit {
        range: at.clone(),
        select: at.start..at.start + with.len(),
        replacement: with.to_string(),
        count: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_all_rewrites_every_match_in_one_edit() {
        let text = "one cat, two cat, red cat";
        let e = replace_all(text, "cat", "dog").expect("matches");
        assert_eq!(e.count, 3);
        let mut out = text.to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "one dog, two dog, red dog");
    }

    /// One edit, not three: undoing a Replace All one match at a time
    /// is not undo.
    #[test]
    fn replace_all_spans_only_the_affected_region() {
        let text = "keep this: cat and cat :keep that";
        let e = replace_all(text, "cat", "x").expect("matches");
        assert_eq!(&text[..e.range.start], "keep this: ", "untouched head is outside the edit");
        assert!(text[e.range.end..].starts_with(" :keep"), "untouched tail too");
    }

    #[test]
    fn replacing_with_a_longer_or_shorter_string_keeps_the_rest_intact() {
        for (with, expect) in [("elephant", "a elephant b elephant c"), ("", "a  b  c")] {
            let text = "a cat b cat c";
            let e = replace_all(text, "cat", with).expect("matches");
            let mut out = text.to_string();
            out.replace_range(e.range.clone(), &e.replacement);
            assert_eq!(out, expect);
        }
    }

    /// The replacement must not be re-scanned, or replacing "a" with
    /// "aa" never terminates.
    #[test]
    fn a_replacement_containing_the_query_terminates() {
        let e = replace_all("a a a", "a", "aa").expect("matches");
        assert_eq!(e.count, 3);
        let mut out = "a a a".to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "aa aa aa");
    }

    #[test]
    fn no_match_is_no_edit() {
        assert!(replace_all("hello", "xyz", "1").is_none());
        assert!(replace_all("hello", "", "1").is_none(), "an empty query matches nothing");
    }

    /// Multi-byte text: ranges must stay on char boundaries.
    #[test]
    fn replace_all_handles_multibyte_text() {
        let text = "café cat café";
        let e = replace_all(text, "cat", "chien").expect("matches");
        let mut out = text.to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "café chien café");
    }

    #[test]
    fn replace_one_touches_a_single_match_and_selects_it() {
        let text = "a cat b cat";
        let at = 2..5;
        let e = replace_one(text, at, "dog");
        assert_eq!(e.count, 1);
        let mut out = text.to_string();
        out.replace_range(e.range.clone(), &e.replacement);
        assert_eq!(out, "a dog b cat");
        assert_eq!(&out[e.select.clone()], "dog", "the new text is selected");
    }
}

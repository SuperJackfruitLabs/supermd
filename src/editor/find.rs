//! Find-in-file matching. ASCII case folding only (documented); smart
//! case: a query containing an uppercase character matches exactly.

use std::ops::Range;

/// Non-overlapping match ranges of `query` in `text`, sorted.
pub fn find_matches(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() || query.len() > text.len() {
        return Vec::new();
    }
    let case_sensitive = query.chars().any(|c| c.is_uppercase());
    let mut out = Vec::new();
    let mut i = 0;
    while i + query.len() <= text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let end = i + query.len();
        if !text.is_char_boundary(end) {
            i += 1;
            continue;
        }
        let window = &text[i..end];
        let hit = if case_sensitive {
            window == query
        } else {
            window.eq_ignore_ascii_case(query)
        };
        if hit {
            out.push(i..end);
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_by_default() {
        assert_eq!(find_matches("Foo foo FOO", "foo"), vec![0..3, 4..7, 8..11]);
    }

    #[test]
    fn smart_case_when_query_has_uppercase() {
        assert_eq!(find_matches("Foo foo FOO", "Foo"), vec![0..3]);
    }

    #[test]
    fn non_overlapping_and_empty_query() {
        assert_eq!(find_matches("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(find_matches("anything", "").is_empty());
    }

    #[test]
    fn unicode_boundaries_are_respected() {
        assert_eq!(find_matches("héllo héllo", "llo"), vec![3..6, 10..13]);
    }
}

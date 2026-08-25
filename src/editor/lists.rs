//! List-item analysis behind Enter-continuation and Tab indenting.
//! Pure line inspection — the editor decides what edit to make.

/// A recognized list marker at the start of a line.
#[derive(Debug, PartialEq, Eq)]
pub struct ListItem {
    /// Leading whitespace bytes before the marker.
    pub indent: usize,
    /// Marker bytes after the indent, including its trailing space
    /// (and the `[ ] ` box on task items).
    pub marker_len: usize,
    /// Nothing but the marker on this line.
    pub content_empty: bool,
    /// Marker a continuation line should carry (numbers increment,
    /// task boxes reset to unchecked).
    pub next_marker: String,
    /// Spaces one Tab press adds or removes for this item.
    pub indent_step: usize,
}

/// Parse `line` (without its newline) as a list item.
pub fn list_item(line: &str) -> Option<ListItem> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];

    let (mut marker_len, next_marker, indent_step) = if let Some(after) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        let bullet = &rest[..2];
        // Task box: `- [ ] ` / `- [x] ` counts as part of the marker.
        let task = ["[ ] ", "[x] ", "[X] "]
            .iter()
            .find(|b| after.starts_with(**b))
            .map(|b| b.len())
            .unwrap_or(0);
        let next = if task > 0 {
            format!("{bullet}[ ] ")
        } else {
            bullet.to_string()
        };
        (2 + task, next, 2)
    } else {
        let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let delim = rest.as_bytes().get(digits).copied();
        if !matches!(delim, Some(b'.') | Some(b')'))
            || rest.as_bytes().get(digits + 1) != Some(&b' ')
        {
            return None;
        }
        let n: u64 = rest[..digits].parse().ok()?;
        let next = format!("{}{} ", n + 1, char::from(delim.unwrap()));
        (digits + 2, next, digits + 2)
    };

    // A bare marker followed only by whitespace is an empty item.
    let content = &rest[marker_len..];
    if !content.is_empty() && content.chars().all(|c| c == ' ' || c == '\t') {
        marker_len += content.len();
    }
    let content_empty = rest.len() == marker_len;

    Some(ListItem { indent, marker_len, content_empty, next_marker, indent_step })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(line: &str) -> ListItem {
        list_item(line).unwrap_or_else(|| panic!("{line:?} should parse"))
    }

    #[test]
    fn bullets_continue_with_the_same_marker() {
        assert_eq!(item("- milk").next_marker, "- ");
        assert_eq!(item("* star").next_marker, "* ");
        assert_eq!(item("+ plus").next_marker, "+ ");
    }

    #[test]
    fn numbers_increment_and_keep_their_delimiter() {
        assert_eq!(item("3. three").next_marker, "4. ");
        assert_eq!(item("9) nine").next_marker, "10) ");
        assert_eq!(item("1. one").marker_len, 3);
    }

    #[test]
    fn tasks_continue_unchecked() {
        assert_eq!(item("- [x] done").next_marker, "- [ ] ");
        assert_eq!(item("- [ ] todo").next_marker, "- [ ] ");
        assert_eq!(item("- [X] loud").next_marker, "- [ ] ");
        assert_eq!(item("- [ ] todo").marker_len, 6);
    }

    #[test]
    fn indent_is_measured_not_consumed() {
        let it = item("   - deep");
        assert_eq!(it.indent, 3);
        assert_eq!(it.marker_len, 2);
    }

    #[test]
    fn empty_items_are_flagged() {
        assert!(item("- ").content_empty);
        assert!(item("  3. ").content_empty);
        assert!(item("- [ ] ").content_empty);
        assert!(!item("- x").content_empty);
        // Trailing spaces after a bare marker still count as empty.
        assert!(item("-   ").content_empty);
    }

    #[test]
    fn indent_step_matches_commonmark_nesting() {
        assert_eq!(item("- b").indent_step, 2);
        assert_eq!(item("- [ ] t").indent_step, 2, "the box is content");
        assert_eq!(item("1. a").indent_step, 3);
        assert_eq!(item("10. a").indent_step, 4);
    }

    #[test]
    fn non_lists_do_not_parse() {
        for line in ["hello", "-nospace", "1.nospace", "12345678901. huge", "", "  ", "> quote"] {
            assert_eq!(list_item(line), None, "{line:?}");
        }
    }
}

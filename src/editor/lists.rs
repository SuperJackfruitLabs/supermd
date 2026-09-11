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

/// Rewrite the ordered-list numbers within `block` (a byte range of
/// `text`) so each indent level counts sequentially, keeping the first
/// number an author chose at each level. Unordered items and lines
/// that aren't list items are left untouched. `None` means the block
/// holds no ordered item at all — nothing to renumber.
///
/// Returns the whole of `text`, unchanged outside `block`.
pub fn renumber(text: &str, block: std::ops::Range<usize>) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..block.start]);

    // Stack of (indent, current number) for the ordered runs in play.
    // A shallower or equal indent pops deeper entries (their scope
    // ended); an equal indent continues counting; a deeper indent
    // starts a fresh run at the author's own first number.
    let mut stack: Vec<(usize, u64)> = Vec::new();
    let mut found_any = false;

    let mut lines = text[block.clone()].split('\n').peekable();
    while let Some(line) = lines.next() {
        let is_last = lines.peek().is_none();
        let Some(item) = list_item(line) else {
            out.push_str(line);
            if !is_last {
                out.push('\n');
            }
            continue;
        };
        let rest = &line[item.indent..];
        let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            // A bullet: leave the line as-is.
            out.push_str(line);
            if !is_last {
                out.push('\n');
            }
            continue;
        }
        found_any = true;
        while stack.last().is_some_and(|(indent, _)| *indent > item.indent) {
            stack.pop();
        }
        let n = match stack.last_mut() {
            Some((indent, n)) if *indent == item.indent => {
                *n += 1;
                *n
            }
            _ => {
                let start: u64 = rest[..digits].parse().unwrap_or(1);
                stack.push((item.indent, start));
                start
            }
        };
        out.push_str(&line[..item.indent]);
        out.push_str(&n.to_string());
        out.push_str(&rest[digits..]);
        if !is_last {
            out.push('\n');
        }
    }

    out.push_str(&text[block.end..]);
    if found_any {
        Some(out)
    } else {
        None
    }
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

    /// A list edited in the middle reads "1. 2. 3. 3. 4." Renumbering
    /// rewrites the run, keeping the first number the author chose.
    #[test]
    fn renumber_fixes_a_run_after_an_insertion() {
        let text = "1. one\n2. two\n2. inserted\n3. three\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(out, "1. one\n2. two\n3. inserted\n4. three\n");
    }

    /// A list that starts at 5 keeps starting at 5.
    #[test]
    fn renumber_keeps_the_starting_number() {
        let text = "5. five\n5. six\n5. seven\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(out, "5. five\n6. six\n7. seven\n");
    }

    /// Nested items renumber within their own level.
    #[test]
    fn renumber_treats_each_indent_level_separately() {
        let text = "1. a\n   1. x\n   1. y\n2. b\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(out, "1. a\n   1. x\n   2. y\n2. b\n");
    }

    /// Bullets are left alone.
    #[test]
    fn renumber_ignores_an_unordered_list() {
        assert!(renumber("- a\n- b\n", 0..6).is_none());
    }
}

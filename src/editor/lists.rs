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

/// Byte ranges of the *bodies* of the fenced code blocks in `text` --
/// the lines between the delimiters, delimiters excluded. Numbers in
/// there are the user's literal text, not a list: renumbering one
/// would silently rewrite a file the user never edited.
///
/// The delimiter lines themselves stay ordinary non-list lines, so a
/// fence still ends an ordered run at its own indent or shallower
/// exactly as any other paragraph line does.
fn fence_bodies(text: &str) -> Vec<std::ops::Range<usize>> {
    super::blocks::blocks(text)
        .into_iter()
        .filter_map(|b| match b.kind {
            super::blocks::BlockKind::Fence { open_line, close_line } => {
                // An unclosed fence runs to the end of its block.
                let end = close_line.map_or(b.range.end + 1, |c| c.start);
                Some(open_line.end..end)
            }
            _ => None,
        })
        .collect()
}

/// Rewrite the ordered-list numbers within `block` (a byte range of
/// `text`) so each indent level counts sequentially, keeping the first
/// number an author chose at each level. Unordered items, lines inside
/// a fenced code block, and lines that aren't list items are left
/// untouched. `None` means the block holds no ordered item at all --
/// nothing to renumber.
///
/// Returns the whole of `text`, unchanged outside `block`.
pub fn renumber(text: &str, block: std::ops::Range<usize>) -> Option<String> {
    let new_block = renumber_block(text, block.clone())?;
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..block.start]);
    out.push_str(&new_block);
    out.push_str(&text[block.end..]);
    Some(out)
}

/// The renumbered replacement for `block` alone -- what `renumber`
/// splices back in. The editor replaces just this range, so one Enter
/// costs one small undo entry instead of two copies of the document.
pub fn renumber_block(text: &str, block: std::ops::Range<usize>) -> Option<String> {
    let mut out = String::with_capacity(block.end - block.start);
    let fences = fence_bodies(text);

    // Stack of (indent, current number) for the ordered runs in play.
    // A shallower or equal indent pops deeper entries (their scope
    // ended); an equal indent continues counting; a deeper indent
    // starts a fresh run at the author's own first number.
    let mut stack: Vec<(usize, u64)> = Vec::new();
    let mut found_any = false;

    let mut line_start = block.start;
    let mut lines = text[block.clone()].split('\n').peekable();
    while let Some(line) = lines.next() {
        let is_last = lines.peek().is_none();
        let at = line_start;
        line_start += line.len() + 1;
        if fences.iter().any(|f| f.contains(&at)) {
            // Inside a fence: copy the line through untouched, and
            // leave the surrounding run's count alone -- the list
            // continues either side of the code block.
            out.push_str(line);
            if !is_last {
                out.push('\n');
            }
            continue;
        }
        let Some(item) = list_item(line) else {
            // A blank line ends every currently open run. A non-blank,
            // non-list line ends any run at its own indent or shallower
            // (a separate paragraph, not a continuation of a deeper
            // item) — so the next ordered item at that level restarts
            // at the author's own number instead of continuing a stale
            // count from an unrelated, textually earlier list.
            if line.trim().is_empty() {
                stack.clear();
            } else {
                let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                while stack.last().is_some_and(|(ind, _)| *ind >= indent) {
                    stack.pop();
                }
            }
            out.push_str(line);
            if !is_last {
                out.push('\n');
            }
            continue;
        };
        let rest = &line[item.indent..];
        let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            // A bullet: leave the line as-is. It doesn't reset an
            // ordered run — a bullet can appear inline within one
            // (e.g. as a nested sub-item) without ending it.
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

    /// Two textually separate ordered lists (a blank-line-and-paragraph
    /// gap between them) keep their own counts — the second list's
    /// deliberate restart at 1 is not a typo to fix.
    #[test]
    fn renumber_restarts_a_separate_list_after_a_paragraph() {
        let text = "1. one\n2. two\n\npara\n\n1. three\n2. four\n";
        let out = renumber(text, 0..text.len());
        // Already correctly numbered, so unchanged (still Some: there
        // are ordered items to renumber, it's just a no-op on them).
        assert_eq!(out.as_deref(), Some(text), "both lists keep restarting correctly");
    }

    /// Numbers inside a fenced code block are the user's literal text,
    /// not a list: renumbering must not touch a single byte of them.
    #[test]
    fn renumber_leaves_a_fenced_code_block_alone() {
        let text = "1. Steps:\n   ```text\n   1. alpha\n   1. beta\n   ```\n2. Done\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(out, text, "the fence body is verbatim and the outer list already counts right");
    }

    /// The outer list still renumbers across a fence it contains.
    #[test]
    fn renumber_counts_across_a_fence_inside_an_item() {
        let text = "1. Steps:\n   ```text\n   1. alpha\n   1. beta\n   ```\n1. Done\n";
        let out = renumber(text, 0..text.len()).expect("an ordered list");
        assert_eq!(
            out,
            "1. Steps:\n   ```text\n   1. alpha\n   1. beta\n   ```\n2. Done\n",
            "the item after the fence continues the run; the fence body is untouched"
        );
    }

    /// Same, but with no blank line between the two lists (an ordinary
    /// paragraph line ends the run just as a blank line does).
    #[test]
    fn renumber_restarts_a_separate_list_after_a_bare_paragraph_line() {
        let text = "1. one\n2. two\npara\n1. three\n2. four\n";
        let out = renumber(text, 0..text.len());
        assert_eq!(out.as_deref(), Some(text), "the paragraph line ends the first run");
    }
}

//! Pure formatting toggles behind the floating toolbar and ⌘B/⌘I.
//! Each returns one contiguous replacement plus the selection to keep
//! after it, so the editor applies it as a single undo group.

use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub struct FmtEdit {
    /// Bytes of the document to replace.
    pub range: Range<usize>,
    pub replacement: String,
    /// Selection after the edit, in post-edit document coordinates.
    pub select: Range<usize>,
}

/// Wrap or unwrap the selection with an inline marker (`**`, `*`,
/// `` ` ``, `~~`). Whitespace at the selection edges stays outside the
/// markers.
pub fn toggle_inline(text: &str, sel: Range<usize>, marker: &str) -> FmtEdit {
    let (start, end) = trim_to_content(text, sel);
    let content = &text[start..end];
    let m = marker.len();

    // Markers inside the selection: `**bold**` selected → unwrap.
    if content.len() > 2 * m && content.starts_with(marker) && content.ends_with(marker) {
        let inner = content[m..content.len() - m].to_string();
        return FmtEdit {
            range: start..end,
            select: start..start + inner.len(),
            replacement: inner,
        };
    }

    // Markers just outside: `**|bold|**` with `bold` selected → unwrap.
    if text[..start].ends_with(marker) && text[end..].starts_with(marker) {
        return FmtEdit {
            range: start - m..end + m,
            replacement: content.to_string(),
            select: start - m..start - m + content.len(),
        };
    }

    FmtEdit {
        range: start..end,
        replacement: format!("{marker}{content}{marker}"),
        select: start + m..start + m + content.len(),
    }
}

/// Shrink a selection so leading/trailing whitespace stays outside the
/// markers. An all-whitespace or empty selection collapses in place.
fn trim_to_content(text: &str, sel: Range<usize>) -> (usize, usize) {
    let content = &text[sel.clone()];
    let lead = content.len() - content.trim_start().len();
    let trail = content.len() - content.trim_end().len();
    if lead + trail >= content.len() {
        (sel.start, sel.start)
    } else {
        (sel.start + lead, sel.end - trail)
    }
}

/// Selection → `[selection](url)` with `url` selected for typing over;
/// a selected `[label](target)` unwraps back to `label`.
pub fn toggle_link(text: &str, sel: Range<usize>) -> FmtEdit {
    let (start, end) = trim_to_content(text, sel);
    let content = &text[start..end];

    // A fully selected `[label](target)` unwraps back to its label.
    if let Some(label) = unwrap_link(content) {
        return FmtEdit {
            range: start..end,
            select: start..start + label.len(),
            replacement: label,
        };
    }

    let replacement = format!("[{content}](url)");
    let url_start = start + 1 + content.len() + 2;
    FmtEdit {
        range: start..end,
        replacement,
        select: url_start..url_start + 3,
    }
}

/// `[label](target)` → `label`, provided the brackets pair up exactly
/// around the whole string.
fn unwrap_link(content: &str) -> Option<String> {
    let rest = content.strip_prefix('[')?;
    let close = rest.find(']')?;
    let after = &rest[close + 1..];
    let target = after.strip_prefix('(')?.strip_suffix(')')?;
    if target.contains('(') || rest[..close].contains('[') {
        return None;
    }
    Some(rest[..close].to_string())
}

/// Cycle the heading level of the line containing the selection start:
/// none → # → ## → ### → none.
pub fn cycle_heading(text: &str, sel: Range<usize>) -> FmtEdit {
    let line_start = text[..sel.start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    let line = &text[line_start..line_end];

    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let old_prefix = if hashes > 0 && line[hashes..].starts_with(' ') {
        hashes + 1
    } else if hashes > 0 && line.len() == hashes {
        hashes
    } else {
        0
    };
    let level = if old_prefix > 0 { hashes } else { 0 };
    let new_prefix = match level {
        0 => "# ",
        1 => "## ",
        2 => "### ",
        _ => "",
    };

    // Shift the selection by the prefix delta, clamped to the line.
    let delta = new_prefix.len() as isize - old_prefix as isize;
    let shift = |offset: usize| -> usize {
        if offset < line_start {
            offset
        } else {
            (offset as isize + delta).max(line_start as isize) as usize
        }
    };
    FmtEdit {
        range: line_start..line_start + old_prefix,
        replacement: new_prefix.to_string(),
        select: shift(sel.start)..shift(sel.end),
    }
}

/// Quote or unquote every line the selection touches.
pub fn toggle_quote(text: &str, sel: Range<usize>) -> FmtEdit {
    let block_start = text[..sel.start].rfind('\n').map_or(0, |i| i + 1);
    let block_end = text[sel.end..]
        .find('\n')
        .map_or(text.len(), |i| sel.end + i);
    let block = &text[block_start..block_end];

    let all_quoted = block.lines().all(|l| l.starts_with('>'));
    let replacement: String = block
        .lines()
        .map(|l| {
            if all_quoted {
                let l = &l[1..];
                l.strip_prefix(' ').unwrap_or(l).to_string()
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    FmtEdit {
        select: block_start..block_start + replacement.len(),
        range: block_start..block_end,
        replacement,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply an FmtEdit the way the editor will, returning the new text
    /// and the selected slice.
    fn apply(text: &str, edit: &FmtEdit) -> (String, String) {
        let mut out = String::new();
        out.push_str(&text[..edit.range.start]);
        out.push_str(&edit.replacement);
        out.push_str(&text[edit.range.end..]);
        let selected = out[edit.select.clone()].to_string();
        (out, selected)
    }

    // ── toggle_inline: wrap ──

    #[test]
    fn wrap_plain_selection_in_bold() {
        let edit = toggle_inline("say hello now", 4..9, "**");
        let (out, sel) = apply("say hello now", &edit);
        assert_eq!(out, "say **hello** now");
        assert_eq!(sel, "hello");
    }

    #[test]
    fn wrap_keeps_edge_whitespace_outside_markers() {
        let edit = toggle_inline("say hello now", 3..10, "**");
        let (out, sel) = apply("say hello now", &edit);
        assert_eq!(out, "say **hello** now");
        assert_eq!(sel, "hello");
    }

    #[test]
    fn wrap_empty_selection_places_cursor_between_markers() {
        let edit = toggle_inline("ab", 1..1, "*");
        let (out, _) = apply("ab", &edit);
        assert_eq!(out, "a**b");
        assert_eq!(edit.select, 2..2);
    }

    #[test]
    fn wrap_with_code_and_strike_markers() {
        let (out, _) = apply("run it", &toggle_inline("run it", 0..3, "`"));
        assert_eq!(out, "`run` it");
        let (out, _) = apply("gone", &toggle_inline("gone", 0..4, "~~"));
        assert_eq!(out, "~~gone~~");
    }

    // ── toggle_inline: unwrap ──

    #[test]
    fn unwrap_when_markers_inside_selection() {
        let edit = toggle_inline("say **hello** now", 4..13, "**");
        let (out, sel) = apply("say **hello** now", &edit);
        assert_eq!(out, "say hello now");
        assert_eq!(sel, "hello");
    }

    #[test]
    fn unwrap_when_markers_just_outside_selection() {
        let edit = toggle_inline("say **hello** now", 6..11, "**");
        let (out, sel) = apply("say **hello** now", &edit);
        assert_eq!(out, "say hello now");
        assert_eq!(sel, "hello");
    }

    #[test]
    fn double_toggle_round_trips() {
        let text = "one two three";
        let e1 = toggle_inline(text, 4..7, "**");
        let (t1, _) = apply(text, &e1);
        let e2 = toggle_inline(&t1, e1.select.clone(), "**");
        let (t2, sel) = apply(&t1, &e2);
        assert_eq!(t2, text);
        assert_eq!(sel, "two");
    }

    #[test]
    fn marker_only_selection_wraps_rather_than_panics() {
        // Selecting just "**" is a wrap of literal text, not an unwrap.
        let edit = toggle_inline("**", 0..2, "**");
        let (out, _) = apply("**", &edit);
        assert_eq!(out, "******");
    }

    // ── toggle_link ──

    #[test]
    fn link_wraps_selection_and_selects_url_placeholder() {
        let edit = toggle_link("see docs here", 4..8, );
        let (out, sel) = apply("see docs here", &edit);
        assert_eq!(out, "see [docs](url) here");
        assert_eq!(sel, "url");
    }

    #[test]
    fn link_unwraps_full_link_selection() {
        let text = "see [docs](https://x.y) here";
        let edit = toggle_link(text, 4..23);
        let (out, sel) = apply(text, &edit);
        assert_eq!(out, "see docs here");
        assert_eq!(sel, "docs");
    }

    #[test]
    fn link_with_empty_selection_inserts_skeleton() {
        let edit = toggle_link("ab", 1..1);
        let (out, sel) = apply("ab", &edit);
        assert_eq!(out, "a[](url)b");
        assert_eq!(sel, "url");
    }

    // ── cycle_heading ──

    #[test]
    fn heading_cycles_none_through_three_and_back() {
        let e = cycle_heading("title\nbody", 2..2);
        let (t1, _) = apply("title\nbody", &e);
        assert_eq!(t1, "# title\nbody");
        let e = cycle_heading(&t1, e.select.clone());
        let (t2, _) = apply(&t1, &e);
        assert_eq!(t2, "## title\nbody");
        let e = cycle_heading(&t2, e.select.clone());
        let (t3, _) = apply(&t2, &e);
        assert_eq!(t3, "### title\nbody");
        let e = cycle_heading(&t3, e.select.clone());
        let (t4, _) = apply(&t3, &e);
        assert_eq!(t4, "title\nbody");
    }

    #[test]
    fn heading_acts_on_line_of_selection_start() {
        let text = "one\ntwo\nthree";
        let edit = cycle_heading(text, 5..6);
        let (out, _) = apply(text, &edit);
        assert_eq!(out, "one\n# two\nthree");
    }

    #[test]
    fn heading_selection_shifts_with_prefix() {
        // Selection over "tw" stays over "tw" after the prefix lands.
        let text = "two";
        let edit = cycle_heading(text, 0..2);
        let (out, sel) = apply(text, &edit);
        assert_eq!(out, "# two");
        assert_eq!(sel, "tw");
    }

    #[test]
    fn deep_manual_heading_resets_to_plain() {
        let edit = cycle_heading("##### deep", 7..7);
        let (out, _) = apply("##### deep", &edit);
        assert_eq!(out, "deep");
    }

    // ── toggle_quote ──

    #[test]
    fn quote_adds_prefix_to_touched_lines() {
        let text = "one\ntwo\nthree";
        let edit = toggle_quote(text, 1..6);
        let (out, sel) = apply(text, &edit);
        assert_eq!(out, "> one\n> two\nthree");
        assert_eq!(sel, "> one\n> two");
    }

    #[test]
    fn quote_removes_prefix_when_all_lines_quoted() {
        let text = "> one\n> two";
        let edit = toggle_quote(text, 0..11);
        let (out, sel) = apply(text, &edit);
        assert_eq!(out, "one\ntwo");
        assert_eq!(sel, "one\ntwo");
    }

    #[test]
    fn quote_cursor_only_quotes_its_line() {
        let text = "one\ntwo";
        let edit = toggle_quote(text, 5..5);
        let (out, _) = apply(text, &edit);
        assert_eq!(out, "one\n> two");
    }

    #[test]
    fn quote_handles_bare_gt_lines() {
        let text = ">one\n> two";
        let edit = toggle_quote(text, 0..10);
        let (out, _) = apply(text, &edit);
        assert_eq!(out, "one\ntwo");
    }

    #[test]
    fn mixed_lines_become_all_quoted() {
        let text = "> one\ntwo";
        let edit = toggle_quote(text, 0..9);
        let (out, _) = apply(text, &edit);
        assert_eq!(out, "> > one\n> two");
    }
}

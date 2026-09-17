//! Cross-line block discovery for the document projection: tables,
//! whole-line images, and fenced code blocks (with delimiter lines).

use std::ops::Range;

use pulldown_cmark::{Event, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Table,
    Image {
        alt: String,
        dest: String,
    },
    Fence {
        /// Byte range of the opening ``` line (newline excluded).
        open_line: Range<usize>,
        /// Byte range of the closing line; None when unclosed at EOF.
        close_line: Option<Range<usize>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    pub range: Range<usize>,
    pub kind: BlockKind,
}

/// The line (byte range, newline excluded) containing byte `offset`.
fn line_containing(source: &str, offset: usize) -> Range<usize> {
    let start = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = source[offset..]
        .find('\n')
        .map(|i| offset + i)
        .unwrap_or(source.len());
    start..end
}

pub fn blocks(source: &str) -> Vec<BlockInfo> {
    if source.len() > crate::editor::spans::MAX_STYLED_BYTES {
        return Vec::new();
    }
    let mut out: Vec<BlockInfo> = Vec::new();

    // Tables and images from the event stream.
    let mut image: Option<(Range<usize>, String, String)> = None;
    for (event, range) in crate::editor::spans::body_events(source) {
        match event {
            Event::Start(Tag::Table(_)) => {
                let mut r = range;
                crate::editor::spans::trim_trailing_newline(source, &mut r);
                out.push(BlockInfo { range: r, kind: BlockKind::Table });
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                image = Some((range, String::new(), dest_url.to_string()));
            }
            Event::Text(text) => {
                if let Some((_, alt, _)) = image.as_mut() {
                    alt.push_str(&text);
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some((range, alt, dest)) = image.take() {
                    // Block image only when the markup is the whole line.
                    let line = line_containing(source, range.start);
                    if source[line].trim() == &source[range.clone()] {
                        out.push(BlockInfo { range, kind: BlockKind::Image { alt, dest } });
                    }
                }
            }
            _ => {}
        }
    }

    // Fences from the shared fence scan.
    for fence in crate::editor::spans::fence_infos(source) {
        if !fence.fenced {
            continue;
        }
        let mut open = fence.block.start..fence.body.start;
        crate::editor::spans::trim_trailing_newline(source, &mut open);
        let mut close = fence.body.end..fence.block.end;
        crate::editor::spans::trim_trailing_newline(source, &mut close);
        let close_line = (close.start < close.end).then_some(close);
        let mut range = fence.block.clone();
        crate::editor::spans::trim_trailing_newline(source, &mut range);
        out.push(BlockInfo {
            range,
            kind: BlockKind::Fence { open_line: open, close_line },
        });
    }

    out.sort_by_key(|b| (b.range.start, b.range.end));
    out
}

/// Split a table row into trimmed cells on unescaped pipes; edge pipes
/// don't produce empty edge cells. Escapes are kept verbatim (the cell
/// text is still raw source this phase).
pub fn parse_row(line: &str) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            cells.last_mut().unwrap().push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => {
                cells.last_mut().unwrap().push(ch);
                escaped = true;
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().unwrap().push(c),
        }
    }
    let mut cells: Vec<String> = cells.into_iter().map(|c| c.trim().to_string()).collect();
    let trimmed = line.trim();
    if trimmed.starts_with('|') && cells.first().is_some_and(|c| c.is_empty()) {
        cells.remove(0);
    }
    if trimmed.ends_with('|')
        && !trimmed.ends_with("\\|")
        && cells.last().is_some_and(|c| c.is_empty())
    {
        cells.pop();
    }
    cells
}

/// A row of only dashes/colons cells (`| --- | :-: |`).
pub fn is_separator_row(line: &str) -> bool {
    let cells = parse_row(line);
    !cells.is_empty()
        && cells.iter().all(|c| {
            !c.is_empty() && c.contains('-') && c.chars().all(|ch| matches!(ch, '-' | ':'))
        })
}

/// How a column's cells sit in their cell, from the colons in a
/// table's delimiter row: `:---` left, `:--:` centre, `---:` right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAlign {
    Left,
    Center,
    Right,
}

/// Per-column alignment from a delimiter row. GFM reads a leading
/// colon as left, a trailing one as right, and both as centre; a bare
/// `---` has no opinion and is left alone.
///
/// `render_table` skips the delimiter row when drawing, which is why
/// this has to be read off it first — the colons are the only place
/// the alignment exists, and pulldown-cmark's `Tag::Table(alignments)`
/// is discarded by `blocks()` above.
pub fn column_alignments(separator_row: &str) -> Vec<Option<ColumnAlign>> {
    parse_row(separator_row)
        .iter()
        .map(|cell| {
            let c = cell.trim();
            match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => Some(ColumnAlign::Center),
                (true, false) => Some(ColumnAlign::Left),
                (false, true) => Some(ColumnAlign::Right),
                (false, false) => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_discovered_with_full_range() {
        let src = "a\n\n| h | i |\n| - | - |\n| 1 | 2 |\n\nb\n";
        let all = blocks(src);
        let table = all.iter().find(|b| matches!(b.kind, BlockKind::Table)).unwrap();
        assert_eq!(table.range.start, 3);
        assert!(src[table.range.clone()].contains("| 1 | 2 |"));
    }

    #[test]
    fn whole_line_image_discovered() {
        let src = "para\n\n![alt text](img.png)\n\nafter\n";
        let all = blocks(src);
        let img = all
            .iter()
            .find_map(|b| match &b.kind {
                BlockKind::Image { alt, dest } => {
                    Some((b.range.clone(), alt.clone(), dest.clone()))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(img.1, "alt text");
        assert_eq!(img.2, "img.png");
        assert_eq!(&src[img.0], "![alt text](img.png)");
    }

    #[test]
    fn inline_image_is_not_a_block() {
        let src = "see ![a](b.png) here\n";
        assert!(!blocks(src)
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Image { .. })));
    }

    #[test]
    fn fence_block_with_delimiter_lines() {
        let src = "```rust\nlet x = 1;\n```\n";
        let all = blocks(src);
        let fence = all
            .iter()
            .find_map(|b| match &b.kind {
                BlockKind::Fence { open_line, close_line } => {
                    Some((open_line.clone(), close_line.clone()))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(fence.0, 0..7);
        assert_eq!(fence.1, Some(19..22));
    }

    /// The colons in a delimiter row are the only record of a table's
    /// alignment, and `render_table` skips that row when drawing — so
    /// this parse is the whole feature. Alignment used to be dropped
    /// twice over: `blocks()` matches `Tag::Table(_)`, discarding
    /// pulldown-cmark's alignment vector, and the delimiter row was
    /// then skipped without being read.
    #[test]
    fn column_alignments_come_from_the_delimiter_colons() {
        assert_eq!(
            column_alignments("| :--- | :---: | ---: | --- |"),
            vec![
                Some(ColumnAlign::Left),
                Some(ColumnAlign::Center),
                Some(ColumnAlign::Right),
                None,
            ]
        );
    }

    #[test]
    fn column_alignments_tolerate_ragged_and_padded_rows() {
        assert_eq!(
            column_alignments("|:-:|   ---:   |"),
            vec![Some(ColumnAlign::Center), Some(ColumnAlign::Right)]
        );
    }

    #[test]
    fn unclosed_fence_has_no_close_line() {
        let src = "```rust\nlet x = 1;\n";
        let all = blocks(src);
        let fence = all
            .iter()
            .find_map(|b| match &b.kind {
                BlockKind::Fence { close_line, .. } => Some(close_line.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(fence, None);
    }

    #[test]
    fn nested_image_markup_is_not_a_block() {
        // An image inside another image's alt text: the inner End(Image)
        // consumes the tracked range, so neither becomes a block widget.
        let src = "![a ![b](c)](d)\n";
        assert!(!blocks(src)
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Image { .. })));
    }

    #[test]
    fn indented_code_is_not_a_fence_block() {
        let src = "para\n\n    code\n";
        assert!(!blocks(src)
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Fence { .. })));
    }

    #[test]
    fn parse_row_basic_and_edge_pipes() {
        assert_eq!(parse_row("| a | b |"), vec!["a", "b"]);
        assert_eq!(parse_row("a | b"), vec!["a", "b"]);
        assert_eq!(parse_row("| a |  | c |"), vec!["a", "", "c"]);
    }

    #[test]
    fn parse_row_escaped_pipe() {
        assert_eq!(parse_row(r"| a \| x | b |"), vec![r"a \| x", "b"]);
    }

    #[test]
    fn separator_rows() {
        assert!(is_separator_row("| --- | --- |"));
        assert!(is_separator_row("|:-:|----:|"));
        assert!(!is_separator_row("| a | b |"));
        assert!(!is_separator_row(""));
    }

    /// The projector's block scan skips the metadata the same way: a
    /// fence marker or an image line up there claims nothing.
    #[test]
    fn frontmatter_projects_no_blocks() {
        let src = "---\n![x](y.png)\n```\n---\n\n|a|b|\n|-|-|\n";
        let all = blocks(src);
        assert_eq!(all.len(), 1, "only the body's table: {all:?}");
        assert_eq!(all[0].kind, BlockKind::Table);
        assert_eq!(&src[all[0].range.clone()], "|a|b|\n|-|-|");
    }

    #[test]
    fn oversize_source_has_no_blocks() {
        let big = "x".repeat(crate::editor::spans::MAX_STYLED_BYTES + 1);
        assert!(blocks(&big).is_empty());
    }
}

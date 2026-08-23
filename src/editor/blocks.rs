//! Cross-line block discovery for the document projection: tables,
//! whole-line images, and fenced code blocks (with delimiter lines).

use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

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
    for (event, range) in
        Parser::new_ext(source, crate::editor::spans::markdown_options()).into_offset_iter()
    {
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
    fn oversize_source_has_no_blocks() {
        let big = "x".repeat(crate::editor::spans::MAX_STYLED_BYTES + 1);
        assert!(blocks(&big).is_empty());
    }
}

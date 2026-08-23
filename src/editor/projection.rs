//! Document projection: which display items the editor list shows.
//! A block renders as a widget iff the selection does not touch its
//! source range (Phase 3's reveal rule at block granularity).

use std::ops::Range;

use super::blocks::{BlockInfo, BlockKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// An ordinary source line (rendered through the Phase 3 pipeline).
    Line(usize),
    /// A pretty table consuming this range of source lines.
    Table { lines: Range<usize> },
    /// A rendered image standing in for this source line.
    Image { line: usize, alt: String, dest: String },
}

/// Index of the last line whose start is <= `byte` (the newline after a
/// line belongs to that line).
fn line_of_byte(lines: &[Range<usize>], byte: usize) -> usize {
    let mut ix = 0;
    for (i, range) in lines.iter().enumerate() {
        if range.start <= byte {
            ix = i;
        } else {
            break;
        }
    }
    ix
}

pub fn project(
    lines: &[Range<usize>],
    blocks: &[BlockInfo],
    selection: Range<usize>,
) -> Vec<Item> {
    let touched = |b: &BlockInfo| {
        b.range.start <= selection.end && selection.start <= b.range.end
    };

    // Plan widgets (consume whole line ranges) and skipped delimiter lines.
    struct Widget {
        first: usize,
        last: usize,
        item: Item,
    }
    let mut widgets: Vec<Widget> = Vec::new();
    let mut skip: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut consumed_until = 0usize;

    for block in blocks {
        if touched(block) {
            continue;
        }
        let first = line_of_byte(lines, block.range.start);
        if first < consumed_until {
            continue; // first-wins on overlap
        }
        match &block.kind {
            BlockKind::Table => {
                let last = line_of_byte(lines, block.range.end.max(block.range.start + 1) - 1);
                widgets.push(Widget {
                    first,
                    last,
                    item: Item::Table { lines: first..last + 1 },
                });
                consumed_until = last + 1;
            }
            BlockKind::Image { alt, dest } => {
                widgets.push(Widget {
                    first,
                    last: first,
                    item: Item::Image { line: first, alt: alt.clone(), dest: dest.clone() },
                });
                consumed_until = first + 1;
            }
            BlockKind::Fence { open_line, close_line } => {
                // Only a closed fence can hide its delimiters; body lines
                // always emit, so nothing is consumed.
                if let Some(close) = close_line {
                    skip.insert(line_of_byte(lines, open_line.start));
                    skip.insert(line_of_byte(lines, close.start));
                }
            }
        }
    }

    let mut items = Vec::new();
    let mut widget_ix = 0;
    let mut line = 0;
    while line < lines.len() {
        if widget_ix < widgets.len() && widgets[widget_ix].first == line {
            let widget = &widgets[widget_ix];
            items.push(widget.item.clone());
            line = widget.last + 1;
            widget_ix += 1;
        } else {
            if !skip.contains(&line) {
                items.push(Item::Line(line));
            }
            line += 1;
        }
    }
    items
}

/// The item showing (or nearest to) the given source line.
pub fn item_of_line(items: &[Item], line: usize) -> usize {
    let mut best = 0;
    for (i, item) in items.iter().enumerate() {
        let first = match item {
            Item::Line(l) => *l,
            Item::Table { lines } => lines.start,
            Item::Image { line: l, .. } => *l,
        };
        if first > line {
            break;
        }
        best = i;
        match item {
            Item::Line(l) if *l == line => return i,
            Item::Table { lines } if lines.contains(&line) => return i,
            Item::Image { line: l, .. } if *l == line => return i,
            _ => {}
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(src: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0;
        for line in src.split('\n') {
            out.push(start..start + line.len());
            start += line.len() + 1;
        }
        out
    }

    #[test]
    fn untouched_table_becomes_one_item() {
        // lines: 0 "a", 1 "", 2..5 table rows, 5 "", 6 "b"
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let blocks = [BlockInfo { range: 3..15, kind: BlockKind::Table }];
        let items = project(&lines, &blocks, 0..0);
        assert_eq!(items.len(), 5); // a, blank, Table, blank, b
        assert!(matches!(items[2], Item::Table { ref lines } if *lines == (2..5)));
        assert!(matches!(items[3], Item::Line(5)));
    }

    #[test]
    fn touched_table_dissolves() {
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let blocks = [BlockInfo { range: 3..15, kind: BlockKind::Table }];
        for sel in [3..3, 10..10, 15..15, 1..4] {
            let items = project(&lines, &blocks, sel);
            assert_eq!(items.len(), 7, "all lines emitted");
            assert!(items.iter().all(|i| matches!(i, Item::Line(_))));
        }
        // One-past the range does NOT touch.
        let items = project(&lines, &blocks, 16..16);
        assert_eq!(items.len(), 5);
    }

    #[test]
    fn untouched_image_becomes_item() {
        let src = "x\n![a](p.png)\ny";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 2..13,
            kind: BlockKind::Image { alt: "a".into(), dest: "p.png".into() },
        }];
        let items = project(&lines, &blocks, 0..0);
        assert!(matches!(items[1], Item::Image { line: 1, .. }));
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn untouched_fence_omits_delimiter_lines() {
        let src = "```rust\nlet x = 1;\n```\ntail";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 0..22,
            kind: BlockKind::Fence { open_line: 0..7, close_line: Some(19..22) },
        }];
        let items = project(&lines, &blocks, 100..100);
        // body line 1 and tail line 3 remain
        assert_eq!(items, vec![Item::Line(1), Item::Line(3)]);
        // touched -> all four lines
        let items = project(&lines, &blocks, 10..10);
        assert_eq!(items.len(), 4);
    }

    #[test]
    fn unclosed_fence_never_omits() {
        let src = "```rust\nbody";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 0..12,
            kind: BlockKind::Fence { open_line: 0..7, close_line: None },
        }];
        let items = project(&lines, &blocks, 100..100);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn item_of_line_maps_emitted_consumed_and_omitted() {
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let blocks = [BlockInfo { range: 3..15, kind: BlockKind::Table }];
        let items = project(&lines, &blocks, 0..0);
        assert_eq!(item_of_line(&items, 0), 0);
        assert_eq!(item_of_line(&items, 3), 2); // inside table -> table item
        assert_eq!(item_of_line(&items, 6), 4);
        // omitted fence delimiter maps to nearest emitted neighbor
        let src2 = "```rust\nbody\n```";
        let lines2 = lines_of(src2);
        let blocks2 = [BlockInfo {
            range: 0..16,
            kind: BlockKind::Fence { open_line: 0..7, close_line: Some(13..16) },
        }];
        let items2 = project(&lines2, &blocks2, 100..100);
        assert_eq!(items2, vec![Item::Line(1)]);
        assert_eq!(item_of_line(&items2, 0), 0);
        assert_eq!(item_of_line(&items2, 2), 0);
    }
}

//! Document projection: which display items the editor list shows.
//! Widgets are claimed by registered projectors (see projector.rs);
//! this module alone owns the reveal rule — a claim renders as a
//! widget iff the selection does not touch its source byte range —
//! plus overlap resolution (first claim wins) and fence-delimiter
//! omission.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use super::blocks::{BlockInfo, BlockKind};
use super::projector::Claim;

#[derive(Debug, Clone)]
pub enum Item {
    /// An ordinary source line (rendered through the Phase 3 pipeline).
    Line(usize),
    /// A projector's widget consuming a range of source lines.
    Widget {
        projector: usize,
        lines: Range<usize>,
        payload: Arc<dyn Any + Send + Sync>,
    },
}

/// Payloads are pure functions of the text within `lines`, so identity
/// for change-detection purposes is (projector, lines).
impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Item::Line(a), Item::Line(b)) => a == b,
            (
                Item::Widget { projector: p1, lines: l1, .. },
                Item::Widget { projector: p2, lines: l2, .. },
            ) => p1 == p2 && l1 == l2,
            _ => false,
        }
    }
}

/// Index of the last line whose start is <= `byte` (the newline after a
/// line belongs to that line).
pub(crate) fn line_of_byte(lines: &[Range<usize>], byte: usize) -> usize {
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
    claims: &[(usize, Claim)],
    selection: Range<usize>,
) -> Vec<Item> {
    // Nothing is omitted from the projection. A line is either emitted
    // or consumed by a widget that stands in its place. Fence
    // delimiters used to be the one exception — omitted for a closed,
    // untouched fence — but the reveal rule does not reach them: a
    // marker folds away when something *replaces* it (`**` because the
    // text renders bold, a table's pipes because a widget takes their
    // place), and a fence's content stays as text lines. Hiding
    // ```rust deleted the language with nothing standing in for it.
    // `spans.rs` styles fence delimiters faded and has always
    // documented them as never hidden.

    // Widgets: untouched claims, sorted (start line, registry order),
    // first claim wins on overlap; losers are dropped entirely.
    let mut sorted: Vec<&(usize, Claim)> = claims
        .iter()
        .filter(|(_, c)| !(c.bytes.start <= selection.end && selection.start <= c.bytes.end))
        .collect();
    sorted.sort_by_key(|(p, c)| (c.lines.start, *p));

    let mut widgets: Vec<(usize, &Claim)> = Vec::new();
    let mut consumed_until = 0usize;
    for (projector, claim) in sorted {
        if claim.lines.start < consumed_until {
            continue;
        }
        widgets.push((*projector, claim));
        consumed_until = claim.lines.end;
    }

    let mut items = Vec::new();
    let mut widget_ix = 0;
    let mut line = 0;
    while line < lines.len() {
        if widget_ix < widgets.len() && widgets[widget_ix].1.lines.start == line {
            let (projector, claim) = widgets[widget_ix];
            items.push(Item::Widget {
                projector,
                lines: claim.lines.clone(),
                payload: claim.payload.clone(),
            });
            line = claim.lines.end;
            widget_ix += 1;
        } else {
            items.push(Item::Line(line));
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
            Item::Widget { lines, .. } => lines.start,
        };
        if first > line {
            break;
        }
        best = i;
        match item {
            Item::Line(l) if *l == line => return i,
            Item::Widget { lines, .. } if lines.contains(&line) => return i,
            _ => {}
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::projector::TablePayload;

    fn lines_of(src: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0;
        for line in src.split('\n') {
            out.push(start..start + line.len());
            start += line.len() + 1;
        }
        out
    }

    fn table_claim(lines: &[Range<usize>], bytes: Range<usize>) -> (usize, Claim) {
        let first = line_of_byte(lines, bytes.start);
        let last = line_of_byte(lines, bytes.end.max(bytes.start + 1) - 1);
        (0, Claim { lines: first..last + 1, bytes, payload: Arc::new(TablePayload) })
    }

    #[test]
    fn untouched_claim_becomes_one_widget() {
        // lines: 0 "a", 1 "", 2..5 table rows, 5 "", 6 "b"
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let claims = [table_claim(&lines, 3..15)];
        let items = project(&lines, &[], &claims, 0..0);
        assert_eq!(items.len(), 5); // a, blank, Widget, blank, b
        assert!(
            matches!(&items[2], Item::Widget { projector: 0, lines: l, .. } if *l == (2..5))
        );
        assert!(matches!(items[3], Item::Line(5)));
    }

    #[test]
    fn touched_claim_dissolves() {
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let claims = [table_claim(&lines, 3..15)];
        for sel in [3..3, 10..10, 15..15, 1..4] {
            let items = project(&lines, &[], &claims, sel);
            assert_eq!(items.len(), 7, "all lines emitted");
            assert!(items.iter().all(|i| matches!(i, Item::Line(_))));
        }
        // One-past the range does NOT touch.
        let items = project(&lines, &[], &claims, 16..16);
        assert_eq!(items.len(), 5);
    }

    #[test]
    fn overlapping_claims_first_wins() {
        let src = "x\ny\nz";
        let lines = lines_of(src);
        let a = (0usize, Claim { lines: 0..2, bytes: 0..3, payload: Arc::new(TablePayload) });
        let b = (1usize, Claim { lines: 1..3, bytes: 2..5, payload: Arc::new(TablePayload) });
        let items = project(&lines, &[], &[a, b], 100..100);
        assert!(matches!(&items[0], Item::Widget { projector: 0, .. }));
        assert!(matches!(items[1], Item::Line(2))); // loser dropped entirely
        assert_eq!(items.len(), 2);
    }

    /// A fence keeps its delimiter lines whether or not the cursor is
    /// inside it. They used to be omitted for a closed, untouched
    /// fence, which hid the language tag — the one piece of information
    /// the opening line carries — and left no way to see it but to
    /// click into the block.
    #[test]
    fn a_fence_always_keeps_its_delimiter_lines() {
        let src = "```rust\nlet x = 1;\n```\ntail";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 0..22,
            kind: BlockKind::Fence { open_line: 0..7, close_line: Some(19..22) },
        }];
        for selection in [100..100, 10..10] {
            let items = project(&lines, &blocks, &[], selection.clone());
            assert_eq!(
                items,
                vec![Item::Line(0), Item::Line(1), Item::Line(2), Item::Line(3)],
                "selection {selection:?}: every line, delimiters included"
            );
        }
    }

    #[test]
    fn unclosed_fence_never_omits() {
        let src = "```rust\nbody";
        let lines = lines_of(src);
        let blocks = [BlockInfo {
            range: 0..12,
            kind: BlockKind::Fence { open_line: 0..7, close_line: None },
        }];
        let items = project(&lines, &blocks, &[], 100..100);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn widget_identity_is_projector_and_lines_not_payload() {
        let w = |projector, lines: Range<usize>| Item::Widget {
            projector,
            lines,
            payload: Arc::new(TablePayload),
        };
        assert_eq!(w(0, 2..5), w(0, 2..5)); // distinct payload Arcs still equal
        assert_ne!(w(0, 2..5), w(1, 2..5));
        assert_ne!(w(0, 2..5), w(0, 2..6));
        assert_ne!(w(0, 2..5), Item::Line(2));
    }

    #[test]
    fn item_of_line_maps_emitted_consumed_and_omitted() {
        let src = "a\n\n|h|\n|-|\n|1|\n\nb";
        let lines = lines_of(src);
        let claims = [table_claim(&lines, 3..15)];
        let items = project(&lines, &[], &claims, 0..0);
        assert_eq!(item_of_line(&items, 0), 0);
        assert_eq!(item_of_line(&items, 3), 2); // inside widget -> widget item
        assert_eq!(item_of_line(&items, 6), 4);
        // A fence keeps every line now, so nothing is omitted and each
        // maps to its own item.
        let src2 = "```rust\nbody\n```";
        let lines2 = lines_of(src2);
        let blocks2 = [BlockInfo {
            range: 0..16,
            kind: BlockKind::Fence { open_line: 0..7, close_line: Some(13..16) },
        }];
        let items2 = project(&lines2, &blocks2, &[], 100..100);
        assert_eq!(items2, vec![Item::Line(0), Item::Line(1), Item::Line(2)]);
        assert_eq!(item_of_line(&items2, 0), 0);
        assert_eq!(item_of_line(&items2, 2), 2);
    }
}

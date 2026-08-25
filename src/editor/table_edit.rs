//! Table editing: cell navigation for Tab/Shift-Tab and pipe
//! re-alignment. Pure text analysis over a table block — the editor
//! applies the returned edits.

use std::ops::Range;

/// One parsed table row: its line and the content range of each cell,
/// all relative to the block string.
#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    pub line: Range<usize>,
    pub cells: Vec<Range<usize>>,
    pub is_separator: bool,
}

/// Logical cell coordinates within a block: `row` indexes every line
/// (separator included), `cell` the columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPos {
    pub row: usize,
    pub cell: usize,
}

/// The contiguous run of table lines (trimmed lines starting with `|`)
/// around `offset`, as a byte range of `text`.
pub fn table_block(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = offset.min(text.len());
    let is_table = |line: &str| line.trim_start().starts_with('|');

    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[line_start..].find('\n').map_or(text.len(), |i| line_start + i);
    if !is_table(&text[line_start..line_end]) {
        return None;
    }

    let mut start = line_start;
    while start > 0 {
        let prev_start = text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        if is_table(&text[prev_start..start - 1]) {
            start = prev_start;
        } else {
            break;
        }
    }
    let mut end = line_end;
    while end < text.len() {
        let next_end = text[end + 1..].find('\n').map_or(text.len(), |i| end + 1 + i);
        if is_table(&text[end + 1..next_end]) {
            end = next_end;
        } else {
            break;
        }
    }
    Some(start..end)
}

/// Parse every line of a block into rows with cell content ranges.
pub fn rows(block: &str) -> Vec<Row> {
    let mut out = Vec::new();
    let mut line_start = 0;
    for line in block.split('\n') {
        let line_end = line_start + line.len();

        // Segment boundaries on unescaped pipes, block-relative.
        let mut segs: Vec<Range<usize>> = vec![line_start..line_start];
        let mut escaped = false;
        for (i, ch) in line.char_indices() {
            let at = line_start + i;
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '|' {
                segs.last_mut().unwrap().end = at;
                segs.push(at + ch.len_utf8()..at + ch.len_utf8());
            }
        }
        segs.last_mut().unwrap().end = line_end;

        // Mirror blocks::parse_row: edge pipes make no empty edge cells.
        let trimmed = line.trim();
        let seg_empty = |s: &Range<usize>| block[s.clone()].trim().is_empty();
        if trimmed.starts_with('|') && segs.first().is_some_and(&seg_empty) {
            segs.remove(0);
        }
        if trimmed.ends_with('|')
            && !trimmed.ends_with("\\|")
            && segs.len() > 1
            && segs.last().is_some_and(&seg_empty)
        {
            segs.pop();
        }

        let cells = segs
            .iter()
            .map(|seg| {
                let s = &block[seg.clone()];
                let lead = s.len() - s.trim_start().len();
                let trail = s.len() - s.trim_end().len();
                if lead + trail >= s.len() {
                    let at = seg.start + s.len().min(1);
                    at..at
                } else {
                    seg.start + lead..seg.end - trail
                }
            })
            .collect();
        out.push(Row {
            line: line_start..line_end,
            cells,
            is_separator: super::blocks::is_separator_row(line),
        });
        line_start = line_end + 1;
    }
    out
}

/// The cell whose line contains `offset` — the nearest cell on that
/// row when the offset sits on a pipe or padding.
pub fn cell_at(block: &str, offset: usize) -> Option<CellPos> {
    let rs = rows(block);
    let row = rs
        .iter()
        .position(|r| offset <= r.line.end)
        .unwrap_or(rs.len().checked_sub(1)?);
    let cells = &rs.get(row)?.cells;
    if cells.is_empty() {
        return None;
    }
    let cell = cells
        .iter()
        .position(|c| offset <= c.end)
        .unwrap_or(cells.len() - 1);
    Some(CellPos { row, cell })
}

/// The next (or previous) content cell, skipping separator rows.
/// None means Tab walked off the table's end.
pub fn next_pos(block: &str, pos: CellPos, backward: bool) -> Option<CellPos> {
    let rs = rows(block);
    let row = rs.get(pos.row)?;
    if backward {
        if pos.cell > 0 {
            return Some(CellPos { row: pos.row, cell: pos.cell - 1 });
        }
        for r in (0..pos.row).rev() {
            if !rs[r].is_separator && !rs[r].cells.is_empty() {
                return Some(CellPos { row: r, cell: rs[r].cells.len() - 1 });
            }
        }
    } else {
        if pos.cell + 1 < row.cells.len() {
            return Some(CellPos { row: pos.row, cell: pos.cell + 1 });
        }
        for (r, rr) in rs.iter().enumerate().skip(pos.row + 1) {
            if !rr.is_separator && !rr.cells.is_empty() {
                return Some(CellPos { row: r, cell: 0 });
            }
        }
    }
    None
}

/// Content range of a cell, relative to the block. An empty cell
/// yields a collapsed range at its padding point.
pub fn cell_range(block: &str, pos: CellPos) -> Option<Range<usize>> {
    rows(block).get(pos.row)?.cells.get(pos.cell).cloned()
}

/// Per-column display widths (chars), honoring separator colon minima.
fn column_widths(block: &str, rs: &[Row]) -> Vec<usize> {
    let ncols = rs.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let mut widths = vec![1usize; ncols];
    for r in rs {
        for (c, cell) in r.cells.iter().enumerate() {
            let s = &block[cell.clone()];
            let min = if r.is_separator {
                // A colon each side must survive beside >= 1 dash.
                1 + s.starts_with(':') as usize + s.ends_with(':') as usize
            } else {
                s.chars().count()
            };
            widths[c] = widths[c].max(min);
        }
    }
    widths
}

/// Re-pad the block so every pipe lines up; separator colons survive.
pub fn align(block: &str) -> String {
    let rs = rows(block);
    let widths = column_widths(block, &rs);
    let mut lines = Vec::with_capacity(rs.len());
    for r in &rs {
        let mut line = String::from("|");
        for (c, w) in widths.iter().enumerate() {
            let content = r
                .cells
                .get(c)
                .map(|cell| &block[cell.clone()])
                .unwrap_or("");
            let rendered = if r.is_separator {
                let (l, rt) = (content.starts_with(':'), content.ends_with(':'));
                let dashes = w - l as usize - rt as usize;
                format!(
                    "{}{}{}",
                    if l { ":" } else { "" },
                    "-".repeat(dashes),
                    if rt { ":" } else { "" }
                )
            } else {
                let pad = w - content.chars().count();
                format!("{content}{}", " ".repeat(pad))
            };
            line.push(' ');
            line.push_str(&rendered);
            line.push_str(" |");
        }
        lines.push(line);
    }
    lines.join("\n")
}

/// A fresh empty row matching the (aligned) block's column widths,
/// without leading/trailing newline.
pub fn new_row(block: &str) -> String {
    let rs = rows(block);
    let mut line = String::from("|");
    for w in column_widths(block, &rs) {
        line.push_str(&" ".repeat(w + 2));
        line.push('|');
    }
    line
}

/// Map `offset` through `align`: same row, same cell, same distance
/// into the cell content.
pub fn map_offset(block: &str, aligned: &str, offset: usize) -> usize {
    let Some(pos) = cell_at(block, offset) else {
        return offset.min(aligned.len());
    };
    // Past the row's last cell (the trailing `|` region): stay at the
    // row's end so an Enter there still lands after the row.
    let rs = rows(block);
    if let (Some(row), Some(arow)) = (rs.get(pos.row), rows(aligned).get(pos.row)) {
        if row.cells.last().is_some_and(|c| offset > c.end) {
            return arow.line.end;
        }
    }
    let (Some(from), Some(to)) = (cell_range(block, pos), cell_range(aligned, pos)) else {
        return offset.min(aligned.len());
    };
    let intra = offset.saturating_sub(from.start).min(to.len());
    to.start + intra
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAGGED: &str = "| h1 | h2 |\n|---|---|\n| a | bbbb |\n| cc | d |";
    const ALIGNED: &str = "| h1 | h2   |\n| -- | ---- |\n| a  | bbbb |\n| cc | d    |";

    #[test]
    fn block_spans_contiguous_pipe_lines() {
        let text = "before\n| a |\n| - |\n| b |\nafter";
        let block = table_block(text, text.find("| b").unwrap()).unwrap();
        assert_eq!(&text[block], "| a |\n| - |\n| b |");
        assert_eq!(table_block(text, 0), None, "prose is not a table");
    }

    #[test]
    fn block_at_document_edges() {
        let text = "| a |\n| - |\n| b |";
        assert_eq!(table_block(text, 2).unwrap(), 0..text.len());
        assert_eq!(table_block(text, text.len()).unwrap(), 0..text.len());
    }

    #[test]
    fn rows_carry_cell_content_ranges() {
        let block = "| aa | b |\n| x | yy |";
        let rs = rows(block);
        assert_eq!(rs.len(), 2);
        assert_eq!(&block[rs[0].cells[0].clone()], "aa");
        assert_eq!(&block[rs[0].cells[1].clone()], "b");
        assert_eq!(&block[rs[1].cells[1].clone()], "yy");
        assert!(!rs[0].is_separator);
    }

    #[test]
    fn separator_rows_are_marked() {
        let rs = rows("| h |\n| :-: |\n| v |");
        assert!(rs[1].is_separator);
        assert!(!rs[2].is_separator);
    }

    #[test]
    fn escaped_pipes_stay_inside_their_cell() {
        let block = "| a\\|b | c |";
        let rs = rows(block);
        assert_eq!(rs[0].cells.len(), 2);
        assert_eq!(&block[rs[0].cells[0].clone()], "a\\|b");
    }

    #[test]
    fn cell_at_finds_the_cell_under_the_cursor() {
        let block = "| aa | bb |\n| cc | dd |";
        let in_bb = block.find("bb").unwrap() + 1;
        assert_eq!(cell_at(block, in_bb), Some(CellPos { row: 0, cell: 1 }));
        let in_cc = block.find("cc").unwrap();
        assert_eq!(cell_at(block, in_cc), Some(CellPos { row: 1, cell: 0 }));
        // On the leading pipe: nearest cell is the first.
        assert_eq!(cell_at(block, 0), Some(CellPos { row: 0, cell: 0 }));
    }

    #[test]
    fn next_pos_walks_cells_and_skips_the_separator() {
        let block = "| a | b |\n| - | - |\n| c | d |";
        let p = |row, cell| CellPos { row, cell };
        assert_eq!(next_pos(block, p(0, 0), false), Some(p(0, 1)));
        assert_eq!(next_pos(block, p(0, 1), false), Some(p(2, 0)), "skips separator");
        assert_eq!(next_pos(block, p(2, 0), false), Some(p(2, 1)));
        assert_eq!(next_pos(block, p(2, 1), false), None, "off the end");
        assert_eq!(next_pos(block, p(2, 0), true), Some(p(0, 1)), "back over separator");
        assert_eq!(next_pos(block, p(0, 0), true), None, "before the start");
    }

    #[test]
    fn align_pads_every_column_to_its_widest_cell() {
        assert_eq!(align(RAGGED), ALIGNED);
    }

    #[test]
    fn align_preserves_separator_colons() {
        let block = "| head | b |\n|:---|--:|\n| x | y |";
        let aligned = align(block);
        assert_eq!(aligned, "| head | b  |\n| :--- | -: |\n| x    | y  |");
    }

    #[test]
    fn align_pads_short_rows_with_empty_cells() {
        let aligned = align("| a | b |\n| - | - |\n| c |");
        assert_eq!(aligned, "| a | b |\n| - | - |\n| c |   |");
    }

    #[test]
    fn align_is_idempotent() {
        assert_eq!(align(ALIGNED), ALIGNED);
    }

    #[test]
    fn new_row_matches_the_column_count() {
        assert_eq!(new_row(ALIGNED), "|    |      |");
    }

    #[test]
    fn map_offset_keeps_line_ends_outside_the_cells() {
        // Cursor at a row's end (after the closing pipe) must stay at
        // the row's end — mapping it into the last cell would make a
        // following Enter split the row.
        let block = "| a | bbbb |\n| ppp | q |";
        let aligned = align(block);
        let row_end = block.find('\n').unwrap();
        let mapped = map_offset(block, &aligned, row_end);
        assert_eq!(mapped, aligned.find('\n').unwrap());
        let mapped = map_offset(block, &aligned, block.len());
        assert_eq!(mapped, aligned.len());
    }

    #[test]
    fn map_offset_lands_in_the_same_cell() {
        let aligned = align(RAGGED);
        // Inside "bbbb" on the ragged text → same spot in the aligned text.
        let off = RAGGED.find("bbbb").unwrap() + 2;
        let mapped = map_offset(RAGGED, &aligned, off);
        assert_eq!(&aligned[mapped - 2..mapped + 2], "bbbb");
        // End of a cell's content maps to end of content, not padding.
        let end = RAGGED.find("cc").unwrap() + 2;
        let mapped = map_offset(RAGGED, &aligned, end);
        assert_eq!(&aligned[mapped - 2..mapped], "cc");
    }
}

//! Structural table edits: whole rows and columns. Pure.

use super::table_edit;

/// Where a row asked for "after row `after`" actually lands. A row
/// between the header and its separator would stop the block being a
/// table at all, so anything above the separator inserts below it
/// instead -- and the cursor has to follow the row, so the caller
/// needs this index too.
pub fn insert_row_index(block: &str, after: usize) -> usize {
    let rs = table_edit::rows(block);
    match rs.iter().position(|r| r.is_separator) {
        Some(sep) if after < sep => sep,
        _ => after,
    }
}

/// A fresh empty row inserted immediately after row `after`, re-aligned
/// so every pipe still lines up.
pub fn insert_row(block: &str, after: usize) -> String {
    let after = insert_row_index(block, after);
    let blank = table_edit::new_row(block);
    let rs = table_edit::rows(block);
    let out = match rs.get(after) {
        Some(row) => format!("{}\n{blank}{}", &block[..row.line.end], &block[row.line.end..]),
        None => format!("{block}\n{blank}"),
    };
    table_edit::align(&out)
}

/// Remove row `row`. Refuses (returns `None`) to delete the separator
/// row or the header above it — those lines are structure, not data;
/// a block that loses either stops being a table and every remaining
/// row falls out of it — or an out-of-range row.
///
/// No other row's content changes, so this does not re-align: the
/// remaining lines, separator included, keep their exact text.
pub fn delete_row(block: &str, row: usize) -> Option<String> {
    let rs = table_edit::rows(block);
    let r = rs.get(row)?;
    if r.is_separator || rs.iter().position(|r| r.is_separator).is_some_and(|sep| row < sep) {
        return None;
    }
    let out = if r.line.end < block.len() {
        // Not the last line: drop it together with its trailing newline.
        format!("{}{}", &block[..r.line.start], &block[r.line.end + 1..])
    } else {
        // The last line has no trailing newline; drop the one before it.
        let start = r.line.start.saturating_sub(1);
        format!("{}{}", &block[..start], &block[r.line.end..])
    };
    Some(out)
}

/// Insert a new blank column immediately after column `after`, in
/// every row including the separator (which gets a plain `-` cell so
/// it still parses as a separator once re-aligned).
pub fn insert_column(block: &str, after: usize) -> String {
    let rs = table_edit::rows(block);
    let mut lines = Vec::with_capacity(rs.len());
    for r in &rs {
        let mut cells: Vec<&str> = r.cells.iter().map(|c| &block[c.clone()]).collect();
        let at = (after + 1).min(cells.len());
        cells.insert(at, if r.is_separator { "-" } else { "" });
        lines.push(format!("| {} |", cells.join(" | ")));
    }
    table_edit::align(&lines.join("\n"))
}

/// Remove column `col` from every row. Refuses (returns `None`) when
/// the table has only one column left — a table cannot lose its last
/// column and remain a table.
pub fn delete_column(block: &str, col: usize) -> Option<String> {
    let rs = table_edit::rows(block);
    let ncols = rs.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    if ncols <= 1 {
        return None;
    }
    let mut lines = Vec::with_capacity(rs.len());
    for r in &rs {
        let mut cells: Vec<&str> = r.cells.iter().map(|c| &block[c.clone()]).collect();
        if col < cells.len() {
            cells.remove(col);
        }
        lines.push(format!("| {} |", cells.join(" | ")));
    }
    Some(table_edit::align(&lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";

    #[test]
    fn insert_row_adds_an_empty_row_after_the_given_one() {
        // Row 2 is "| 1 | 2 |" (row 1 is the separator).
        let out = insert_row(T, 2);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 5, "one more row: {out}");
        assert!(lines[3].starts_with('|') && lines[3].contains("  "), "blank cells: {:?}", lines[3]);
        assert_eq!(lines[4], "| 3 | 4 |", "the row below survives");
    }

    /// The most natural first use of the command: the cursor is still
    /// in the header the user just typed. A row inserted literally
    /// after row 0 lands between the header and its separator, which
    /// stops the whole thing being a table at all.
    #[test]
    fn a_row_inserted_from_the_header_lands_below_the_separator() {
        let out = insert_row(T, 0);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 5, "one more row: {out}");
        assert_eq!(lines[0].replace(' ', ""), "|a|b|", "the header is still the header");
        let rs = table_edit::rows(&out);
        assert!(rs[1].is_separator, "the separator still follows it: {:?}", lines[1]);
        assert!(lines[2].trim_matches(['|', ' ']).is_empty(), "blank row: {:?}", lines[2]);
        assert_eq!(insert_row_index(T, 0), 1, "reported as row 1, where the cursor must go");
        // And it is still a table to the block scanner.
        assert_eq!(crate::editor::blocks::blocks(&out).len(), 1, "one table block: {out}");
    }

    /// From the separator itself, the new row is the first body row.
    #[test]
    fn a_row_inserted_from_the_separator_becomes_the_first_body_row() {
        let out = insert_row(T, 1);
        let lines: Vec<&str> = out.lines().collect();
        assert!(table_edit::rows(&out)[1].is_separator, "separator untouched: {:?}", lines[1]);
        assert!(lines[2].trim_matches(['|', ' ']).is_empty(), "blank row: {:?}", lines[2]);
        assert_eq!(lines[3].replace(' ', ""), "|1|2|");
        assert_eq!(insert_row_index(T, 1), 1);
    }

    /// A body row still inserts exactly where the cursor is.
    #[test]
    fn insert_row_index_leaves_body_rows_alone() {
        assert_eq!(insert_row_index(T, 2), 2);
        assert_eq!(insert_row_index(T, 3), 3);
        // A pipe block with no separator is not a real table; nothing
        // to protect, so the index passes straight through.
        assert_eq!(insert_row_index("| a |\n| b |", 0), 0);
    }

    /// Deleting the header leaves a separator with nothing above it --
    /// no longer a table, and the rest of the rows fall out with it.
    #[test]
    fn the_header_row_cannot_be_deleted() {
        assert!(delete_row(T, 0).is_none(), "row 0 is the header");
    }

    /// The separator row is structure, not data.
    #[test]
    fn the_separator_row_cannot_be_deleted() {
        assert!(delete_row(T, 1).is_none(), "row 1 is the separator");
    }

    #[test]
    fn delete_row_removes_only_that_row() {
        let out = delete_row(T, 2).expect("a body row");
        assert!(!out.contains("| 1 | 2 |"), "{out}");
        assert!(out.contains("| 3 | 4 |"), "{out}");
        assert!(out.contains("| --- |"), "the separator stays: {out}");
    }

    #[test]
    fn insert_column_widens_every_row_including_the_separator() {
        let out = insert_column(T, 0);
        for line in out.lines() {
            assert_eq!(line.matches('|').count(), 4, "three cells now: {line:?}");
        }
    }

    #[test]
    fn delete_column_narrows_every_row() {
        let out = delete_column(T, 0).expect("two columns exist");
        for line in out.lines() {
            assert_eq!(line.matches('|').count(), 2, "one cell left: {line:?}");
        }
        assert!(out.contains('b'), "the other column survives: {out}");
    }

    /// A one-column table cannot lose its last column.
    #[test]
    fn the_last_column_cannot_be_deleted() {
        let one = "| a |\n| --- |\n| 1 |";
        assert!(delete_column(one, 0).is_none());
    }
}

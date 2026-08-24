//! Viewer plugin: csv/tsv files render as a markdown table (the host
//! shows it with the Reader; ⌘E toggles to the raw source).

wit_bindgen::generate!({ path: "../wit-v4", world: "extension" });

use supermd::extension::types as t;

const MAX_ROWS: usize = 500;

/// CSV/TSV → markdown table. Delimiter: tab when the first line has
/// one, else comma. Errors on non-tabular content (host falls back to
/// the source editor).
pub fn csv_markdown(content: &str) -> Result<String, String> {
    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
    let Some(first) = lines.next() else {
        return Err("empty file".to_string());
    };
    let delim = if first.contains('\t') { '\t' } else { ',' };
    let split = |line: &str| -> Vec<String> {
        line.split(delim)
            .map(|cell| cell.trim().replace('|', "\\|"))
            .collect()
    };
    let header = split(first);
    if header.len() < 2 {
        return Err("not tabular (fewer than 2 columns)".to_string());
    }
    let width = header.len();
    let mut out = String::new();
    out.push_str(&format!("| {} |\n", header.join(" | ")));
    out.push_str(&format!("|{}\n", " --- |".repeat(width)));
    let mut shown = 0usize;
    let mut skipped = 0usize;
    for line in lines {
        if shown >= MAX_ROWS {
            skipped += 1;
            continue;
        }
        let mut row = split(line);
        row.resize(width, String::new());
        row.truncate(width);
        out.push_str(&format!("| {} |\n", row.join(" | ")));
        shown += 1;
    }
    if skipped > 0 {
        out.push_str(&format!("\n… {skipped} more rows\n"));
    }
    Ok(out)
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_: String, _: String, _: t::Theme) -> Result<String, String> {
        Err("unused".into())
    }

    fn run_command(_: String, _: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("unused".into())
    }

    fn render_inline(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn format_document(d: String) -> Result<String, String> {
        Ok(d)
    }

    fn process_paste(_: String) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn export_document(_: String, _: String, _: t::Theme) -> Result<Vec<ExportFile>, String> {
        Err("unused".into())
    }

    fn render_view(_filename: String, content: String) -> Result<String, String> {
        csv_markdown(&content)
    }

    fn status_text(_: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn render_template(_: String, _: TemplateContext) -> Result<TemplateFile, String> {
        Err("unused".into())
    }

    fn on_save(_: String, _: String) -> Result<Option<String>, String> {
        Ok(None)
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::csv_markdown;

    #[test]
    fn renders_comma_and_tab_tables() {
        let md = csv_markdown("a,b\n1,2\n3,4\n").unwrap();
        assert!(md.contains("| a | b |"), "{md}");
        assert!(md.contains("| --- | --- |"), "{md}");
        assert!(md.contains("| 3 | 4 |"), "{md}");
        let md = csv_markdown("x\ty\n1\t2\n").unwrap();
        assert!(md.contains("| x | y |"), "{md}");
    }

    #[test]
    fn escapes_pipes_and_caps_rows() {
        let md = csv_markdown("h1,h2\na|b,c\n").unwrap();
        assert!(md.contains("a\\|b"), "{md}");
        let big: String = std::iter::once("h,i".to_string())
            .chain((0..600).map(|n| format!("{n},{n}")))
            .collect::<Vec<_>>()
            .join("\n");
        let md = csv_markdown(&big).unwrap();
        assert!(md.contains("… 100 more rows"), "{md}");
        assert!(!md.contains("| 599 |"), "{md}");
    }

    #[test]
    fn rejects_non_tabular() {
        assert!(csv_markdown("just some prose without delimiters").is_err());
        assert!(csv_markdown("").is_err());
    }
}

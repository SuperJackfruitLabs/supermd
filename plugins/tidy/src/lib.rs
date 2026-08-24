//! Tidy: formatter (smart punctuation, blank-line collapse, trailing
//! whitespace) and paste processor (TSV/CSV → markdown table).

wit_bindgen::generate!({ path: "../wit-v2", world: "extension" });
use supermd::extension::types as t;

/// Smart punctuation on prose (fenced code left untouched).
pub fn tidy(document: &str) -> String {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut blank_run = 0;
    for line in document.lines() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~") {
            in_fence = !in_fence;
            out.push(line.trim_end().to_string());
            blank_run = 0;
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run >= 3 {
                continue; // collapse 3+ blank lines to 2
            }
            out.push(String::new());
            continue;
        }
        blank_run = 0;
        out.push(smart_punctuation(line));
    }
    let mut s = out.join("\n");
    if document.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn smart_punctuation(line: &str) -> String {
    // Straight quotes → curly (open after start/space/parens), inline
    // code spans skipped; -- → en dash, --- → em dash.
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    let mut prev: Option<char> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            in_code = !in_code;
            out.push(c);
            prev = Some(c);
            i += 1;
            continue;
        }
        if in_code {
            out.push(c);
            prev = Some(c);
            i += 1;
            continue;
        }
        match c {
            '-' if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] == '-' => {
                out.push('—');
                prev = Some('—');
                i += 3;
                continue;
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                out.push('–');
                prev = Some('–');
                i += 2;
                continue;
            }
            '"' => {
                let open = matches!(prev, None | Some(' ') | Some('(') | Some('['));
                out.push(if open { '“' } else { '”' });
            }
            '\'' => {
                let open = matches!(prev, None | Some(' ') | Some('(') | Some('['));
                out.push(if open { '‘' } else { '’' });
            }
            _ => out.push(c),
        }
        prev = out.chars().last();
        i += 1;
    }
    out
}

/// Detect pasted TSV/comma-CSV and convert to a markdown table.
pub fn csv_to_table(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    if lines.len() < 2 {
        return None;
    }
    let sep = if lines.iter().all(|l| l.contains('\t')) {
        '\t'
    } else if lines.iter().all(|l| l.contains(',') && !l.contains('"')) {
        ','
    } else {
        return None;
    };
    let rows: Vec<Vec<&str>> = lines.iter().map(|l| l.split(sep).map(str::trim).collect()).collect();
    let cols = rows[0].len();
    if cols < 2 || rows.iter().any(|r| r.len() != cols) {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("| {} |\n", rows[0].join(" | ")));
    out.push_str(&format!("|{}\n", " --- |".repeat(cols)));
    for row in &rows[1..] {
        out.push_str(&format!("| {} |\n", row.join(" | ")));
    }
    Some(out)
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_l: String, _s: String, _t: t::Theme) -> Result<String, String> {
        Err("no blocks".into())
    }
    fn run_command(_i: String, _in: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("no commands".into())
    }
    fn render_inline(_p: String, _m: String) -> Result<String, String> {
        Err("no inline".into())
    }
    fn format_document(document: String) -> Result<String, String> {
        Ok(tidy(&document))
    }
    fn process_paste(text: String) -> Result<Option<String>, String> {
        Ok(csv_to_table(&text))
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_quotes_and_dashes() {
        assert_eq!(smart_punctuation("say \"hi\" -- ok"), "say “hi” – ok");
        assert_eq!(smart_punctuation("a --- b"), "a — b");
        assert_eq!(smart_punctuation("`\"raw\"` stays"), "`\"raw\"` stays");
    }

    #[test]
    fn collapses_blank_runs_and_trailing_space() {
        assert_eq!(tidy("a  \n\n\n\n\nb\n"), "a\n\n\nb\n");
    }

    #[test]
    fn fences_untouched() {
        let doc = "```\nsay \"hi\" -- ok   \n```\n";
        assert_eq!(tidy(doc), "```\nsay \"hi\" -- ok   \n```\n");
    }

    #[test]
    fn tsv_becomes_table_and_quoted_csv_declines() {
        let out = csv_to_table("a\tb\n1\t2\n").unwrap();
        assert!(out.starts_with("| a | b |\n| --- | --- |\n"));
        assert!(csv_to_table("a,\"x,y\"\n1,2\n").is_none());
        assert!(csv_to_table("plain text\n").is_none());
    }
}

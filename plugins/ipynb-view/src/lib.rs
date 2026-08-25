//! Viewer: .ipynb notebooks render as documents — markdown cells as
//! prose, code cells as highlighted fences, text outputs beneath them.

wit_bindgen::generate!({ path: "../wit-v4", world: "extension" });

use supermd::extension::types as t;

const MAX_OUTPUT_LINES: usize = 40;

/// Notebook JSON → markdown for the Reader.
pub fn notebook_markdown(json: &str) -> Result<String, String> {
    let nb: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("not a notebook: {e}"))?;
    let cells = nb
        .get("cells")
        .and_then(|c| c.as_array())
        .ok_or("no cells in notebook")?;
    let lang = nb
        .pointer("/metadata/language_info/name")
        .and_then(|l| l.as_str())
        .unwrap_or("python")
        .to_string();

    let mut out = String::new();
    for cell in cells {
        let kind = cell.get("cell_type").and_then(|t| t.as_str()).unwrap_or("");
        let source = joined_text(cell.get("source"));
        match kind {
            "markdown" => {
                out.push_str(source.trim_end());
                out.push_str("\n\n");
            }
            "code" => {
                if source.trim().is_empty() {
                    continue;
                }
                out.push_str(&format!("```{lang}\n{}\n```\n\n", source.trim_end()));
                for output in cell
                    .get("outputs")
                    .and_then(|o| o.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(rendered) = render_output(output) {
                        out.push_str(&rendered);
                        out.push_str("\n\n");
                    }
                }
            }
            _ => {} // raw cells and future kinds are skipped
        }
    }
    if out.trim().is_empty() {
        return Err("notebook has no renderable cells".to_string());
    }
    Ok(out)
}

/// Notebook "source"/"text" fields are either a string or a line list.
fn joined_text(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(lines)) => lines
            .iter()
            .filter_map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn render_output(output: &serde_json::Value) -> Option<String> {
    let kind = output.get("output_type").and_then(|t| t.as_str())?;
    let text = match kind {
        "stream" => joined_text(output.get("text")),
        "execute_result" | "display_data" => {
            let data = output.get("data")?;
            if data.get("image/png").is_some() || data.get("image/jpeg").is_some() {
                return Some("*[image output]*".to_string());
            }
            joined_text(data.get("text/plain"))
        }
        "error" => {
            let name = output.get("ename").and_then(|n| n.as_str()).unwrap_or("Error");
            let value = output.get("evalue").and_then(|v| v.as_str()).unwrap_or("");
            format!("{name}: {value}")
        }
        _ => return None,
    };
    let text = strip_ansi(&text);
    if text.trim().is_empty() {
        return None;
    }
    let mut lines: Vec<&str> = text.lines().collect();
    let truncated = lines.len() > MAX_OUTPUT_LINES;
    lines.truncate(MAX_OUTPUT_LINES);
    let mut block = format!("```\n{}\n```", lines.join("\n").trim_end());
    if truncated {
        block.push_str("\n*…output truncated*");
    }
    Some(block)
}

/// Drop ANSI escape sequences (tracebacks are full of them).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // skip until a letter ends the escape sequence
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
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
        notebook_markdown(&content)
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
    use super::*;

    const NOTEBOOK: &str = r##"{
        "metadata": {"language_info": {"name": "python"}},
        "cells": [
            {"cell_type": "markdown", "source": ["# Analysis\n", "Some *prose*.\n"]},
            {"cell_type": "code", "source": ["print(1 + 1)\n"],
             "outputs": [{"output_type": "stream", "text": ["2\n"]}]},
            {"cell_type": "code", "source": "x = 5",
             "outputs": [{"output_type": "execute_result",
                          "data": {"text/plain": ["5"]}}]},
            {"cell_type": "code", "source": ["plot()\n"],
             "outputs": [{"output_type": "display_data",
                          "data": {"image/png": "aGk="}}]},
            {"cell_type": "code", "source": ["boom\n"],
             "outputs": [{"output_type": "error", "ename": "ValueError",
                          "evalue": "bad", "traceback": ["trace"]}]},
            {"cell_type": "raw", "source": ["ignored"]}
        ]
    }"##;

    #[test]
    fn renders_cells_outputs_and_placeholders() {
        let md = notebook_markdown(NOTEBOOK).unwrap();
        assert!(md.contains("# Analysis"), "{md}");
        assert!(md.contains("```python\nprint(1 + 1)\n```"), "{md}");
        assert!(md.contains("```\n2\n```"), "stream output");
        assert!(md.contains("```\n5\n```"), "execute_result");
        assert!(md.contains("*[image output]*"), "image placeholder");
        assert!(md.contains("ValueError: bad"), "error summary");
        assert!(!md.contains("ignored"), "raw cells skipped");
    }

    #[test]
    fn long_outputs_truncate() {
        let big: String = (0..100).map(|i| format!("line {i}\\n")).collect();
        let nb = format!(
            r#"{{"cells":[{{"cell_type":"code","source":["x"],
                "outputs":[{{"output_type":"stream","text":["{big}"]}}]}}]}}"#
        );
        let md = notebook_markdown(&nb).unwrap();
        assert!(md.contains("…output truncated"), "{md}");
        assert!(!md.contains("line 99"));
    }

    #[test]
    fn garbage_and_empty_notebooks_error() {
        assert!(notebook_markdown("not json").is_err());
        assert!(notebook_markdown(r#"{"cells": []}"#).is_err());
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("a\u{1b}[31mred\u{1b}[0mb"), "aredb");
    }
}

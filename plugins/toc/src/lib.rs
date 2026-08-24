//! Table-of-contents plugin: pure text logic + WIT exports.

wit_bindgen::generate!({ path: "../wit-v4", world: "extension" });
use supermd::extension::types as t;

/// GitHub-style slug: lowercase, alphanumerics kept, spaces → '-'.
fn slug(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Build a TOC from ATX headings, skipping fenced code blocks.
pub fn build_toc(document: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in document.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if hashes == 0 || hashes > 6 {
            continue;
        }
        let rest = trimmed[hashes..].trim();
        if rest.is_empty() {
            continue;
        }
        let indent = "  ".repeat(hashes - 1);
        out.push_str(&format!("{indent}- [{rest}](#{})\n", slug(rest)));
    }
    out
}

const OPEN: &str = "<!-- toc -->";
const CLOSE: &str = "<!-- /toc -->";

/// Replace the content between the toc markers with a fresh TOC.
pub fn update_between_markers(document: &str) -> Result<String, String> {
    let start = document.find(OPEN).ok_or_else(|| {
        format!("no {OPEN} marker found — add {OPEN} and {CLOSE} around your TOC")
    })?;
    let after_open = start + OPEN.len();
    let close_rel = document[after_open..].find(CLOSE).ok_or_else(|| {
        format!("found {OPEN} but no closing {CLOSE} marker")
    })?;
    let close = after_open + close_rel;
    let toc = build_toc(document);
    Ok(format!(
        "{}{}\n{}{}{}",
        &document[..after_open],
        "",
        toc,
        &document[close..close], // nothing; kept for clarity
        &document[close..]
    ))
}

/// Save-hook body: refresh the markers when present AND stale;
/// None = leave the save untouched.
pub fn save_hook(document: &str) -> Option<String> {
    let updated = update_between_markers(document).ok()?;
    (updated != document).then_some(updated)
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_lang: String, _source: String, _theme: t::Theme) -> Result<String, String> {
        Err("toc has no block renderer".to_string())
    }

    fn run_command(id: String, input: t::CommandInput) -> Result<t::CommandOutput, String> {
        match id.as_str() {
            "toc.insert" => Ok(t::CommandOutput::InsertAtCursor(build_toc(&input.document))),
            "toc.update" => {
                update_between_markers(&input.document).map(t::CommandOutput::ReplaceDocument)
            }
            other => Err(format!("unknown command '{other}'")),
        }
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

    fn render_view(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn status_text(_: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn render_template(_: String, _: TemplateContext) -> Result<TemplateFile, String> {
        Err("unused".into())
    }

    fn on_save(_path: String, document: String) -> Result<Option<String>, String> {
        Ok(save_hook(&document))
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_skips_fences_and_indents() {
        let doc = "# Top\n\n```md\n# not a heading\n```\n\n## Child Section\n";
        let toc = build_toc(doc);
        assert_eq!(toc, "- [Top](#top)\n  - [Child Section](#child-section)\n");
    }

    #[test]
    fn update_replaces_marker_content() {
        let doc = "# A\n<!-- toc -->\nold\n<!-- /toc -->\ntail\n";
        let out = update_between_markers(doc).unwrap();
        assert!(out.contains("- [A](#a)"));
        assert!(!out.contains("old"));
        assert!(out.contains("<!-- /toc -->\ntail"));
    }

    #[test]
    fn update_without_markers_errs_with_guidance() {
        let e = update_between_markers("# A\n").unwrap_err();
        assert!(e.contains("<!-- toc -->"), "{e}");
    }

    #[test]
    fn save_hook_refreshes_markers_or_stays_silent() {
        let doc = "# A\n<!-- toc -->\nstale\n<!-- /toc -->\n";
        let out = save_hook(doc).expect("markers present → Some");
        assert!(out.contains("- [A](#a)"), "{out}");
        assert!(!out.contains("stale"));
        assert_eq!(save_hook("# A\nno markers\n"), None);
        // already fresh → None (no churn on every save)
        assert_eq!(save_hook(&out), None);
    }
}

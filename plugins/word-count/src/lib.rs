//! Status widget: word count + reading time for the active document.

wit_bindgen::generate!({ path: "../wit-v4", world: "extension" });

use supermd::extension::types as t;

/// Words outside fenced code, at 200 wpm (minimum 1 minute).
pub fn word_stats(document: &str) -> String {
    let mut in_fence = false;
    let mut words = 0usize;
    for line in document.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            words += line.split_whitespace().count();
        }
    }
    let minutes = ((words + 199) / 200).max(1);
    format!("{words} words · {minutes} min read")
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

    fn render_view(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn status_text(document: String) -> Result<String, String> {
        Ok(word_stats(&document))
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
    use super::word_stats;

    #[test]
    fn counts_words_outside_fences() {
        assert_eq!(word_stats("one two three"), "3 words · 1 min read");
        assert_eq!(word_stats("a b\n```\ncode words here\n```\nc"), "3 words · 1 min read");
        assert_eq!(word_stats(""), "0 words · 1 min read");
        let long = "w ".repeat(401);
        assert!(word_stats(&long).starts_with("401 words · 3 min"), "{}", word_stats(&long));
    }
}

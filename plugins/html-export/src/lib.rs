//! Exporter: the current document as one standalone HTML file with
//! the active theme inlined as CSS. No external assets — the file
//! renders identically offline.

wit_bindgen::generate!({ path: "../wit-v3", world: "extension" });

pub use supermd::extension::types as t;

pub fn render_html(markdown: &str, theme: &t::Theme) -> String {
    let parser = pulldown_cmark::Parser::new_ext(
        markdown,
        pulldown_cmark::Options::ENABLE_TABLES
            | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
            | pulldown_cmark::Options::ENABLE_TASKLISTS,
    );
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, parser);
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <style>\
         body{{background:{bg};color:{text};font-family:\"{font}\",sans-serif;\
         max-width:44rem;margin:2rem auto;padding:0 1rem;line-height:1.6}}\
         a{{color:{primary}}}\
         code,pre{{background:{surface};border:1px solid {border};border-radius:4px}}\
         code{{padding:.1em .3em}}pre{{padding:.8em;overflow-x:auto}}\
         pre code{{border:0;padding:0;background:none}}\
         blockquote{{border-left:3px solid {border};margin-left:0;\
         padding-left:1em;color:{muted}}}\
         img{{max-width:100%}}\
         table{{border-collapse:collapse}}\
         td,th{{border:1px solid {border};padding:.3em .6em}}\
         hr{{border:none;border-top:1px solid {border}}}\
         </style></head><body>\n{body}</body></html>\n",
        bg = theme.background,
        text = theme.text,
        font = theme.font_body,
        primary = theme.primary,
        surface = theme.surface,
        border = theme.border,
        muted = theme.muted,
    )
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

    fn export_document(
        document: String,
        format: String,
        theme: t::Theme,
    ) -> Result<Vec<ExportFile>, String> {
        if format != "html" {
            return Err(format!("unknown format {format}"));
        }
        Ok(vec![ExportFile {
            path: "export.html".into(),
            bytes: render_html(&document, &theme).into_bytes(),
        }])
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::{render_html, t};

    #[test]
    fn renders_standalone_themed_html() {
        let theme = t::Theme {
            background: "#101010".into(),
            surface: "#181818".into(),
            primary: "#4a9eff".into(),
            text: "#e0e0e0".into(),
            muted: "#808080".into(),
            border: "#303030".into(),
            font_body: "Helvetica".into(),
            dark: true,
        };
        let html = render_html("# Hello\n\nworld *em*\n\n- [ ] task\n", &theme);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("<em>em</em>"));
        assert!(html.contains("#101010"), "theme background inlined");
        assert!(html.contains("Helvetica"));
        assert!(!html.contains("src=\"http"), "no external assets");
    }
}

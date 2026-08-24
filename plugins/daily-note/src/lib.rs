//! Template plugin: "New: Daily Note" creates journal/<date>.md.
//! Idempotent by host contract — if today's note exists it just opens.

wit_bindgen::generate!({ path: "../wit-v4", world: "extension" });

use supermd::extension::types as t;

/// Pure template body, decoupled from wit types for testability.
pub struct Ctx {
    pub date: String,
    pub weekday: String,
}

pub fn daily(ctx: &Ctx) -> (String, String) {
    (
        format!("journal/{}.md", ctx.date),
        format!(
            "# {}, {}\n\n## Today\n\n- [ ] \n\n## Notes\n\n",
            ctx.weekday, ctx.date
        ),
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

    fn export_document(_: String, _: String, _: t::Theme) -> Result<Vec<ExportFile>, String> {
        Err("unused".into())
    }

    fn render_view(_: String, _: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn status_text(_: String) -> Result<String, String> {
        Err("unused".into())
    }

    fn render_template(id: String, context: TemplateContext) -> Result<TemplateFile, String> {
        if id != "daily" {
            return Err(format!("unknown template {id}"));
        }
        let (filename, content) = daily(&Ctx {
            date: context.date,
            weekday: context.weekday,
        });
        Ok(TemplateFile { filename, content })
    }

    fn on_save(_: String, _: String) -> Result<Option<String>, String> {
        Ok(None)
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    #[test]
    fn renders_dated_note() {
        let f = super::daily(&super::Ctx {
            date: "2026-08-24".into(),
            weekday: "Monday".into(),
        });
        assert_eq!(f.0, "journal/2026-08-24.md");
        assert!(f.1.starts_with("# Monday, 2026-08-24\n"), "{}", f.1);
        assert!(f.1.contains("- [ ] "), "{}", f.1);
    }
}

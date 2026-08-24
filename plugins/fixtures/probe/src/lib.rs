//! Test fixture for the 0.4 surfaces: deterministic viewer, widget,
//! template, and save-hook responses the host test suite asserts on.

wit_bindgen::generate!({ path: "../../wit-v4", world: "extension" });

use supermd::extension::types as t;

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

    fn render_view(filename: String, content: String) -> Result<String, String> {
        if content.contains("fail") {
            return Err("cannot view".into());
        }
        Ok(format!("# view:{filename}\n\n{content}\n"))
    }

    fn status_text(document: String) -> Result<String, String> {
        Ok(format!("status:{}", document.len()))
    }

    fn render_template(id: String, context: TemplateContext) -> Result<TemplateFile, String> {
        Ok(TemplateFile {
            filename: format!("from-template/{id}-{}.md", context.date),
            content: format!(
                "# {id} on {} ({})\nws={}\n",
                context.date, context.weekday, context.workspace
            ),
        })
    }

    fn on_save(_path: String, document: String) -> Result<Option<String>, String> {
        if document.contains("hookme") {
            Ok(Some(format!("{document}\n<!-- saved -->\n")))
        } else {
            Ok(None)
        }
    }
}

export!(Plugin);

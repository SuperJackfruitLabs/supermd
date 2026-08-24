wit_bindgen::generate!({ path: "../../wit", world: "extension" });
use supermd::extension::types as t;
struct Plugin;
impl Guest for Plugin {
    fn render_block(lang: String, source: String, _theme: t::Theme) -> Result<String, String> {
        Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><desc>{lang}:{source}</desc></svg>"
        ))
    }
    fn run_command(id: String, _input: t::CommandInput) -> Result<t::CommandOutput, String> {
        Ok(t::CommandOutput::InsertAtCursor(format!("echo:{id}")))
    }
}
export!(Plugin);

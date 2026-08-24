wit_bindgen::generate!({ path: "../../wit-v2", world: "extension" });
use supermd::extension::types as t;
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
        // "escape" in the input probes the preopen boundary.
        let path = if document.contains("escape") {
            "/workspace/../outside.txt"
        } else {
            "/workspace/probe.txt"
        };
        std::fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))
    }
    fn process_paste(_t: String) -> Result<Option<String>, String> {
        Ok(None)
    }
}
export!(Plugin);

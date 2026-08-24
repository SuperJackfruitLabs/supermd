wit_bindgen::generate!({ path: "../../wit", world: "extension" });
use supermd::extension::types as t;
struct Plugin;
impl Guest for Plugin {
    fn render_block(_l: String, _s: String, _t: t::Theme) -> Result<String, String> {
        loop { std::hint::spin_loop(); }
    }
    fn run_command(_i: String, _in: t::CommandInput) -> Result<t::CommandOutput, String> {
        loop { std::hint::spin_loop(); }
    }
}
export!(Plugin);

//! SuperMD plugin template. Implement `render_block` (fence → SVG)
//! and/or `run_command` (palette command). Build with:
//!   cargo build --release --target wasm32-wasip2
//! then copy target/wasm32-wasip2/release/*.wasm next to plugin.toml
//! in ~/.supermd/plugins/<your-plugin>/.

wit_bindgen::generate!({
    path: "../wit",
    world: "extension",
});

use supermd::extension::types as t;

struct Plugin;

impl Guest for Plugin {
    fn render_block(_lang: String, _source: String, _theme: t::Theme) -> Result<String, String> {
        Err("this plugin does not render blocks".to_string())
    }

    fn run_command(_id: String, _input: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("this plugin has no commands".to_string())
    }
}

export!(Plugin);

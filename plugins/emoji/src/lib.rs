//! Emoji shortcodes as inline replacements: :tada: → 🎉

wit_bindgen::generate!({ path: "../wit-v2", world: "extension" });
use supermd::extension::types as t;

mod table;

pub fn lookup(matched: &str) -> Option<&'static str> {
    let name = matched.strip_prefix(':')?.strip_suffix(':')?;
    table::TABLE
        .binary_search_by_key(&name, |(alias, _)| alias)
        .ok()
        .map(|ix| table::TABLE[ix].1)
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_l: String, _s: String, _t: t::Theme) -> Result<String, String> {
        Err("no blocks".into())
    }
    fn run_command(_i: String, _in: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("no commands".into())
    }
    fn render_inline(_pattern_id: String, matched: String) -> Result<String, String> {
        lookup(&matched)
            .map(str::to_string)
            .ok_or_else(|| format!("unknown shortcode {matched}"))
    }
    fn format_document(_d: String) -> Result<String, String> {
        Err("no formatter".into())
    }
    fn process_paste(_t: String) -> Result<Option<String>, String> {
        Ok(None)
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    #[test]
    fn known_shortcode_resolves() {
        assert_eq!(super::lookup(":tada:"), Some("🎉"));
    }
    #[test]
    fn unknown_stays_raw() {
        assert_eq!(super::lookup(":definitely-not-real:"), None);
    }
}

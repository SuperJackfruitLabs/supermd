//! Graphviz DOT block renderer on the pure-Rust layout-rs engine.

wit_bindgen::generate!({ path: "../wit", world: "extension" });
use supermd::extension::types as t;

use layout::backends::svg::SVGWriter;
use layout::gv::{DotParser, GraphBuilder};

/// DOT source → untinted SVG.
pub fn render(source: &str) -> Result<String, String> {
    let mut parser = DotParser::new(source);
    let graph = parser.process().map_err(|e| format!("dot parse: {e}"))?;
    let mut builder = GraphBuilder::new();
    builder.visit_graph(&graph);
    let mut visual = builder.get();
    let mut writer = SVGWriter::new();
    visual.do_it(false, false, false, &mut writer);
    Ok(writer.finalize())
}

/// Recolor layout-rs's fixed palette (8-digit hex) to the active theme.
pub fn themed(svg: &str, theme: &t::Theme) -> String {
    svg.replace("fill=\"#ffffffff\"", &format!("fill=\"{}\"", theme.surface))
        .replace("fill=\"#000000ff\"", &format!("fill=\"{}\"", theme.text))
        .replace("stroke=\"#000000ff\"", &format!("stroke=\"{}\"", theme.muted))
        .replace(
            "<svg ",
            &format!("<svg style=\"background-color:{}\" ", theme.background),
        )
        // Labels use an undefined CSS class and fall back to black;
        // stamp the theme text color on every text element instead.
        .replace("<text ", &format!("<text fill=\"{}\" ", theme.text))
        .replace("<text>", &format!("<text fill=\"{}\">", theme.text))
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_lang: String, source: String, theme: t::Theme) -> Result<String, String> {
        render(&source).map(|svg| themed(&svg, &theme))
    }

    fn run_command(_id: String, _input: t::CommandInput) -> Result<t::CommandOutput, String> {
        Err("dot has no commands".to_string())
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digraph_renders_with_labels() {
        let svg = render("digraph { a -> b; a [label=\"Start\"]; }").unwrap();
        assert!(svg.contains("Start"), "{}", &svg[..300.min(svg.len())]);
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn bad_dot_reports_error() {
        assert!(render("digraph { -> -> }").is_err());
    }

    #[test]
    fn theming_recolors_defaults() {
        let theme = t::Theme {
            background: "#211f1a".into(),
            surface: "#2b2822".into(),
            primary: "#e5a63b".into(),
            text: "#d9d4c8".into(),
            muted: "#8f897a".into(),
            border: "#383428".into(),
            font_body: "Helvetica".into(),
            dark: true,
        };
        let svg = themed(&render("digraph { a -> b; }").unwrap(), &theme);
        assert!(svg.contains("#211f1a"));
        assert!(!svg.contains("#ffffffff"), "white fills must be themed");
        assert!(!svg.contains("stroke=\"#000000ff\""), "black strokes must be themed");
        // every text element (not textPath) carries the theme text fill
        for (i, _) in svg.match_indices("<text").collect::<Vec<_>>() {
            let next = svg.as_bytes()[i + 5];
            if next != b' ' && next != b'>' {
                continue; // <textPath>
            }
            assert!(svg[i..i + 40].contains("fill=\"#d9d4c8\""), "unthemed text at {i}");
        }
    }
}



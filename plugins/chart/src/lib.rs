//! ```chart fences rendered as themed SVG bar/line charts.
//!
//! ```chart
//! type: bar          # bar (default) | line
//! title: Sales
//! Jan: 4
//! Feb: 7.5
//! ```

wit_bindgen::generate!({ path: "../wit-v4", world: "extension" });

use supermd::extension::types as t;

#[derive(Debug, PartialEq)]
pub struct Chart {
    pub kind: Kind,
    pub title: Option<String>,
    pub points: Vec<(String, f64)>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Kind {
    Bar,
    Line,
}

/// Parse the fence body: optional `type:`/`title:` headers, then one
/// `label: value` per line.
pub fn parse_chart(source: &str) -> Result<Chart, String> {
    let mut kind = Kind::Bar;
    let mut title = None;
    let mut points = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("expected `label: value`, got `{line}`"));
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "type" => {
                kind = match value {
                    "bar" => Kind::Bar,
                    "line" => Kind::Line,
                    other => return Err(format!("unknown chart type `{other}`")),
                }
            }
            "title" => title = Some(value.to_string()),
            label => {
                let n: f64 = value
                    .parse()
                    .map_err(|_| format!("`{label}` needs a numeric value, got `{value}`"))?;
                points.push((label.to_string(), n));
            }
        }
    }
    if points.is_empty() {
        return Err("a chart needs at least one `label: value` line".to_string());
    }
    if points.len() > 60 {
        return Err("too many data points (max 60)".to_string());
    }
    Ok(Chart { kind, title, points })
}

const W: f64 = 560.0;
const H: f64 = 300.0;
const PAD_L: f64 = 46.0;
const PAD_B: f64 = 34.0;
const PAD_T: f64 = 30.0;
const PAD_R: f64 = 14.0;

/// Chart → themed SVG.
pub fn render_chart(chart: &Chart, theme: &t::Theme) -> String {
    let plot_w = W - PAD_L - PAD_R;
    let plot_h = H - PAD_T - PAD_B;
    let max = chart.points.iter().map(|(_, v)| *v).fold(f64::MIN, f64::max).max(0.0);
    let min = chart.points.iter().map(|(_, v)| *v).fold(f64::MAX, f64::min).min(0.0);
    let span = (max - min).max(1e-9);
    let y_of = |v: f64| PAD_T + plot_h * (1.0 - (v - min) / span);
    let n = chart.points.len() as f64;

    let mut body = String::new();
    // axis + baseline
    body.push_str(&format!(
        "<line x1='{PAD_L}' y1='{PAD_T}' x2='{PAD_L}' y2='{}' stroke='{}' stroke-width='1'/>",
        PAD_T + plot_h,
        theme.border
    ));
    body.push_str(&format!(
        "<line x1='{PAD_L}' y1='{y0}' x2='{}' y2='{y0}' stroke='{}' stroke-width='1'/>",
        W - PAD_R,
        theme.border,
        y0 = y_of(0.0),
    ));
    // min/max labels
    for (v, y) in [(max, y_of(max)), (min, y_of(min))] {
        body.push_str(&format!(
            "<text x='{}' y='{}' font-size='10' fill='{}' text-anchor='end'>{}</text>",
            PAD_L - 6.0,
            y + 3.0,
            theme.muted,
            trim_num(v)
        ));
    }
    match chart.kind {
        Kind::Bar => {
            let slot = plot_w / n;
            let bar_w = (slot * 0.66).min(48.0);
            for (i, (_, v)) in chart.points.iter().enumerate() {
                let x = PAD_L + slot * i as f64 + (slot - bar_w) / 2.0;
                let (top, bottom) = if *v >= 0.0 { (y_of(*v), y_of(0.0)) } else { (y_of(0.0), y_of(*v)) };
                body.push_str(&format!(
                    "<rect x='{x:.1}' y='{top:.1}' width='{bar_w:.1}' height='{:.1}' rx='3' fill='{}'/>",
                    (bottom - top).max(1.0),
                    theme.primary
                ));
            }
        }
        Kind::Line => {
            let step = if n > 1.0 { plot_w / (n - 1.0) } else { 0.0 };
            let pts: Vec<String> = chart
                .points
                .iter()
                .enumerate()
                .map(|(i, (_, v))| format!("{:.1},{:.1}", PAD_L + step * i as f64, y_of(*v)))
                .collect();
            body.push_str(&format!(
                "<polyline points='{}' fill='none' stroke='{}' stroke-width='2'/>",
                pts.join(" "),
                theme.primary
            ));
            for p in &pts {
                let (x, y) = p.split_once(',').unwrap();
                body.push_str(&format!(
                    "<circle cx='{x}' cy='{y}' r='3' fill='{}'/>",
                    theme.primary
                ));
            }
        }
    }
    // x labels (skip some when crowded)
    let every = (chart.points.len() / 12).max(1);
    let slot = plot_w / n;
    for (i, (label, _)) in chart.points.iter().enumerate() {
        if i % every != 0 {
            continue;
        }
        let x = match chart.kind {
            Kind::Bar => PAD_L + slot * (i as f64 + 0.5),
            Kind::Line => {
                let step = if n > 1.0 { plot_w / (n - 1.0) } else { 0.0 };
                PAD_L + step * i as f64
            }
        };
        body.push_str(&format!(
            "<text x='{x:.1}' y='{}' font-size='10' fill='{}' text-anchor='middle'>{}</text>",
            H - PAD_B + 16.0,
            theme.muted,
            escape(label)
        ));
    }
    if let Some(title) = &chart.title {
        body.push_str(&format!(
            "<text x='{}' y='18' font-size='13' font-weight='600' fill='{}' text-anchor='middle'>{}</text>",
            W / 2.0,
            theme.text,
            escape(title)
        ));
    }
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{W}' height='{H}' viewBox='0 0 {W} {H}' \
         font-family='{}' style='background-color:{}'>{body}</svg>",
        escape(&theme.font_body),
        theme.background
    )
}

fn trim_num(n: f64) -> String {
    let s = format!("{n:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

struct Plugin;

impl Guest for Plugin {
    fn render_block(_lang: String, source: String, theme: t::Theme) -> Result<String, String> {
        parse_chart(&source).map(|c| render_chart(&c, &theme))
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
    use super::*;

    fn theme() -> t::Theme {
        t::Theme {
            background: "#111".into(),
            surface: "#222".into(),
            primary: "#e5a63b".into(),
            text: "#eee".into(),
            muted: "#888".into(),
            border: "#333".into(),
            font_body: "Helvetica".into(),
            dark: true,
        }
    }

    #[test]
    fn parses_headers_and_points() {
        let c = parse_chart("type: line\ntitle: Growth\nQ1: 10\nQ2: 12.5\n").unwrap();
        assert_eq!(c.kind, Kind::Line);
        assert_eq!(c.title.as_deref(), Some("Growth"));
        assert_eq!(c.points, vec![("Q1".into(), 10.0), ("Q2".into(), 12.5)]);
        // defaults: bar, no title
        let c = parse_chart("A: 1\nB: 2\n").unwrap();
        assert_eq!(c.kind, Kind::Bar);
        assert!(c.title.is_none());
    }

    #[test]
    fn rejects_bad_input_with_guidance() {
        assert!(parse_chart("").is_err());
        assert!(parse_chart("type: pie\nA: 1\n").is_err());
        assert!(parse_chart("A: lots\n").is_err());
        assert!(parse_chart("no separator line\n").is_err());
    }

    #[test]
    fn bar_svg_carries_theme_and_bars() {
        let c = parse_chart("title: T\nA: 4\nB: -2\nC: 7\n").unwrap();
        let svg = render_chart(&c, &theme());
        assert!(svg.starts_with("<svg"));
        assert_eq!(svg.matches("<rect").count(), 3);
        assert!(svg.contains("#e5a63b"), "primary used");
        assert!(svg.contains(">T<"), "title rendered");
        assert!(svg.contains(">A<") && svg.contains(">C<"), "labels rendered");
    }

    #[test]
    fn line_svg_has_polyline_and_points() {
        let c = parse_chart("type: line\nA: 1\nB: 2\nC: 3\n").unwrap();
        let svg = render_chart(&c, &theme());
        assert_eq!(svg.matches("<circle").count(), 3);
        assert!(svg.contains("<polyline"));
    }

    #[test]
    fn escapes_labels() {
        let c = parse_chart("<b>: 1\nok: 2\n").unwrap();
        let svg = render_chart(&c, &theme());
        assert!(!svg.contains("<b>"), "{svg}");
        assert!(svg.contains("&lt;b&gt;"));
    }
}

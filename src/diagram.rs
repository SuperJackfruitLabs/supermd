//! Diagram engine: mermaid source → themed SVG (merman) → PNG raster
//! (resvg) → cached gpui image. All rendering happens on the
//! background executor; the UI only reads the cache.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use crate::theme::Theme;

// ── theming ────────────────────────────────────────────────────────────

/// The palette handed to mermaid's `base` theme via an init directive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiagramTheme {
    pub background: String, // #rrggbb
    pub primary: String,
    pub text: String,
    pub muted: String,
    pub border: String,
    pub font_body: String,
    pub dark: bool,
}

fn hex(color: gpui::Hsla) -> String {
    let rgba = gpui::Rgba::from(color);
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8
    )
}

impl DiagramTheme {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            background: hex(t.bg),
            primary: hex(t.accent),
            text: hex(t.fg),
            muted: hex(t.fg_muted),
            border: hex(t.border),
            font_body: t.body_family.to_string(),
            dark: t.is_dark,
        }
    }

    pub fn default_light() -> Self {
        Self::from_theme(&Theme::light())
    }

    pub fn default_dark() -> Self {
        Self::from_theme(&Theme::dark())
    }

    pub fn fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.background.hash(&mut h);
        self.primary.hash(&mut h);
        self.text.hash(&mut h);
        self.muted.hash(&mut h);
        self.border.hash(&mut h);
        self.font_body.hash(&mut h);
        self.dark.hash(&mut h);
        h.finish()
    }

    /// Mermaid init directive carrying the palette.
    fn init_directive(&self) -> String {
        format!(
            concat!(
                "%%{{init: {{\"theme\":\"base\",\"themeVariables\":{{",
                "\"background\":\"{bg}\",\"primaryColor\":\"{bg}\",",
                "\"primaryTextColor\":\"{text}\",\"primaryBorderColor\":\"{primary}\",",
                "\"lineColor\":\"{muted}\",\"textColor\":\"{text}\",",
                "\"secondaryColor\":\"{bg}\",\"tertiaryColor\":\"{bg}\",",
                "\"fontFamily\":\"{font}\",\"darkMode\":{dark}",
                "}}}}}}%%\n"
            ),
            bg = self.background,
            text = self.text,
            primary = self.primary,
            muted = self.muted,
            font = self.font_body,
            dark = self.dark,
        )
    }
}

// ── rendering ──────────────────────────────────────────────────────────

/// Mermaid source → themed standalone SVG.
pub fn to_svg(source: &str, theme: &DiagramTheme) -> Result<String, String> {
    let themed = format!("{}{}", theme.init_directive(), source);
    merman::render_svg(&themed).map_err(|e| e.to_string())
}

fn fontdb() -> &'static resvg::usvg::fontdb::Database {
    static DB: std::sync::OnceLock<resvg::usvg::fontdb::Database> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// SVG → (PNG bytes, pixel width, pixel height) at `scale`.
pub fn rasterize(svg: &str, scale: f32) -> Result<(Vec<u8>, u32, u32), String> {
    let mut opts = resvg::usvg::Options::default();
    opts.fontdb = std::sync::Arc::new(fontdb().clone());
    let tree = resvg::usvg::Tree::from_str(svg, &opts).map_err(|e| e.to_string())?;
    let size = tree.size();
    let w = (size.width() * scale).ceil() as u32;
    let h = (size.height() * scale).ceil() as u32;
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(format!("diagram size out of range ({w}x{h})"));
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| "pixmap alloc failed".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    Ok((png, w, h))
}

// ── cache ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DiagramKey {
    pub source_hash: u64,
    pub theme_fingerprint: u64,
    pub width_bucket: u32,
}

impl DiagramKey {
    pub fn bucket(width: f32) -> u32 {
        ((width / 64.0).round() as u32) * 64
    }
}

#[derive(Clone)]
pub enum DiagramState {
    Pending,
    Ready(Arc<gpui::Image>),
    Failed(String),
}

const CACHE_CAP: usize = 128;

#[derive(Default)]
pub struct DiagramCache {
    map: HashMap<DiagramKey, DiagramState>,
    order: VecDeque<DiagramKey>,
}

impl gpui::Global for DiagramCache {}

impl DiagramCache {
    pub fn insert(&mut self, key: DiagramKey, state: DiagramState) {
        if !self.map.contains_key(&key) {
            self.order.push_back(key.clone());
            while self.order.len() > CACHE_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
        self.map.insert(key, state);
    }

    pub fn get(&self, key: &DiagramKey) -> Option<&DiagramState> {
        self.map.get(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Cache lookup for the UI: on a miss, inserts Pending and spawns the
/// background render, refreshing the window when the result lands.
pub fn diagram_state(source: &str, width: f32, cx: &mut gpui::App) -> DiagramState {
    let theme = DiagramTheme::from_theme(&crate::theme::theme(cx));
    let key = DiagramKey {
        source_hash: hash_str(source),
        theme_fingerprint: theme.fingerprint(),
        width_bucket: DiagramKey::bucket(width),
    };
    if cx.try_global::<DiagramCache>().is_none() {
        cx.set_global(DiagramCache::default());
    }
    if let Some(state) = cx.global::<DiagramCache>().get(&key) {
        return state.clone();
    }
    cx.global_mut::<DiagramCache>().insert(key.clone(), DiagramState::Pending);

    let source = source.to_string();
    let render = cx.background_executor().spawn(async move {
        to_svg(&source, &theme).and_then(|svg| rasterize(&svg, 2.0))
    });
    cx.spawn(async move |cx| {
        let state = match render.await {
            Ok((png, _, _)) => {
                DiagramState::Ready(Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)))
            }
            Err(e) => DiagramState::Failed(e),
        };
        cx.update(|cx| {
            cx.global_mut::<DiagramCache>().insert(key, state);
            cx.refresh_windows();
        })
        .ok();
    })
    .detach();
    DiagramState::Pending
}

/// Convenience for widgets: line ranges of a fence body (used by tests
/// and the projector to slice the body out of the fence claim).
pub fn body_of_fence(text: &str, fence: &Range<usize>) -> String {
    text.get(fence.clone()).unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowchart_renders_to_svg_with_labels() {
        let t = DiagramTheme::default_light();
        let svg = to_svg("flowchart LR\n  a[Start] --> b[End]\n", &t).unwrap();
        assert!(svg.contains("Start") && svg.contains("End"), "{}", &svg[..200.min(svg.len())]);
    }

    #[test]
    fn bad_source_reports_error() {
        let t = DiagramTheme::default_light();
        assert!(to_svg("not_a_diagram_type_xyz\n  a --> b", &t).is_err());
    }

    #[test]
    fn rasterize_produces_scaled_png() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="#c9821c"/></svg>"##;
        let (png, w, h) = rasterize(svg, 2.0).unwrap();
        assert_eq!((w, h), (200, 100));
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn theme_fingerprint_tracks_fields() {
        let a = DiagramTheme::default_light();
        let b = DiagramTheme::default_dark();
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint(), DiagramTheme::default_light().fingerprint());
    }

    #[test]
    fn cache_evicts_oldest_beyond_cap() {
        let mut c = DiagramCache::default();
        for i in 0..130u64 {
            c.insert(
                DiagramKey { source_hash: i, theme_fingerprint: 0, width_bucket: 704 },
                DiagramState::Pending,
            );
        }
        assert!(c.len() <= 128);
        assert!(c
            .get(&DiagramKey { source_hash: 0, theme_fingerprint: 0, width_bucket: 704 })
            .is_none());
        assert!(c
            .get(&DiagramKey { source_hash: 129, theme_fingerprint: 0, width_bucket: 704 })
            .is_some());
    }

    #[test]
    fn width_buckets_round_to_64() {
        assert_eq!(DiagramKey::bucket(700.0), 704);
        assert_eq!(DiagramKey::bucket(650.0), 640);
    }
}

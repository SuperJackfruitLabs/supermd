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
    pub background: String, // #rrggbb — the canvas
    pub surface: String,    // node fill (raised, like code_bg)
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
            surface: hex(t.code_bg),
            primary: hex(t.accent),
            text: hex(t.fg),
            muted: hex(t.fg_muted),
            border: hex(t.border),
            // ".SystemUIFont" is a private name resvg's fontdb cannot
            // resolve; substitute the closest real face.
            font_body: if t.body_family.starts_with('.') {
                "Helvetica Neue, Helvetica, Arial, sans-serif".to_string()
            } else {
                t.body_family.to_string()
            },
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
        self.surface.hash(&mut h);
        self.primary.hash(&mut h);
        self.text.hash(&mut h);
        self.muted.hash(&mut h);
        self.border.hash(&mut h);
        self.font_body.hash(&mut h);
        self.dark.hash(&mut h);
        h.finish()
    }

    /// Mermaid site config carrying the palette. htmlLabels is forced
    /// off — resvg cannot rasterize foreignObject HTML labels.
    fn site_config(&self) -> serde_json::Value {
        serde_json::json!({
            "theme": "base",
            "htmlLabels": false,
            "flowchart": { "htmlLabels": false },
            "fontFamily": self.font_body,
            "themeVariables": {
                "background": self.background,
                "mainBkg": self.surface,
                "primaryColor": self.surface,
                "primaryTextColor": self.text,
                "primaryBorderColor": self.primary,
                "secondaryColor": self.surface,
                "secondaryTextColor": self.text,
                "tertiaryColor": self.background,
                "tertiaryTextColor": self.text,
                "lineColor": self.muted,
                "textColor": self.text,
                "nodeBorder": self.primary,
                "clusterBkg": self.background,
                "clusterBorder": self.border,
                "actorBkg": self.surface,
                "actorBorder": self.primary,
                "actorTextColor": self.text,
                "actorLineColor": self.muted,
                "signalColor": self.text,
                "signalTextColor": self.text,
                "noteBkgColor": self.surface,
                "noteTextColor": self.text,
                "noteBorderColor": self.border,
                "labelBoxBkgColor": self.surface,
                "labelTextColor": self.text,
                "edgeLabelBackground": self.background,
                "fontFamily": self.font_body,
                "darkMode": self.dark,
            },
        })
    }
}

// ── rendering ──────────────────────────────────────────────────────────

/// Mermaid source → themed standalone SVG, through merman's
/// resvg-safe pipeline (no foreignObject, themed root background).
pub fn to_svg(source: &str, theme: &DiagramTheme) -> Result<String, String> {
    let pipeline = merman::svg::SvgOutputPolicy {
        preset: merman::svg::SvgPipelinePreset::ResvgSafe,
        css_override_policy: merman::svg::CssOverridePolicy::StripExistingImportant,
        root_background_color: Some(theme.background.clone()),
        drop_native_duplicate_fallbacks: false,
        scoped_css: None,
    }
    .pipeline();
    let renderer = merman::svg::HeadlessRenderer::new()
        .with_site_config(merman::MermaidConfig::from_value(theme.site_config()))
        .with_svg_pipeline(pipeline);
    match renderer.render_svg_sync(source) {
        Ok(Some(svg)) => Ok(svg),
        Ok(None) => Err("no mermaid diagram detected".to_string()),
        Err(e) => Err(e.to_string()),
    }
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

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
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

/// Like `diagram_state`, but the SVG comes from a wasm plugin's
/// render-block export instead of merman. Same cache, same states —
/// the key folds in the plugin identity so upgrades re-render.
pub fn plugin_diagram_state(
    plugin: &str,
    version: &str,
    lang: &str,
    source: &str,
    width: f32,
    cx: &mut gpui::App,
) -> DiagramState {
    let theme = DiagramTheme::from_theme(&crate::theme::theme(cx));
    let key = DiagramKey {
        source_hash: hash_str(&format!("{plugin}@{version}:{lang}\u{0}{source}")),
        theme_fingerprint: theme.fingerprint(),
        width_bucket: DiagramKey::bucket(width),
    };
    if cx.try_global::<DiagramCache>().is_none() {
        cx.set_global(DiagramCache::default());
    }
    if let Some(state) = cx.global::<DiagramCache>().get(&key) {
        return state.clone();
    }
    let Some(host) = cx
        .try_global::<crate::extensions::ExtensionState>()
        .map(|s| s.0.clone())
    else {
        return DiagramState::Failed("extensions not initialized".to_string());
    };
    cx.global_mut::<DiagramCache>().insert(key.clone(), DiagramState::Pending);

    let (plugin, lang, source) = (plugin.to_string(), lang.to_string(), source.to_string());
    let render = cx.background_executor().spawn(async move {
        let svg = host
            .lock()
            .unwrap()
            .render_block(&plugin, &lang, &source, &theme)?;
        rasterize(&svg, 2.0)
    });
    cx.spawn(async move |cx| {
        let state = match render.await {
            Ok((png, _, _)) => DiagramState::Ready(Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                png,
            ))),
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
        // Theme must reach the SVG: canvas background + no HTML labels
        // (resvg cannot draw foreignObject).
        assert!(svg.contains(&t.background), "canvas not themed");
        assert!(!svg.contains("foreignObject"), "HTML labels leaked through");
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

    #[test]
    fn real_font_family_passes_through_untranslated() {
        let mut t = Theme::light();
        t.body_family = "Georgia".into();
        let dt = DiagramTheme::from_theme(&t);
        assert_eq!(dt.font_body, "Georgia");
        // and it lands in the mermaid site config verbatim
        assert_eq!(dt.site_config()["fontFamily"], "Georgia");
    }

    #[test]
    fn empty_source_reports_detection_error() {
        let t = DiagramTheme::default_light();
        let err = to_svg("", &t).unwrap_err();
        assert!(err.contains("No diagram type detected"), "unexpected error: {err}");
    }

    #[test]
    fn rasterize_rejects_out_of_range_sizes() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="#c9821c"/></svg>"##;
        let err = rasterize(svg, 100.0).unwrap_err();
        assert!(err.contains("out of range"), "unexpected error: {err}");
        assert!(err.contains("10000x5000"), "size missing from error: {err}");
    }

    #[test]
    fn reinserting_key_updates_state_without_duplicating_order() {
        let mut c = DiagramCache::default();
        let key = DiagramKey { source_hash: 7, theme_fingerprint: 0, width_bucket: 704 };
        c.insert(key.clone(), DiagramState::Pending);
        c.insert(key.clone(), DiagramState::Failed("boom".into()));
        assert_eq!(c.len(), 1);
        let Some(DiagramState::Failed(msg)) = c.get(&key) else { panic!("expected Failed") };
        assert_eq!(msg, "boom");
        // The re-insert must not have queued a second eviction entry:
        // fill up to exactly CACHE_CAP distinct keys — nothing evicts,
        // so the original key must survive.
        for i in 100..227u64 {
            c.insert(
                DiagramKey { source_hash: i, theme_fingerprint: 0, width_bucket: 704 },
                DiagramState::Pending,
            );
        }
        assert_eq!(c.len(), 128);
        assert!(c.get(&key).is_some(), "duplicate order entry caused premature eviction");
    }

    #[test]
    fn hash_str_is_deterministic_and_input_sensitive() {
        assert_eq!(hash_str("flowchart LR"), hash_str("flowchart LR"));
        assert_ne!(hash_str("flowchart LR"), hash_str("flowchart TD"));
    }

    #[test]
    fn body_of_fence_slices_range() {
        let text = "```mermaid\nflowchart LR\n```\n";
        assert_eq!(body_of_fence(text, &(11..24)), "flowchart LR\n");
        // out-of-bounds and non-char-boundary ranges degrade to empty
        assert_eq!(body_of_fence("abc", &(0..99)), "");
        assert_eq!(body_of_fence("é", &(1..2)), "");
    }
}


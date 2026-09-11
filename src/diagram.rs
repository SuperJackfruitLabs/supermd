//! Diagram engine: mermaid source → themed SVG (merman) → PNG raster
//! (resvg) → cached gpui image. All rendering happens on the
//! background executor; the UI only reads the cache.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
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
    /// The workspace root the render ran against, hashed; 0 for a
    /// render that cannot read one (merman, or no host root).
    ///
    /// The cache is process-wide and shared by every window. A plugin
    /// fence renderer holding `workspace-read` reads files named in the
    /// fence body, so the *same* fence body in two vaults is two
    /// different pictures — without the root in the key, window B gets
    /// a cache hit and is shown vault A's contents. It also fixes the
    /// single-window half: `open_path` re-roots the host, and diagrams
    /// rendered against the old root must not survive it.
    pub root_hash: u64,
    pub theme_fingerprint: u64,
    pub width_bucket: u32,
}

impl DiagramKey {
    pub fn bucket(width: f32) -> u32 {
        ((width / 64.0).round() as u32) * 64
    }

    /// Hash of the workspace root a render may read under. `None` (no
    /// folder open, or a renderer that reads nothing) is 0.
    pub fn root(root: Option<&Path>) -> u64 {
        root.map(|r| hash_str(&r.to_string_lossy())).unwrap_or(0)
    }
}

#[derive(Clone)]
pub enum DiagramState {
    Pending,
    /// The rendered image, with the size it should be *drawn* at.
    ///
    /// The PNG is rasterised at `RASTER_SCALE` for crispness, so its
    /// pixel dimensions are that many times larger than the diagram
    /// actually is. Drawing it at its pixel size made every diagram
    /// twice its intended size — a seven-node flowchart needed two
    /// screens.
    Ready { image: Arc<gpui::Image>, width: f32, height: f32 },
    Failed(String),
}

/// Rasterisation factor: enough for a retina display without making the
/// cache enormous. Divide pixel dimensions by this to lay the image out.
pub const RASTER_SCALE: f32 = 2.0;

/// The size to draw a diagram, fitted into `available` width and never
/// enlarged beyond its natural size.
///
/// Returns logical points, aspect preserved. A diagram narrower than
/// the column keeps its own size rather than stretching to fill it.
pub fn fit(pixel_w: u32, pixel_h: u32, available: f32) -> (f32, f32) {
    let (w, h) = (pixel_w as f32 / RASTER_SCALE, pixel_h as f32 / RASTER_SCALE);
    if w <= 0.0 || h <= 0.0 {
        return (0.0, 0.0);
    }
    if w <= available {
        return (w, h);
    }
    let k = available / w;
    (available, (h * k).round())
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
        // merman renders the fence text and nothing else: no host, no
        // preopen, so no workspace to be wrong about.
        root_hash: 0,
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

    let available = width;
    let source = source.to_string();
    let render = cx.background_executor().spawn(async move {
        to_svg(&source, &theme).and_then(|svg| rasterize(&svg, RASTER_SCALE))
    });
    cx.spawn(async move |cx| {
        let state = match render.await {
            Ok((png, w, h)) => {
                let (width, height) = fit(w, h, available);
                DiagramState::Ready {
                    image: Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)),
                    width,
                    height,
                }
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
    host: Option<crate::extensions::HostHandle>,
    root: Option<PathBuf>,
    cx: &mut gpui::App,
) -> DiagramState {
    // The host belongs to the workspace that owns the editor drawing
    // this block, never to the process: its preopen root is that
    // window's folder, and `root` is that same root read off a handle
    // the host shares (never by locking the host on the render path).
    let Some(host) = host else {
        return DiagramState::Failed("extensions not initialized".to_string());
    };
    let theme = DiagramTheme::from_theme(&crate::theme::theme(cx));
    let key = DiagramKey {
        source_hash: hash_str(&format!("{plugin}@{version}:{lang}\u{0}{source}")),
        root_hash: DiagramKey::root(root.as_deref()),
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

    let available = width;
    let (plugin, lang, source) = (plugin.to_string(), lang.to_string(), source.to_string());
    let render = cx.background_executor().spawn(async move {
        let svg = host
            .lock()
            .unwrap()
            .render_block(&plugin, &lang, &source, &theme)?;
        rasterize(&svg, RASTER_SCALE)
    });
    cx.spawn(async move |cx| {
        let state = match render.await {
            Ok((png, w, h)) => {
                let (width, height) = fit(w, h, available);
                DiagramState::Ready {
                    image: Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)),
                    width,
                    height,
                }
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

    /// A diagram is drawn at the size it actually is. The PNG is
    /// rasterised at 2x for crispness, and drawing it at its pixel size
    /// made every diagram twice its intended size — a seven-node
    /// flowchart ran past two screens.
    #[test]
    fn fit_undoes_the_raster_scale() {
        // Comfortably inside the column: natural size, halved.
        assert_eq!(fit(530, 1400, 664.0), (265.0, 700.0));
        // Exactly the column width after halving: unchanged.
        assert_eq!(fit(1328, 664, 664.0), (664.0, 332.0));
    }

    /// Wider than the column: scaled down, aspect kept.
    #[test]
    fn fit_shrinks_a_wide_diagram_and_keeps_its_shape() {
        let (w, h) = fit(2656, 1328, 664.0);
        assert_eq!(w, 664.0);
        assert_eq!(h, 332.0, "half the width means half the height");
        let (w2, h2) = fit(4000, 1000, 664.0);
        assert_eq!(w2, 664.0);
        assert!((h2 - 166.0).abs() <= 1.0, "aspect preserved, got {h2}");
    }

    /// A narrow diagram is never stretched to fill the column.
    #[test]
    fn fit_never_enlarges() {
        assert_eq!(fit(100, 100, 664.0), (50.0, 50.0));
    }

    #[test]
    fn fit_tolerates_a_degenerate_image() {
        assert_eq!(fit(0, 0, 664.0), (0.0, 0.0));
        assert_eq!(fit(100, 0, 664.0), (0.0, 0.0));
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
                DiagramKey { source_hash: i, root_hash: 0, theme_fingerprint: 0, width_bucket: 704 },
                DiagramState::Pending,
            );
        }
        assert!(c.len() <= 128);
        assert!(c
            .get(&DiagramKey { source_hash: 0, root_hash: 0, theme_fingerprint: 0, width_bucket: 704 })
            .is_none());
        assert!(c
            .get(&DiagramKey { source_hash: 129, root_hash: 0, theme_fingerprint: 0, width_bucket: 704 })
            .is_some());
    }

    /// The cache is process-wide; the workspace root is part of the
    /// key. A plugin fence renderer holding `workspace-read` reads
    /// files under that root, so the *same* fence body in two vaults is
    /// two different renders — without the root in the key, the second
    /// window takes a cache hit and is shown the first window's data.
    /// The lookup happens before the host is ever consulted, so the
    /// host alone cannot keep them apart.
    #[gpui::test]
    fn two_vaults_with_the_same_fence_body_are_two_cache_entries(
        cx: &mut gpui::TestAppContext,
    ) {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(crate::theme::Theme::dark())));
            // An empty host: the render fails, but only *after* the key
            // has been computed and the Pending entry inserted, which
            // is the part under test.
            let host: crate::extensions::HostHandle = Arc::new(std::sync::Mutex::new(
                crate::extensions::ExtensionHost::load(Path::new("/nonexistent")),
            ));
            let body = "same fence body";
            let call = |root: &Path, cx: &mut gpui::App| {
                plugin_diagram_state(
                    "renderer",
                    "0.1.0",
                    "demo",
                    body,
                    664.0,
                    Some(host.clone()),
                    Some(root.to_path_buf()),
                    cx,
                )
            };
            call(a.path(), cx);
            assert_eq!(cx.global::<DiagramCache>().len(), 1);
            // Same body, same plugin, same width, different vault.
            call(b.path(), cx);
            assert_eq!(
                cx.global::<DiagramCache>().len(),
                2,
                "vault B must not hit vault A's cached render"
            );
            // And the same vault twice is still one entry.
            call(a.path(), cx);
            assert_eq!(cx.global::<DiagramCache>().len(), 2);
        });
    }

    #[test]
    fn root_hash_separates_roots_and_collapses_none() {
        assert_eq!(DiagramKey::root(None), 0);
        assert_ne!(DiagramKey::root(Some(Path::new("/vault/a"))), 0);
        assert_ne!(
            DiagramKey::root(Some(Path::new("/vault/a"))),
            DiagramKey::root(Some(Path::new("/vault/b")))
        );
        assert_eq!(
            DiagramKey::root(Some(Path::new("/vault/a"))),
            DiagramKey::root(Some(Path::new("/vault/a")))
        );
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
        let key = DiagramKey { source_hash: 7, root_hash: 0, theme_fingerprint: 0, width_bucket: 704 };
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
                DiagramKey { source_hash: i, root_hash: 0, theme_fingerprint: 0, width_bucket: 704 },
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


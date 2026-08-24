//! The projector registry: each block-widget kind (table, image,
//! diagram, …) is a `Projector` that *claims* line ranges during
//! discovery and renders a widget for untouched claims. The reveal
//! rule itself lives in projection.rs — projectors never see the
//! selection.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use gpui::AnyElement;

use super::blocks::{BlockInfo, BlockKind};
use super::projection::line_of_byte;
use crate::theme::Theme;

pub struct Claim {
    /// Source lines the widget consumes.
    pub lines: Range<usize>,
    /// Source byte range for the central touch test.
    pub bytes: Range<usize>,
    /// Projector-specific parsed data, downcast at render time.
    pub payload: Arc<dyn Any + Send + Sync>,
}

pub struct WidgetCtx<'a> {
    pub editor: &'a gpui::Entity<super::Editor>,
    pub item_ix: usize,
    pub lines: Range<usize>,
    pub payload: &'a Arc<dyn Any + Send + Sync>,
    pub theme: &'a Theme,
    pub cx: &'a mut gpui::App,
}

pub trait Projector: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// Pure discovery over the parsed document.
    fn discover(
        &self,
        text: &str,
        blocks: &[BlockInfo],
        lines: &[Range<usize>],
    ) -> Vec<Claim>;
    /// Render the widget for an untouched claim. GPUI-only entry point.
    fn render(&self, ctx: &mut WidgetCtx<'_>) -> AnyElement;
}

/// Registered in priority order; earlier projectors win overlaps.
pub fn projectors() -> &'static [&'static dyn Projector] {
    static REGISTRY: &[&dyn Projector] = &[&TableProjector, &ImageProjector];
    REGISTRY
}

/// Run every projector's discovery; returns (registry index, claim).
pub fn discover_all(
    text: &str,
    blocks: &[BlockInfo],
    lines: &[Range<usize>],
) -> Vec<(usize, Claim)> {
    let mut out = Vec::new();
    for (ix, p) in projectors().iter().enumerate() {
        for claim in p.discover(text, blocks, lines) {
            out.push((ix, claim));
        }
    }
    out
}

// ── table ──────────────────────────────────────────────────────────────

pub struct TablePayload;

pub struct TableProjector;

impl Projector for TableProjector {
    fn name(&self) -> &'static str {
        "table"
    }

    fn discover(
        &self,
        _text: &str,
        blocks: &[BlockInfo],
        lines: &[Range<usize>],
    ) -> Vec<Claim> {
        blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Table))
            .map(|b| {
                let first = line_of_byte(lines, b.range.start);
                let last = line_of_byte(lines, b.range.end.max(b.range.start + 1) - 1);
                Claim {
                    lines: first..last + 1,
                    bytes: b.range.clone(),
                    payload: Arc::new(TablePayload),
                }
            })
            .collect()
    }

    fn render(&self, ctx: &mut WidgetCtx<'_>) -> AnyElement {
        super::render_table(ctx.editor, ctx.item_ix, ctx.lines.clone(), ctx.theme, ctx.cx)
    }
}

// ── image ──────────────────────────────────────────────────────────────

pub struct ImagePayload {
    pub alt: String,
    pub dest: String,
}

pub struct ImageProjector;

impl Projector for ImageProjector {
    fn name(&self) -> &'static str {
        "image"
    }

    fn discover(
        &self,
        _text: &str,
        blocks: &[BlockInfo],
        lines: &[Range<usize>],
    ) -> Vec<Claim> {
        blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Image { alt, dest } => {
                    let first = line_of_byte(lines, b.range.start);
                    Some(Claim {
                        lines: first..first + 1,
                        bytes: b.range.clone(),
                        payload: Arc::new(ImagePayload {
                            alt: alt.clone(),
                            dest: dest.clone(),
                        }),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn render(&self, ctx: &mut WidgetCtx<'_>) -> AnyElement {
        let payload = ctx
            .payload
            .downcast_ref::<ImagePayload>()
            .expect("image projector payload");
        super::render_image(
            ctx.editor,
            ctx.item_ix,
            ctx.lines.start,
            &payload.alt,
            &payload.dest,
            ctx.theme,
            ctx.cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(src: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut start = 0;
        for line in src.split('\n') {
            out.push(start..start + line.len());
            start += line.len() + 1;
        }
        out
    }

    #[test]
    fn table_and_image_discovery_matches_blocks() {
        let src = "a\n\n|h|\n|-|\n|1|\n\n![x](p.png)\n\nb";
        let lines = lines_of(src);
        let blocks = crate::editor::blocks::blocks(src);
        let claims = discover_all(src, &blocks, &lines);
        let tables: Vec<_> = claims.iter().filter(|(p, _)| *p == 0).collect();
        let images: Vec<_> = claims.iter().filter(|(p, _)| *p == 1).collect();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].1.lines, 2..5);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].1.lines, 6..7);
        let img = images[0].1.payload.downcast_ref::<ImagePayload>().unwrap();
        assert_eq!(img.dest, "p.png");
    }
}

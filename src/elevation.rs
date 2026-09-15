//! Surface roles and their depth. Geometry lives here rather than in a
//! theme file: exposing twelve numbers to theme authors invites themes
//! that look broken, and shadow geometry is design, not palette.

use gpui::{px, BoxShadow, Hsla, Pixels};

/// Where a surface sits. Every pixel in the app belongs to exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The window itself -- sidebar, tab strip, outline, status bar.
    /// Never lifts.
    Ground,
    /// The document, and only the document. The one thing that rests
    /// on the ground at rest.
    Page,
    /// Hover previews, the finder, the palette, context menus.
    Floating,
    /// Dialogs, the install flow, confirmations.
    Modal,
}

pub fn shadows(surface: Surface, shadow: Hsla) -> Vec<BoxShadow> {
    let layer = |y: f32, blur: f32, alpha: f32| BoxShadow {
        color: Hsla { a: shadow.a * alpha, ..shadow },
        offset: gpui::point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    };
    match surface {
        Surface::Ground => vec![],
        Surface::Page => vec![layer(1., 3., 0.9), layer(6., 10., 0.6)],
        Surface::Floating => vec![layer(1., 3., 1.0), layer(4., 12., 0.8)],
        Surface::Modal => vec![
            layer(1., 2., 1.0),
            layer(4., 10., 0.9),
            layer(10., 24., 0.7),
            layer(18., 44., 0.5),
        ],
    }
}

pub fn radius(surface: Surface) -> Pixels {
    match surface {
        Surface::Ground => px(0.),
        Surface::Page => px(8.),
        Surface::Floating => px(8.),
        Surface::Modal => px(12.),
    }
}

/// Apply a whole `Edges` of margins in one call.
///
/// gpui's margin setters are one per edge (`ml`, `mr`, `mb`...), so a
/// four-edge rule spelled as three or four separate calls is coupled
/// only by convention: drop one and the geometry silently changes,
/// while anything that asserts on the rule itself still passes. Taking
/// the `Edges` whole makes the edge set the unit of code -- an edge
/// cannot go missing at the call site without deleting the field it
/// came from.
pub(crate) trait Margins: gpui::Styled + Sized {
    fn margins(mut self, m: gpui::Edges<Pixels>) -> Self {
        self.style().margin = gpui::EdgesRefinement {
            top: Some(m.top.into()),
            right: Some(m.right.into()),
            bottom: Some(m.bottom.into()),
            left: Some(m.left.into()),
        };
        self
    }
}

impl<T: gpui::Styled> Margins for T {}

/// Apple's concentric rule: a rounded rect inside another shares its
/// centre of curvature, so the inner radius is the outer minus the gap
/// between them. Clamped at zero -- a negative radius is a square.
pub fn inner_radius(outer: Pixels, padding: Pixels) -> Pixels {
    let v = f32::from(outer) - f32::from(padding);
    px(v.max(0.))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each edge has to land on its own side. A transposition here
    /// would tilt every surface that takes its margins as a unit, and
    /// the four values are close enough that the screen would not
    /// obviously say so.
    #[test]
    fn margins_land_each_edge_on_its_own_side() {
        use crate::elevation::Margins as _;
        use gpui::Styled as _;
        let mut el = gpui::div().margins(gpui::Edges {
            top: px(1.),
            right: px(2.),
            bottom: px(3.),
            left: px(4.),
        });
        let m = el.style().margin.clone();
        assert_eq!(m.top, Some(px(1.).into()), "top");
        assert_eq!(m.right, Some(px(2.).into()), "right");
        assert_eq!(m.bottom, Some(px(3.).into()), "bottom");
        assert_eq!(m.left, Some(px(4.).into()), "left");
    }

    /// The ground never lifts. A shadow on a static panel tells the
    /// user it can be picked up, and the sidebar cannot.
    #[test]
    fn only_the_page_and_floating_things_cast_shadows() {
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        assert!(shadows(Surface::Ground, s).is_empty(), "ground must not lift");
        assert!(!shadows(Surface::Page, s).is_empty(), "the page rests on the ground");
        assert!(!shadows(Surface::Floating, s).is_empty());
        assert!(!shadows(Surface::Modal, s).is_empty());
    }

    /// Depth increases with the tier: a dialog reads as further from
    /// the page than a popover does.
    #[test]
    fn depth_increases_with_the_tier() {
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        let blur = |sf| {
            shadows(sf, s).iter().map(|b| f32::from(b.blur_radius)).fold(0., f32::max)
        };
        assert!(blur(Surface::Page) < blur(Surface::Floating));
        assert!(blur(Surface::Floating) < blur(Surface::Modal));
        assert!(shadows(Surface::Modal, s).len() >= shadows(Surface::Page, s).len());
    }

    /// Apple's concentric rule: a rounded thing inside a rounded thing
    /// shares its centre of curvature. inner = outer - padding.
    #[test]
    fn inner_radius_is_concentric_and_never_negative() {
        assert_eq!(inner_radius(px(12.), px(4.)), px(8.));
        assert_eq!(inner_radius(px(4.), px(9.)), px(0.), "clamped, not negative");
    }
}

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
        // gpui hands `blur_radius` to its shader as the Gaussian sigma, not
        // CSS's sigma = blur / 2, so every number here renders twice as soft
        // as it reads. Page keeps its measured geometry (Task 8). Floating
        // and Modal were first written in CSS idiom -- a 4px offset under a
        // sigma-12 blur, a sigma-44 layer -- and on screen that was a halo
        // with no direction plus a haze reaching ~75pt past a dialog. Lift
        // reads from offset relative to blur, so the tiers now drop further
        // and spread less.
        Surface::Floating => vec![layer(1., 3., 1.0), layer(10., 16., 0.9)],
        Surface::Modal => vec![layer(1., 3., 1.0), layer(8., 14., 0.8), layer(20., 30., 0.6)],
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

/// Every surface the app floats above the page, by name.
///
/// The tier each one sits at is decided here, in `surface`, and nowhere
/// else: a call site names *what it is* and `Elevated::elevated` turns
/// that into shadow and radius. Before this, fifteen sites each reached
/// for gpui's `shadow_lg()` on their own, so depth was a default nobody
/// had chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// ⌘P.
    Finder,
    /// ⌘⇧P, and the move-to-folder picker, which is a palette.
    Palette,
    /// ⌘⇧F.
    Search,
    /// The format toolbar over a selection.
    FormatToolbar,
    /// The `[[` completion list.
    LinkCompletion,
    /// The editor's link hover popover.
    LinkHover,
    /// The reading view's link preview tooltip.
    PreviewTooltip,
    /// The sidebar's right-click menu.
    ContextMenu,
    /// The title-bar app menu.
    AppMenu,
    /// "Install Plugins…".
    InstallFlow,
    /// ⌘T, behind a scrim.
    ThemePicker,
    /// About SuperMD.
    About,
    /// ⌘/, behind a scrim.
    Shortcuts,
    /// A plugin asking for a capability; the plugin waits on the answer.
    ConsentPrompt,
    /// "Install this plugin?"; the install waits on the answer.
    InstallConfirmation,
}

impl Overlay {
    #[cfg(test)]
    pub const ALL: [Overlay; 15] = [
        Overlay::Finder,
        Overlay::Palette,
        Overlay::Search,
        Overlay::FormatToolbar,
        Overlay::LinkCompletion,
        Overlay::LinkHover,
        Overlay::PreviewTooltip,
        Overlay::ContextMenu,
        Overlay::AppMenu,
        Overlay::InstallFlow,
        Overlay::ThemePicker,
        Overlay::About,
        Overlay::Shortcuts,
        Overlay::ConsentPrompt,
        Overlay::InstallConfirmation,
    ];

    /// A thing you pick from and leave floats; a thing that stops to
    /// ask, or that the app dims itself behind, is modal.
    pub fn surface(self) -> Surface {
        match self {
            Overlay::Finder
            | Overlay::Palette
            | Overlay::Search
            | Overlay::FormatToolbar
            | Overlay::LinkCompletion
            | Overlay::LinkHover
            | Overlay::PreviewTooltip
            | Overlay::ContextMenu
            | Overlay::AppMenu => Surface::Floating,
            Overlay::InstallFlow
            | Overlay::ThemePicker
            | Overlay::About
            | Overlay::Shortcuts
            | Overlay::ConsentPrompt
            | Overlay::InstallConfirmation => Surface::Modal,
        }
    }
}

/// How far a child with its own fill must stay from an overlay's rounded
/// edge. gpui's `ContentMask` is a square, so a selected row flush with a
/// rounded container paints straight over the corner arc; keeping it one
/// radius clear of the edge keeps it out of the arc entirely.
pub fn corner_inset(overlay: Overlay) -> Pixels {
    radius(overlay.surface())
}

/// Lift an element to an overlay's tier: its shadow and its corner
/// radius together, so the two cannot disagree about how far away it is.
pub(crate) trait Elevated: gpui::Styled + Sized {
    fn elevated(self, overlay: Overlay, shadow: Hsla) -> Self {
        let surface = overlay.surface();
        self.shadow(shadows(surface, shadow)).rounded(radius(surface))
    }
}

impl<T: gpui::Styled> Elevated for T {}

/// Apply a whole `Edges` of margins in one call.
///
/// gpui's margin setters are one per edge (`ml`, `mr`, `mb`...), so a
/// four-edge rule spelled as three or four separate calls is coupled
/// only by convention: drop one and the geometry silently changes,
/// while anything that asserts on the rule itself still passes. Taking
/// the `Edges` whole makes the edge set the unit of code -- an edge
/// cannot go missing at the call site without deleting the field it
/// came from.
///
/// It overwrites `style().margin` wholesale rather than merging into
/// it, so a chained `.mt(...)`/`.mb(...)`/etc on the same element is
/// only safe *after* this call -- one applied before `margins()` is
/// silently clobbered.
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

    /// Overlays share one vocabulary. A popover and a dialog are
    /// different depths on purpose, and neither invents its own.
    #[test]
    fn floating_and_modal_are_distinguishable() {
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        let f = shadows(Surface::Floating, s);
        let m = shadows(Surface::Modal, s);
        assert_ne!(f.len(), m.len(), "a dialog is not a popover");
        let deepest = |v: &Vec<gpui::BoxShadow>| {
            v.iter().map(|b| f32::from(b.blur_radius)).fold(0., f32::max)
        };
        assert!(deepest(&m) > deepest(&f) * 1.5, "a modal reads as further away");
    }

    /// Pickers, menus and popovers float; anything that stops to ask or
    /// dims the app behind it is modal. Nothing an overlay names may
    /// land on the ground or the page -- those are not overlays.
    #[test]
    fn each_overlay_sits_at_its_tier() {
        use Overlay::*;
        let floating = [
            Finder,
            Palette,
            Search,
            FormatToolbar,
            LinkCompletion,
            LinkHover,
            PreviewTooltip,
            ContextMenu,
            AppMenu,
        ];
        let modal = [
            InstallFlow,
            ThemePicker,
            About,
            Shortcuts,
            ConsentPrompt,
            InstallConfirmation,
        ];
        for o in floating {
            assert_eq!(o.surface(), Surface::Floating, "{o:?}");
        }
        for o in modal {
            assert_eq!(o.surface(), Surface::Modal, "{o:?}");
        }
        assert_eq!(
            floating.len() + modal.len(),
            Overlay::ALL.len(),
            "every overlay is classified"
        );
    }

    /// `elevated` applies the tier's shadow AND its radius. A popover
    /// with a modal's corners, or no shadow at all, is the drift this
    /// helper exists to prevent.
    #[test]
    fn elevated_applies_the_tiers_shadow_and_radius() {
        use gpui::Styled as _;
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        for o in Overlay::ALL {
            let mut el = gpui::div().elevated(o, s);
            let st = el.style();
            assert_eq!(st.box_shadow, Some(shadows(o.surface(), s)), "{o:?} shadow");
            let r: Option<gpui::AbsoluteLength> = Some(radius(o.surface()).into());
            let c = &st.corner_radii;
            assert_eq!(
                (c.top_left, c.top_right, c.bottom_right, c.bottom_left),
                (r, r, r, r),
                "{o:?} radius"
            );
        }
    }

    /// A child with its own fill must clear the whole corner arc, or a
    /// square selected row paints over the rounded edge (gpui cannot
    /// round-clip). Anything less than the radius leaves part of the arc
    /// exposed.
    #[test]
    fn corner_inset_clears_the_whole_arc() {
        for o in Overlay::ALL {
            assert!(corner_inset(o) >= radius(o.surface()), "{o:?}");
        }
    }

    /// Lift is offset relative to blur. gpui's blur is the sigma, so a
    /// floating surface whose drop is small against its blur renders as
    /// an undirected halo -- what the first CSS-idiom geometry did.
    #[test]
    fn floating_and_modal_drop_further_than_the_page() {
        let s = Hsla { h: 0., s: 0., l: 0., a: 0.3 };
        let drop = |sf| {
            shadows(sf, s).iter().map(|b| f32::from(b.offset.y)).fold(0., f32::max)
        };
        assert!(drop(Surface::Floating) > drop(Surface::Page), "a popover sits above the page");
        assert!(drop(Surface::Modal) > drop(Surface::Floating), "a dialog sits above a popover");
    }

    /// Apple's concentric rule: a rounded thing inside a rounded thing
    /// shares its centre of curvature. inner = outer - padding.
    #[test]
    fn inner_radius_is_concentric_and_never_negative() {
        assert_eq!(inner_radius(px(12.), px(4.)), px(8.));
        assert_eq!(inner_radius(px(4.), px(9.)), px(0.), "clamped, not negative");
    }
}

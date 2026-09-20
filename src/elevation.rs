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
/// that into surface, shadow and radius. Before this, fifteen sites each reached
/// for gpui's `shadow_lg()` on their own, so depth was a default nobody
/// had chosen.
///
/// The list is the whole of what floats: anything absolutely positioned
/// over the page belongs in it. `CommandError` was the counter-example
/// that proved the claim needed checking -- a strip over the page with
/// its own fill, no shadow and no radius, outside the vocabulary the
/// module's first line says covers every pixel.
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
    /// The strip that says a command refused, bottom-centre over the
    /// page for four seconds. The only overlay that carries the refusal
    /// colour instead of the floating surface -- see `fill`.
    CommandError,
}

impl Overlay {
    #[cfg(test)]
    pub const ALL: [Overlay; 16] = [
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
        Overlay::CommandError,
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
            | Overlay::AppMenu
            | Overlay::CommandError => Surface::Floating,
            Overlay::InstallFlow
            | Overlay::ThemePicker
            | Overlay::About
            | Overlay::Shortcuts
            | Overlay::ConsentPrompt
            | Overlay::InstallConfirmation => Surface::Modal,
        }
    }

    /// What an overlay is filled with.
    ///
    /// `floating_bg` for all but one, and it lives here for the same
    /// reason the tier does: the fill is what a call site gets wrong.
    /// The exception is the command-error strip, whose whole job is to
    /// say that something refused -- it carries the removed-diff wash,
    /// the app's one "this did not happen" colour, and it read as an
    /// ordinary card in any other fill. Naming the exception here keeps
    /// it inside the vocabulary rather than beside it: before this the
    /// toast was absolutely positioned with its own `.bg`, no shadow and
    /// no tier, a sixteenth floating thing the system did not know about.
    pub fn fill(self, t: &crate::theme::Theme) -> Hsla {
        match self {
            Overlay::CommandError => t.diff_deleted_bg,
            _ => t.floating_bg,
        }
    }
}

/// How far the app dims itself behind a surface, if it does.
///
/// A modal is partly *defined* by this (`Overlay::surface`), and the two
/// dialogs that painted one spelled two different alphas inline -- 0.25
/// under the theme picker, 0.35 under the shortcuts sheet -- so "behind
/// a scrim" meant two different amounts of dimming depending on which
/// dialog you opened. One number for the tier, the midpoint of the two
/// that shipped: the strength of a scrim is a property of how far away
/// the thing above it is, not of which dialog it happens to be.
pub fn scrim(surface: Surface) -> Option<Hsla> {
    match surface {
        Surface::Ground | Surface::Page | Surface::Floating => None,
        Surface::Modal => Some(Hsla { h: 0., s: 0., l: 0., a: 0.30 }),
    }
}

/// How far a child with its own fill must stay from an overlay's rounded
/// edge. gpui's `ContentMask` is a square, so a selected row flush with a
/// rounded container paints straight over the corner arc; keeping it one
/// radius clear of the edge keeps it out of the arc entirely.
pub fn corner_inset(overlay: Overlay) -> Pixels {
    radius(overlay.surface())
}

/// Lift an element to an overlay's tier: its surface, its shadow and its
/// corner radius together, so none of the three can disagree about how
/// far away it is.
///
/// The surface is here, not at the call site, because it is the one a
/// site is most likely to get wrong. Every overlay used to paint
/// `panel_bg`, which is also the table header and the knowledge panel,
/// and in most themes it is darker than the page -- so a shadow under it
/// read as a recess. Setting it here means a new overlay cannot choose
/// it. A site must not follow this with its own `.bg(...)`: gpui keeps
/// the last fill set, and `no_overlay_paints_its_own_surface` holds the
/// sites to that.
pub(crate) trait Elevated: gpui::Styled + Sized {
    fn elevated(self, overlay: Overlay, t: &crate::theme::Theme) -> Self {
        let surface = overlay.surface();
        self.bg(overlay.fill(t)).shadow(shadows(surface, t.shadow)).rounded(radius(surface))
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

    /// One tier dims the app behind it and the others do not, and the
    /// amount is the tier's rather than each dialog's. The theme picker
    /// spelled 0.25 and the shortcuts sheet 0.35, inline, while
    /// `Overlay::surface` defined modal partly *as* the tier the app
    /// dims itself behind -- two numbers for one idea.
    #[test]
    fn only_a_modal_dims_the_app_behind_it() {
        assert_eq!(scrim(Surface::Ground), None);
        assert_eq!(scrim(Surface::Page), None);
        assert_eq!(scrim(Surface::Floating), None, "a popover does not dim the app");
        let dim = scrim(Surface::Modal).expect("a modal dims what is behind it");
        assert_eq!((dim.h, dim.s, dim.l), (0., 0., 0.), "a scrim is neutral black");
        assert!(dim.a > 0.1 && dim.a < 0.5, "visible, and still see-through: {}", dim.a);
    }

    /// And no dialog spells one for itself. Two did, at two alphas; the
    /// shape they used was a literal neutral black with an alpha, so
    /// that is what this looks for outside this module.
    #[test]
    fn no_dialog_spells_its_own_scrim() {
        for (file, src) in [
            ("workspace.rs", include_str!("workspace.rs")),
            ("install_ui.rs", include_str!("install_ui.rs")),
            ("search_ui.rs", include_str!("search_ui.rs")),
            ("finder.rs", include_str!("finder.rs")),
            ("palette.rs", include_str!("palette.rs")),
        ] {
            for (ix, line) in src.lines().enumerate() {
                assert!(
                    !(line.contains(".bg(") && line.contains("s: 0., l: 0., a:")),
                    "{file}:{}: a dialog spells its own scrim -- `elevation::scrim` owns it",
                    ix + 1
                );
            }
        }
    }

    /// The command-error strip is the one overlay that does not take the
    /// floating surface: it carries the removed-diff wash, because what
    /// it says is that something refused. Every other overlay takes
    /// `floating_bg`, and the decision is here either way.
    #[test]
    fn only_the_command_error_strip_departs_from_the_floating_surface() {
        let t = crate::theme::Theme::dark();
        for o in Overlay::ALL {
            let expected =
                if o == Overlay::CommandError { t.diff_deleted_bg } else { t.floating_bg };
            assert_eq!(o.fill(&t), expected, "{o:?}");
        }
        assert_ne!(
            t.diff_deleted_bg, t.floating_bg,
            "the exception has to be an exception, or this test proves nothing"
        );
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
            CommandError,
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

    /// `elevated` applies the floating surface, the tier's shadow AND its
    /// radius. A popover with a modal's corners, no shadow at all, or a
    /// surface darker than the page it floats over, is the drift this
    /// helper exists to prevent.
    #[test]
    fn elevated_applies_the_surface_and_the_tiers_shadow_and_radius() {
        use gpui::Styled as _;
        let t = crate::theme::Theme::dark();
        let s = t.shadow;
        for o in Overlay::ALL {
            let mut el = gpui::div().elevated(o, &t);
            let st = el.style();
            assert_eq!(st.background, Some(o.fill(&t).into()), "{o:?} surface");
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

    /// Every overlay takes its surface from `elevated`, and none paints
    /// its own over it. gpui keeps the last fill set on an element, so a
    /// `.bg(...)` anywhere in the same builder chain -- before the call or
    /// after -- either is overwritten or overwrites; after is the silent
    /// one. Both are a site choosing its own surface, which is how every
    /// overlay came to be `panel_bg`, a colour darker than the page in
    /// all eight shipped themes.
    ///
    /// The chain is every line from the `.elevated(` call outward until
    /// the indentation drops below it (the `div()` that starts the
    /// builder, or whatever consumes it). Of those, only the lines at the
    /// call's own indentation are the container's methods: a child's fill
    /// sits deeper, so a selected row's `.bg(t.selected_bg)` is not
    /// mistaken for the container's, and a multi-line closure argument's
    /// closing `})` does not end the walk early.
    #[test]
    fn no_overlay_paints_its_own_surface() {
        let sources = [
            ("finder.rs", include_str!("finder.rs")),
            ("palette.rs", include_str!("palette.rs")),
            ("search_ui.rs", include_str!("search_ui.rs")),
            ("install_ui.rs", include_str!("install_ui.rs")),
            ("preview.rs", include_str!("preview.rs")),
            ("workspace.rs", include_str!("workspace.rs")),
            ("editor/mod.rs", include_str!("editor/mod.rs")),
        ];
        let mut sites = 0;
        for (file, src) in sources {
            let lines: Vec<&str> = src.lines().collect();
            for (ix, line) in lines.iter().enumerate() {
                let body = line.trim_start();
                if !body.starts_with(".elevated(crate::elevation::Overlay::") {
                    continue;
                }
                sites += 1;
                let indent = line.len() - body.len();
                let depth = |l: &str| l.len() - l.trim_start().len();
                let in_chain = |l: &&&str| l.trim().is_empty() || depth(l) >= indent;
                let before = lines[..ix].iter().rev().take_while(in_chain);
                let after = lines[ix + 1..].iter().take_while(in_chain);
                let own = |l: &&&str| depth(l) == indent && l.trim_start().starts_with('.');
                for l in before.chain(after).filter(own) {
                    assert!(
                        !l.contains(".bg("),
                        "{file}:{}: an overlay paints its own surface ({}) -- `elevated` owns it",
                        ix + 1,
                        l.trim()
                    );
                }
            }
        }
        // Not a count of overlays -- `Overlay::ALL` is that -- but proof
        // the scan found the call shape at all. If a refactor respells
        // the call, this fires rather than the test passing on nothing.
        assert!(sites >= Overlay::ALL.len(), "found only {sites} `.elevated(` sites");
    }

    /// Apple's concentric rule: a rounded thing inside a rounded thing
    /// shares its centre of curvature. inner = outer - padding.
    #[test]
    fn inner_radius_is_concentric_and_never_negative() {
        assert_eq!(inner_radius(px(12.), px(4.)), px(8.));
        assert_eq!(inner_radius(px(4.), px(9.)), px(0.), "clamped, not negative");
    }
}

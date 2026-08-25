//! One open document: parsed blocks, outline, and its scroll position.

use std::path::{Path, PathBuf};

use gpui::{
    actions, div, list, px, App, FocusHandle, IntoElement, ListAlignment, ListOffset, ListState,
    ParentElement, Render, SharedString, Styled, Window,
};
use gpui::prelude::*;

use crate::highlight::Languages;
use crate::markdown::{self, Block, Document};
use crate::theme::theme;
use crate::view;

actions!(
    reader,
    [ScrollUp, ScrollDown, PageUp, PageDown, ScrollTop, ScrollBottom]
);

/// One arrow-key step, in pixels. The editor pages by a fixed line
/// count because its lines are uniform height; a reader's blocks are
/// not, so the reader steps in pixels against its measured viewport.
const LINE_STEP: f32 = 72.;
/// How much of the current screen a page jump carries over.
const PAGE_OVERLAP: f32 = 48.;

/// New offset (px from the top) after moving `delta`, clamped to the
/// scrollable range. `ListState::scroll_by` clamps at the top but seeks
/// past the content bottom, so the bottom clamp has to be ours.
fn clamped_scroll(current: f32, delta: f32, max: f32) -> f32 {
    (current + delta).clamp(0., max.max(0.))
}

/// A page jump keeps a sliver of the previous screen for continuity,
/// and always advances even when the viewport is tiny.
fn page_step(viewport_height: f32) -> f32 {
    (viewport_height - PAGE_OVERLAP).max(LINE_STEP)
}

pub struct TocEntry {
    pub level: u8,
    pub text: SharedString,
    pub block_ix: usize,
}

pub struct Reader {
    pub path: Option<PathBuf>,
    pub title: SharedString,
    pub document: std::sync::Arc<Document>,
    pub toc: Vec<TocEntry>,
    pub list_state: ListState,
    focus_handle: FocusHandle,
    scroll_anim: Option<gpui::Task<()>>,
}

/// Language token for a file. Delegates to the central mapping.
pub fn language_for_path(path: &Path) -> Option<String> {
    crate::highlight::language_for_file(path)
}

impl Reader {
    /// Build a pretty-rendered document from Markdown source (used for
    /// the ⌘E preview of an editor buffer).
    pub fn from_source(
        title: SharedString,
        source: &str,
        langs: &Languages,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut document = markdown::parse(source);
        langs.highlight_document(&mut document);
        Self::from_document(None, title, document, cx)
    }

    pub fn welcome(langs: &Languages, cx: &mut Context<Self>) -> Self {
        let mut document = markdown::parse(include_str!("../WELCOME.md"));
        langs.highlight_document(&mut document);
        Self::from_document(None, "Welcome".into(), document, cx)
    }

    fn from_document(
        path: Option<PathBuf>,
        title: SharedString,
        document: Document,
        cx: &mut Context<Self>,
    ) -> Self {
        let toc = document
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(ix, block)| match block {
                Block::Heading { level, content } => Some(TocEntry {
                    level: *level,
                    text: SharedString::from(content.text.clone()),
                    block_ix: ix,
                }),
                _ => None,
            })
            .collect();
        let document = std::sync::Arc::new(document);
        let list_state = ListState::new(document.blocks.len(), ListAlignment::Top, px(512.));
        Self {
            path,
            title,
            document,
            toc,
            list_state,
            focus_handle: cx.focus_handle(),
            scroll_anim: None,
        }
    }

    pub fn scroll_to_block(&mut self, block_ix: usize, cx: &mut Context<Self>) {
        let state = self.list_state.clone();
        let current = -state.scroll_px_offset_for_scrollbar().y;
        state.scroll_to(ListOffset { item_ix: block_ix, offset_in_item: px(0.) });
        let target_px = -state.scroll_px_offset_for_scrollbar().y;
        if (target_px - current).abs() < px(24.) {
            cx.notify();
            return;
        }
        state.set_offset_from_scrollbar(gpui::point(px(0.), -current));
        self.scroll_anim = Some(cx.spawn(async move |this, cx| {
            const FRAMES: u32 = 22;
            for frame in 1..=FRAMES {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(11))
                    .await;
                let t = frame as f32 / FRAMES as f32;
                let eased = 1.0 - (1.0 - t).powi(3);
                let y = current + (target_px - current) * eased;
                if this
                    .update(cx, |reader, cx| {
                        reader
                            .list_state
                            .set_offset_from_scrollbar(gpui::point(px(0.), -y));
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            this.update(cx, |reader, cx| {
                reader
                    .list_state
                    .scroll_to(ListOffset { item_ix: block_ix, offset_in_item: px(0.) });
                cx.notify();
            })
            .ok();
        }));
    }

    /// Move the viewport by `delta` px, clamped to the document. A key
    /// press cancels any in-flight outline animation: the key wins.
    fn scroll_px(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.scroll_anim = None;
        let current = f32::from(-self.list_state.scroll_px_offset_for_scrollbar().y);
        let max = f32::from(self.list_state.max_offset_for_scrollbar().height);
        let target = clamped_scroll(current, delta, max);
        self.list_state
            .set_offset_from_scrollbar(gpui::point(px(0.), -px(target)));
        cx.notify();
    }

    /// A page is the measured viewport, less the overlap kept for context.
    fn page(&self) -> f32 {
        page_step(f32::from(self.list_state.viewport_bounds().size.height))
    }

    fn scroll_up(&mut self, _: &ScrollUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(-LINE_STEP, cx);
    }

    fn scroll_down(&mut self, _: &ScrollDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(LINE_STEP, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(-self.page(), cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_px(self.page(), cx);
    }

    fn scroll_top(&mut self, _: &ScrollTop, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_anim = None;
        self.list_state
            .scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
        cx.notify();
    }

    fn scroll_bottom(&mut self, _: &ScrollBottom, _: &mut Window, cx: &mut Context<Self>) {
        // The list measures items lazily, so content height is only known
        // for what has already been rendered — a px jump to `max_offset`
        // stops short. Scrolling past the last item is measurement-free:
        // layout finds nothing below, walks back up measuring until the
        // viewport is full, and settles at the true bottom.
        self.scroll_anim = None;
        self.list_state.scroll_to(ListOffset {
            item_ix: self.list_state.item_count(),
            offset_in_item: px(0.),
        });
        cx.notify();
    }
}

impl gpui::Focusable for Reader {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Reader {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.weak_entity();
        let t = theme(cx);
        div()
            .size_full()
            .bg(t.bg)
            .key_context("Reader")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::scroll_up))
            .on_action(cx.listener(Self::scroll_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::scroll_top))
            .on_action(cx.listener(Self::scroll_bottom))
            .child(
                list(self.list_state.clone(), move |ix, _window, cx| {
                    let Some(reader) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let t = theme(cx);
                    let document = reader.read(cx).document.clone();
                    view::list_item(&document, ix, &t, cx)
                })
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Entity, TestAppContext, VisualTestContext};
    use std::sync::Arc;

    const DOC: &str = "# Alpha\n\nintro\n\n## Beta\n\n```rust\nfn main() {}\n```\n\n### Gamma\n\ntail\n";

    // ── pure construction ──────────────────────────────────────────────

    #[test]
    fn language_for_path_delegates_to_central_mapping() {
        assert_eq!(language_for_path(Path::new("main.rs")).as_deref(), Some("rust"));
        assert_eq!(language_for_path(Path::new("Dockerfile")).as_deref(), Some("dockerfile"));
        assert_eq!(language_for_path(Path::new("noext")), None);
    }

    #[gpui::test]
    fn from_source_builds_outline_and_highlights_code(cx: &mut TestAppContext) {
        let langs = Languages::new();
        let entity = cx.new(|cx| Reader::from_source("Preview".into(), DOC, &langs, cx));
        entity.read_with(cx, |reader, _| {
        assert_eq!(reader.title.as_ref(), "Preview");
        assert!(reader.path.is_none());
        assert!(reader.scroll_anim.is_none());

        let toc: Vec<(u8, &str)> =
            reader.toc.iter().map(|e| (e.level, e.text.as_ref())).collect();
        assert_eq!(toc, [(1, "Alpha"), (2, "Beta"), (3, "Gamma")]);

        // Each outline entry points at the heading block it was built from.
        for entry in &reader.toc {
            let Block::Heading { level, content } = &reader.document.blocks[entry.block_ix]
            else {
                panic!("toc entry {} does not point at a heading", entry.text)
            };
            assert_eq!(*level, entry.level);
            assert_eq!(content.text.as_str(), entry.text.as_ref());
        }

        // The rust fence got real highlight spans during construction.
        let highlighted = reader.document.blocks.iter().any(|b| {
            matches!(b, Block::Code { lang: Some(l), spans, .. } if l == "rust" && !spans.is_empty())
        });
        assert!(highlighted, "code block should be highlighted");
        });
    }

    #[gpui::test]
    fn welcome_document_carries_the_bundled_tour(cx: &mut TestAppContext) {
        let langs = Languages::new();
        let entity = cx.new(|cx| Reader::welcome(&langs, cx));
        entity.read_with(cx, |reader, _| {
        assert_eq!(reader.title.as_ref(), "Welcome");
        assert!(reader.path.is_none());
        assert_eq!(reader.toc[0].level, 1);
        assert_eq!(reader.toc[0].text.as_ref(), "Welcome to SuperMD");
        assert!(
            reader.toc.iter().any(|e| e.text.as_ref() == "Start here"),
            "expected the Start here section in the welcome outline"
        );
        assert!(reader.document.blocks.len() > 5, "welcome tour lost its body");
        });
    }

    // ── window rendering and scrolling ─────────────────────────────────

    /// Enough paragraphs that the far end sits well past one viewport.
    fn long_source() -> String {
        let mut s = String::from("# Top\n\n");
        for i in 0..120 {
            s.push_str(&format!("Paragraph number {i} with a little bit of text.\n\n"));
        }
        s.push_str("## Bottom\n");
        s
    }

    fn open_reader<'a>(
        cx: &'a mut TestAppContext,
        source: &str,
    ) -> (Entity<Reader>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(crate::theme::Theme::dark())));
        });
        let langs = Languages::new();
        let (reader, cx) =
            cx.add_window_view(|_, cx| Reader::from_source("doc".into(), source, &langs, cx));
        cx.run_until_parked();
        (reader, cx)
    }

    #[gpui::test]
    fn scrolling_to_a_nearby_block_snaps_without_animation(cx: &mut TestAppContext) {
        let (reader, cx) = open_reader(cx, &long_source());
        reader.update_in(cx, |reader, _, cx| {
            reader.scroll_to_block(0, cx);
            assert!(reader.scroll_anim.is_none(), "top-to-top scroll must not animate");
            assert_eq!(reader.list_state.logical_scroll_top().item_ix, 0);
        });
    }

    #[gpui::test]
    fn scrolling_to_a_far_block_animates_to_its_offset(cx: &mut TestAppContext) {
        let (reader, cx) = open_reader(cx, &long_source());
        let last = cx.update(|_, app| reader.read(app).document.blocks.len() - 1);

        // Where does a direct jump to the last block settle once the list
        // clamps at the content bottom? That is the animation's target.
        reader.update_in(cx, |reader, _, cx| {
            reader
                .list_state
                .scroll_to(ListOffset { item_ix: last, offset_in_item: px(0.) });
            cx.notify();
        });
        cx.run_until_parked();
        let expected = cx.update(|_, app| reader.read(app).list_state.logical_scroll_top());
        assert!(expected.item_ix > 0, "window draw did not measure list items");

        // Back to the top, then take the animated path.
        reader.update_in(cx, |reader, _, cx| {
            reader
                .list_state
                .scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
            cx.notify();
        });
        cx.run_until_parked();
        reader.update_in(cx, |reader, _, cx| {
            reader.scroll_to_block(last, cx);
            assert!(reader.scroll_anim.is_some(), "far scroll should animate");
        });

        // Let all 22 animation frames (11ms apart) play out.
        cx.executor().advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();

        reader.update_in(cx, |reader, _, _| {
            let landed = reader.list_state.logical_scroll_top();
            assert_eq!(landed.item_ix, expected.item_ix, "animation should land where a direct jump does");
            assert_eq!(landed.offset_in_item, expected.offset_in_item);
        });
    }

    // ── keyboard scrolling ─────────────────────────────────────────────

    #[test]
    fn scrolling_up_from_the_top_stays_at_the_top() {
        assert_eq!(clamped_scroll(0., -LINE_STEP, 900.), 0.);
        assert_eq!(clamped_scroll(30., -LINE_STEP, 900.), 0.);
    }

    #[test]
    fn scrolling_down_stops_at_the_content_bottom() {
        assert_eq!(clamped_scroll(880., 200., 900.), 900.);
        // A document shorter than its viewport has nowhere to go.
        assert_eq!(clamped_scroll(0., 200., 0.), 0.);
    }

    #[test]
    fn scrolling_within_the_range_moves_by_the_full_delta() {
        assert_eq!(clamped_scroll(100., 72., 900.), 172.);
        assert_eq!(clamped_scroll(100., -72., 900.), 28.);
    }

    #[test]
    fn a_page_step_keeps_some_context_from_the_previous_screen() {
        assert_eq!(page_step(600.), 600. - PAGE_OVERLAP);
    }

    #[test]
    fn a_page_step_on_a_short_viewport_still_advances() {
        // Never zero, or PageDown would appear dead on a tiny window.
        assert!(page_step(20.) > 0.);
        assert!(page_step(0.) > 0.);
    }

    // ── the keys, through a real window ────────────────────────────────

    /// Scroll position in px from the top of the document.
    fn offset_px(reader: &Entity<Reader>, cx: &mut VisualTestContext) -> f32 {
        cx.update(|_, app| {
            f32::from(-reader.read(app).list_state.scroll_px_offset_for_scrollbar().y)
        })
    }

    fn open_focused_reader<'a>(
        cx: &'a mut TestAppContext,
        source: &str,
    ) -> (Entity<Reader>, &'a mut VisualTestContext) {
        let (reader, cx) = open_reader(cx, source);
        reader.update_in(cx, |reader, window, _| window.focus(&reader.focus_handle));
        cx.run_until_parked();
        (reader, cx)
    }

    #[gpui::test]
    fn arrow_keys_scroll_the_rendered_document(cx: &mut TestAppContext) {
        let (reader, cx) = open_focused_reader(cx, &long_source());
        assert_eq!(offset_px(&reader, cx), 0.);

        cx.dispatch_action(ScrollDown);
        cx.run_until_parked();
        let down = offset_px(&reader, cx);
        assert!(down > 0., "down arrow should scroll the preview, got {down}");

        cx.dispatch_action(ScrollUp);
        cx.run_until_parked();
        assert_eq!(offset_px(&reader, cx), 0., "up arrow should scroll back");
    }

    #[gpui::test]
    fn page_down_moves_further_than_an_arrow_key(cx: &mut TestAppContext) {
        let (reader, cx) = open_focused_reader(cx, &long_source());
        cx.dispatch_action(ScrollDown);
        cx.run_until_parked();
        let line = offset_px(&reader, cx);

        cx.dispatch_action(ScrollTop);
        cx.run_until_parked();
        cx.dispatch_action(PageDown);
        cx.run_until_parked();
        let page = offset_px(&reader, cx);

        assert!(page > line, "PageDown ({page}) should outrun one arrow ({line})");

        cx.dispatch_action(PageUp);
        cx.run_until_parked();
        assert_eq!(offset_px(&reader, cx), 0., "PageUp should return to the top");
    }

    #[gpui::test]
    fn end_and_home_jump_to_the_document_ends(cx: &mut TestAppContext) {
        let (reader, cx) = open_focused_reader(cx, &long_source());
        let last = cx.update(|_, app| reader.read(app).document.blocks.len() - 1);
        // The end of a long document starts off screen.
        let offscreen = cx
            .update(|_, app| reader.read(app).list_state.bounds_for_item(last).is_none());
        assert!(offscreen, "test document must overflow its viewport");

        cx.dispatch_action(ScrollBottom);
        cx.run_until_parked();
        let onscreen = cx
            .update(|_, app| reader.read(app).list_state.bounds_for_item(last).is_some());
        assert!(onscreen, "End should bring the last block on screen");

        cx.dispatch_action(ScrollTop);
        cx.run_until_parked();
        assert_eq!(offset_px(&reader, cx), 0., "Home should land at the top");
    }
}

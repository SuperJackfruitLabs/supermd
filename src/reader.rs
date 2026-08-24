//! One open document: parsed blocks, outline, and its scroll position.

use std::path::{Path, PathBuf};

use gpui::{
    div, list, px, IntoElement, ListAlignment, ListOffset, ListState, ParentElement, Render,
    SharedString, Styled, Window,
};
use gpui::prelude::*;

use crate::highlight::Languages;
use crate::markdown::{self, Block, Document};
use crate::theme::theme;
use crate::view;

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
    scroll_anim: Option<gpui::Task<()>>,
}

/// Language token for a file. Delegates to the central mapping.
pub fn language_for_path(path: &Path) -> Option<String> {
    crate::highlight::language_for_file(path)
}

impl Reader {
    /// Build a pretty-rendered document from Markdown source (used for
    /// the ⌘E preview of an editor buffer).
    pub fn from_source(title: SharedString, source: &str, langs: &Languages) -> Self {
        let mut document = markdown::parse(source);
        langs.highlight_document(&mut document);
        Self::from_document(None, title, document)
    }

    pub fn welcome(langs: &Languages) -> Self {
        let mut document = markdown::parse(include_str!("../WELCOME.md"));
        langs.highlight_document(&mut document);
        Self::from_document(None, "Welcome".into(), document)
    }

    fn from_document(path: Option<PathBuf>, title: SharedString, document: Document) -> Self {
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
}

impl Render for Reader {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.weak_entity();
        let t = theme(cx);
        div().size_full().bg(t.bg).child(
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

    #[test]
    fn from_source_builds_outline_and_highlights_code() {
        let langs = Languages::new();
        let reader = Reader::from_source("Preview".into(), DOC, &langs);
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
    }

    #[test]
    fn welcome_document_carries_the_bundled_tour() {
        let reader = Reader::welcome(&Languages::new());
        assert_eq!(reader.title.as_ref(), "Welcome");
        assert!(reader.path.is_none());
        assert_eq!(reader.toc[0].level, 1);
        assert_eq!(reader.toc[0].text.as_ref(), "Welcome to SuperMD");
        assert!(
            reader.toc.iter().any(|e| e.text.as_ref() == "Start here"),
            "expected the Start here section in the welcome outline"
        );
        assert!(reader.document.blocks.len() > 5, "welcome tour lost its body");
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
        let (reader, cx) = cx.add_window_view(|_, _| Reader::from_source("doc".into(), source, &langs));
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
}

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

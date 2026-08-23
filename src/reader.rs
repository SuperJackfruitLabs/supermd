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
    pub document: Document,
    pub toc: Vec<TocEntry>,
    pub list_state: ListState,
}

/// Map a file extension to the language name used for fenced code blocks.
pub fn language_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    Some(match ext {
        "rs" => "rust",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "py" => "python",
        "json" => "json",
        "sh" | "bash" | "zsh" => "bash",
        "toml" => "toml",
        "css" => "css",
        "html" | "htm" => "html",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "swift" => "swift",
        "yml" | "yaml" => "yaml",
        _ => return None,
    })
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "markdown" | "mdown" | "mdx")
    )
}

impl Reader {
    pub fn open(path: &Path, langs: &Languages) -> std::io::Result<Self> {
        let source = std::fs::read_to_string(path)?;
        let title: SharedString = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
            .into();

        let mut document = if is_markdown(path) {
            markdown::parse(&source)
        } else {
            // Non-Markdown files render as one big highlighted code block.
            Document {
                blocks: vec![Block::Code {
                    lang: language_for_path(path).map(str::to_string),
                    code: source,
                    spans: Vec::new(),
                }],
            }
        };
        langs.highlight_document(&mut document);

        Ok(Self::from_document(Some(path.to_path_buf()), title, document))
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
        let list_state = ListState::new(document.blocks.len(), ListAlignment::Top, px(512.));
        Self {
            path,
            title,
            document,
            toc,
            list_state,
        }
    }

    pub fn scroll_to_block(&mut self, block_ix: usize) {
        self.list_state.scroll_to(ListOffset {
            item_ix: block_ix,
            offset_in_item: px(0.),
        });
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
                let reader = reader.read(cx);
                view::list_item(&reader.document, ix, &t)
            })
            .size_full(),
        )
    }
}

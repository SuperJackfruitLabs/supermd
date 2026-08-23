//! Syntax highlighting via inkjet (78 bundled tree-sitter grammars with
//! Helix's highlight queries). Highlighting runs once when a document is
//! opened or edited (not per frame): each code block/file gets a list of
//! (byte range, capture index) spans that the view layer maps to theme
//! colors.

use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Global};
use inkjet::tree_sitter_highlight::HighlightEvent;
use inkjet::{Highlighter, Language};

use crate::markdown::{Block, Document};

/// Capture names (Helix scheme); a span's `u8` indexes into this table.
pub const CAPTURE_NAMES: &[&str] = inkjet::constants::HIGHLIGHT_NAMES;

pub struct Languages;

impl Languages {
    pub fn new() -> Self {
        Self
    }

    fn language_for(name: &str) -> Option<Language> {
        let canonical = match name {
            "rs" => "rust",
            "js" | "mjs" | "cjs" => "javascript",
            "ts" => "typescript",
            "py" => "python",
            "sh" | "shell" | "zsh" => "bash",
            "golang" => "go",
            "yml" => "yaml",
            "rb" => "ruby",
            "cs" | "c#" => "csharp",
            "c++" | "cc" | "hpp" | "cxx" => "cpp",
            "ex" | "exs" => "elixir",
            "hs" => "haskell",
            "ml" | "mli" => "ocaml",
            "sc" => "scala",
            "erl" | "hrl" => "erlang",
            "pl" | "pm" => "perl",
            "gql" => "graphql",
            "kt" => "kotlin",
            "dockerfile" => "dockerfile",
            other => other,
        };
        Language::from_token(canonical)
    }

    /// Highlight `code`, returning (byte range, capture index) spans.
    pub fn highlight(&self, lang: &str, code: &str) -> Vec<(Range<usize>, u8)> {
        let Some(language) = Self::language_for(lang) else {
            return Vec::new();
        };
        // Plaintext has no queries; skip the parse entirely.
        if matches!(language, Language::Plaintext) {
            return Vec::new();
        }
        let mut highlighter = Highlighter::new();
        let Ok(events) = highlighter.highlight_raw(language, &code) else {
            return Vec::new();
        };

        let mut spans = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for event in events.flatten() {
            match event {
                HighlightEvent::HighlightStart(h) => stack.push(h.0),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
                HighlightEvent::Source { start, end } => {
                    if let Some(&capture) = stack.last() {
                        if start < end && capture < CAPTURE_NAMES.len() && capture <= u8::MAX as usize
                        {
                            spans.push((start..end, capture as u8));
                        }
                    }
                }
            }
        }
        spans
    }

    /// Fill in highlight spans for every code block in the document.
    pub fn highlight_document(&self, doc: &mut Document) {
        for block in &mut doc.blocks {
            self.highlight_block(block);
        }
    }

    fn highlight_block(&self, block: &mut Block) {
        match block {
            Block::Code { lang, code, spans } => {
                if let Some(lang) = lang {
                    *spans = self.highlight(lang, code);
                }
            }
            Block::Quote(blocks) => {
                for b in blocks {
                    self.highlight_block(b);
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    for b in &mut item.blocks {
                        self.highlight_block(b);
                    }
                }
            }
            _ => {}
        }
    }
}

pub struct SyntaxLanguages(pub Arc<Languages>);

impl Global for SyntaxLanguages {}

/// The shared language registry. Cheap to clone (Arc).
pub fn languages(cx: &App) -> Arc<Languages> {
    cx.global::<SyntaxLanguages>().0.clone()
}

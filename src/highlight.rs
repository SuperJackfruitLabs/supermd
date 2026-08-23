//! Tree-sitter syntax highlighting for code blocks and code files.
//!
//! Highlighting runs once when a document is opened (not per frame): each
//! `Block::Code` gets a list of (byte range, capture index) spans that the
//! view layer maps to theme colors.

use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Global};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::markdown::{Block, Document};

/// Capture names we recognize, in priority order. A span's `u8` capture index
/// points into this table.
pub const CAPTURE_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "function",
    "function.builtin",
    "keyword",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

pub struct Languages {
    configs: Vec<(&'static str, HighlightConfiguration)>,
}

impl Languages {
    pub fn new() -> Self {
        let mut configs = Vec::new();

        let mut add = |name: &'static str,
                       language: tree_sitter_language::LanguageFn,
                       highlights: &str,
                       injections: &str,
                       locals: &str| {
            match HighlightConfiguration::new(language.into(), name, highlights, injections, locals)
            {
                Ok(mut config) => {
                    config.configure(CAPTURE_NAMES);
                    configs.push((name, config));
                }
                Err(err) => eprintln!("supermd: failed to load {name} grammar: {err}"),
            }
        };

        add(
            "rust",
            tree_sitter_rust::LANGUAGE,
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        );
        add(
            "javascript",
            tree_sitter_javascript::LANGUAGE,
            &format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            ),
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        );
        add(
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            &format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ),
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        );
        add(
            "tsx",
            tree_sitter_typescript::LANGUAGE_TSX,
            &format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ),
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        );
        add(
            "python",
            tree_sitter_python::LANGUAGE,
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        );
        add(
            "json",
            tree_sitter_json::LANGUAGE,
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        );
        add(
            "bash",
            tree_sitter_bash::LANGUAGE,
            tree_sitter_bash::HIGHLIGHT_QUERY,
            "",
            "",
        );
        add(
            "css",
            tree_sitter_css::LANGUAGE,
            tree_sitter_css::HIGHLIGHTS_QUERY,
            "",
            "",
        );
        add(
            "html",
            tree_sitter_html::LANGUAGE,
            tree_sitter_html::HIGHLIGHTS_QUERY,
            tree_sitter_html::INJECTIONS_QUERY,
            "",
        );
        add(
            "go",
            tree_sitter_go::LANGUAGE,
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        );
        add(
            "c",
            tree_sitter_c::LANGUAGE,
            tree_sitter_c::HIGHLIGHT_QUERY,
            "",
            "",
        );
        add("yaml", tree_sitter_yaml::LANGUAGE, tree_sitter_yaml::HIGHLIGHTS_QUERY, "", "");
        add("toml", tree_sitter_toml_ng::LANGUAGE, tree_sitter_toml_ng::HIGHLIGHTS_QUERY, "", "");
        add("ruby", tree_sitter_ruby::LANGUAGE, tree_sitter_ruby::HIGHLIGHTS_QUERY, "", "");
        add("java", tree_sitter_java::LANGUAGE, tree_sitter_java::HIGHLIGHTS_QUERY, "", "");
        add("php", tree_sitter_php::LANGUAGE_PHP, tree_sitter_php::HIGHLIGHTS_QUERY, "", "");
        // C++'s bundled query holds only the additions over C's.
        add(
            "cpp",
            tree_sitter_cpp::LANGUAGE,
            &format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            ),
            "",
            "",
        );
        add("csharp", tree_sitter_c_sharp::LANGUAGE, tree_sitter_c_sharp::HIGHLIGHTS_QUERY, "", "");
        add("lua", tree_sitter_lua::LANGUAGE, tree_sitter_lua::HIGHLIGHTS_QUERY, "", "");
        add("elixir", tree_sitter_elixir::LANGUAGE, tree_sitter_elixir::HIGHLIGHTS_QUERY, "", "");
        add("haskell", tree_sitter_haskell::LANGUAGE, tree_sitter_haskell::HIGHLIGHTS_QUERY, "", "");
        add("ocaml", tree_sitter_ocaml::LANGUAGE_OCAML, tree_sitter_ocaml::HIGHLIGHTS_QUERY, "", "");
        add("scala", tree_sitter_scala::LANGUAGE, tree_sitter_scala::HIGHLIGHTS_QUERY, "", "");
        add("zig", tree_sitter_zig::LANGUAGE, tree_sitter_zig::HIGHLIGHTS_QUERY, "", "");
        add("swift", tree_sitter_swift::LANGUAGE, tree_sitter_swift::HIGHLIGHTS_QUERY, "", "");
        add("elm", tree_sitter_elm::LANGUAGE, tree_sitter_elm::HIGHLIGHTS_QUERY, "", "");
        add("erlang", tree_sitter_erlang::LANGUAGE, tree_sitter_erlang::HIGHLIGHTS_QUERY, "", "");
        add("sql", tree_sitter_sequel::LANGUAGE, tree_sitter_sequel::HIGHLIGHTS_QUERY, "", "");
        add("xml", tree_sitter_xml::LANGUAGE_XML, tree_sitter_xml::XML_HIGHLIGHT_QUERY, "", "");
        add("regex", tree_sitter_regex::LANGUAGE, tree_sitter_regex::HIGHLIGHTS_QUERY, "", "");
        add("nix", tree_sitter_nix::LANGUAGE, tree_sitter_nix::HIGHLIGHTS_QUERY, "", "");
        add("r", tree_sitter_r::LANGUAGE, tree_sitter_r::HIGHLIGHTS_QUERY, "", "");
        add("gleam", tree_sitter_gleam::LANGUAGE, tree_sitter_gleam::HIGHLIGHT_QUERY, "", "");
        add("svelte", tree_sitter_svelte_ng::LANGUAGE, tree_sitter_svelte_ng::HIGHLIGHTS_QUERY, "", "");
        add("dart", tree_sitter_dart::LANGUAGE, tree_sitter_dart::HIGHLIGHTS_QUERY, "", "");
        // tree-sitter-d ships its query file but not the Rust const;
        // vendored under assets/queries (see its README).
        add(
            "d",
            tree_sitter_d::LANGUAGE,
            include_str!("../assets/queries/d-highlights.scm"),
            "",
            "",
        );

        Self { configs }
    }

    fn get(&self, name: &str) -> Option<&HighlightConfiguration> {
        let canonical = match name {
            "rs" => "rust",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
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
            other => other,
        };
        self.configs
            .iter()
            .find(|(n, _)| *n == canonical)
            .map(|(_, c)| c)
    }

    /// Highlight `code`, returning (byte range, capture index) spans.
    pub fn highlight(&self, lang: &str, code: &str) -> Vec<(Range<usize>, u8)> {
        let Some(config) = self.get(lang) else {
            return Vec::new();
        };
        let mut highlighter = Highlighter::new();
        let Ok(events) = highlighter.highlight(config, code.as_bytes(), None, |injected| {
            self.get(injected)
        }) else {
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
                        if start < end && capture < CAPTURE_NAMES.len() {
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

//! Syntax highlighting via inkjet (78 bundled tree-sitter grammars with
//! Helix's highlight queries). Highlighting runs once when a document is
//! opened or edited (not per frame): each code block/file gets a list of
//! (byte range, capture index) spans that the view layer maps to theme
//! colors.

use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Global};
use inkjet::tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as RawHighlighter};
use inkjet::{Highlighter, Language};

use crate::markdown::{Block, Document};

/// Capture names (Helix scheme); a span's `u8` indexes into this table.
pub const CAPTURE_NAMES: &[&str] = inkjet::constants::HIGHLIGHT_NAMES;

/// Grammars inkjet doesn't bundle, built on its tree-sitter runtime and
/// configured with the same capture table.
pub struct Languages {
    extras: Vec<(&'static str, HighlightConfiguration)>,
}

impl Languages {
    pub fn new() -> Self {
        let mut extras = Vec::new();
        let mut add = |name: &'static str,
                       language: tree_sitter::Language,
                       highlights: &str| {
            match HighlightConfiguration::new(language, name, highlights, "", "") {
                Ok(mut config) => {
                    config.configure(CAPTURE_NAMES);
                    extras.push((name, config));
                }
                Err(err) => eprintln!("supermd: failed to load {name} grammar: {err}"),
            }
        };
        add(
            "xml",
            tree_sitter_xml::LANGUAGE_XML.into(),
            tree_sitter_xml::XML_HIGHLIGHT_QUERY,
        );
        // GraphQL blocked: its crate is generated at tree-sitter ABI 15,
        // inkjet's 0.23 runtime accepts <= 14. Revisit on inkjet bump.
        Self { extras }
    }

    fn canonical(name: &str) -> &str {
        match name {
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
        }
    }

    /// Highlight `code`, returning (byte range, capture index) spans.
    pub fn highlight(&self, lang: &str, code: &str) -> Vec<(Range<usize>, u8)> {
        let canonical = Self::canonical(lang);

        if let Some((_, config)) = self.extras.iter().find(|(n, _)| *n == canonical) {
            let mut highlighter = RawHighlighter::new();
            let Ok(events) = highlighter.highlight(config, code.as_bytes(), None, |_| None)
            else {
                return Vec::new();
            };
            return collect_spans(events.flatten());
        }

        let Some(language) = Language::from_token(canonical) else {
            return Vec::new();
        };
        // Plaintext has no queries; skip the parse entirely.
        if matches!(language, Language::Plaintext) {
            return Vec::new();
        }
        // A handful of inkjet grammars fail query compilation on some
        // platforms (their configs are lazy statics that panic on
        // first touch — e.g. julia under MSVC). Degrade to plain text
        // instead of crashing the app.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut highlighter = Highlighter::new();
            highlighter
                .highlight_raw(language, &code)
                .map(|events| collect_spans(events.flatten()))
                .unwrap_or_default()
        }));
        result.unwrap_or_else(|_| {
            eprintln!("supermd: grammar for '{canonical}' unavailable on this platform");
            Vec::new()
        })
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

fn collect_spans(events: impl Iterator<Item = HighlightEvent>) -> Vec<(Range<usize>, u8)> {
    let mut spans = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for event in events {
        match event {
            HighlightEvent::HighlightStart(h) => stack.push(h.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if let Some(&capture) = stack.last() {
                    if start < end
                        && capture < CAPTURE_NAMES.len()
                        && capture <= u8::MAX as usize
                    {
                        spans.push((start..end, capture as u8));
                    }
                }
            }
        }
    }
    spans
}

/// Language token (inkjet vocabulary) for a file, by exact filename
/// first, then extension. Covers every language inkjet bundles plus the
/// extras registry.
pub fn language_for_file(path: &std::path::Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    match name {
        "Dockerfile" | "dockerfile" | "Containerfile" => return Some("dockerfile"),
        "Makefile" | "makefile" | "GNUmakefile" => return Some("make"),
        "meson.build" => return Some("meson"),
        _ => {}
    }
    let ext = path.extension()?.to_str()?;
    Some(match ext {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "ts" => "typescript",
        "tsx" => "tsx",
        "py" => "python",
        "json" | "jsonc" | "json5" => "json",
        "sh" | "bash" | "zsh" => "bash",
        "toml" => "toml",
        "css" => "css",
        "scss" => "scss",
        "html" | "htm" => "html",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" | "cxx" => "cpp",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "swift" => "swift",
        "yml" | "yaml" => "yaml",
        "php" => "php",
        "cs" => "csharp",
        "lua" => "lua",
        "ex" | "exs" => "elixir",
        "hs" => "haskell",
        "ml" | "mli" => "ocaml",
        "scala" | "sc" => "scala",
        "zig" => "zig",
        "elm" => "elm",
        "erl" | "hrl" => "erlang",
        "sql" => "sql",
        "nix" => "nix",
        "r" | "R" => "r",
        "gleam" => "gleam",
        "svelte" => "svelte",
        "dart" => "dart",
        "d" => "d",
        "kt" | "kts" => "kotlin",
        "jl" => "julia",
        "clj" | "cljs" | "cljc" | "edn" => "clojure",
        "fish" => "fish",
        "vim" | "vimrc" => "vim",
        "tex" => "latex",
        "bib" => "bibtex",
        "ini" | "env" | "cfg" | "conf" => "ini",
        "rkt" => "racket",
        "scm" | "ss" => "scheme",
        "proto" => "protobuf",
        "gd" => "gdscript",
        "hcl" | "tf" | "tfvars" => "hcl",
        "cue" => "cue",
        "awk" => "awk",
        "f" | "f90" | "f95" | "f03" => "fortran",
        "pas" | "pp" => "pascal",
        "el" => "elisp",
        "diff" | "patch" => "diff",
        "wgsl" => "wgsl",
        "glsl" | "vert" | "frag" | "comp" => "glsl",
        "ll" => "llvm",
        "asm" | "s" => "asm",
        "m" | "mm" => "objc",
        "scad" => "openscad",
        "bicep" => "bicep",
        "ada" | "adb" | "ads" => "ada",
        "wat" => "wat",
        "wast" => "wast",
        "mk" => "make",
        "xml" => "xml",
        "graphql" | "gql" => "graphql",
        _ => return None,
    })
}

#[cfg(test)]
mod mapping_tests {
    use std::path::Path;

    fn f(p: &str) -> Option<&'static str> {
        super::language_for_file(Path::new(p))
    }

    #[test]
    fn maps_filenames_and_new_extensions() {
        assert_eq!(f("Dockerfile"), Some("dockerfile"));
        assert_eq!(f("Makefile"), Some("make"));
        assert_eq!(f("meson.build"), Some("meson"));
        assert_eq!(f("App.kt"), Some("kotlin"));
        assert_eq!(f("sim.jl"), Some("julia"));
        assert_eq!(f("core.clj"), Some("clojure"));
        assert_eq!(f("theme.scss"), Some("scss"));
        assert_eq!(f("main.tf"), Some("hcl"));
        assert_eq!(f("shader.wgsl"), Some("wgsl"));
        assert_eq!(f("view.m"), Some("objc"));
        assert_eq!(f("x.fs"), None); // F# vs Forth ambiguity
        assert_eq!(f("noext"), None);
    }

    #[test]
    fn legacy_extensions_still_map() {
        assert_eq!(f("main.rs"), Some("rust"));
        assert_eq!(f("app.tsx"), Some("tsx"));
        assert_eq!(f("q.sql"), Some("sql"));
        assert_eq!(f("conf.yaml"), Some("yaml"));
    }
}

pub struct SyntaxLanguages(pub Arc<Languages>);

impl Global for SyntaxLanguages {}

/// The shared language registry. Cheap to clone (Arc).
pub fn languages(cx: &App) -> Arc<Languages> {
    cx.global::<SyntaxLanguages>().0.clone()
}

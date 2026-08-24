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
        // GraphQL ships as a grammar PLUGIN (plugins/graphql) through
        // the wasm registry below — its native crate is ABI 15, above
        // inkjet's 0.23 runtime.
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

        // Plugin grammars answer only when no built-in claims the name
        // (built-ins always win collisions).
        if Language::from_token(canonical).is_none() {
            if let Some(spans) = plugin_highlight(canonical, code) {
                return spans;
            }
            return Vec::new();
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

// ── plugin grammar registry ───────────────────────────────────────────

/// Plugin grammars: name → compiled highlight config. The WasmStore
/// must sit in a parser during a parse (the API moves it), so it lives
/// beside the registry and is taken/put around each highlight.
/// WasmStore wraps a raw pointer and lacks Send; every access here is
/// serialized behind the GRAMMARS mutex and a store never crosses
/// threads mid-parse, so moving it between locked sections is sound.
struct SendStore(tree_sitter::WasmStore);
unsafe impl Send for SendStore {}

struct GrammarRegistry {
    store: Option<SendStore>,
    grammars: Vec<(String, HighlightConfiguration)>,
    /// extension → grammar name
    extensions: Vec<(String, String)>,
}

static GRAMMARS: std::sync::Mutex<Option<GrammarRegistry>> = std::sync::Mutex::new(None);

/// Load grammar plugins into the registry, replacing its contents
/// (reload semantics). Returns (plugin name, error) per failed grammar.
pub fn load_plugin_grammars(
    specs: &[(String, std::path::PathBuf, crate::extensions::GrammarInfo)],
) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    let engine = tree_sitter::wasmtime::Engine::default();
    let mut store = match tree_sitter::WasmStore::new(&engine) {
        Ok(s) => s,
        Err(e) => {
            *GRAMMARS.lock().unwrap() = None;
            return specs
                .iter()
                .map(|(p, ..)| (p.clone(), format!("wasm store: {e}")))
                .collect();
        }
    };
    let mut grammars = Vec::new();
    let mut extensions = Vec::new();
    for (plugin, dir, g) in specs {
        let (wasm_path, scm_path) = crate::extensions::grammar_paths(dir, g);
        let result = (|| -> Result<HighlightConfiguration, String> {
            let bytes = std::fs::read(&wasm_path).map_err(|e| e.to_string())?;
            let language = store
                .load_language(&g.name, &bytes)
                .map_err(|e| format!("grammar wasm: {e}"))?;
            let query = std::fs::read_to_string(&scm_path).map_err(|e| e.to_string())?;
            let mut config = HighlightConfiguration::new(language, &g.name, &query, "", "")
                .map_err(|e| format!("highlights.scm: {e}"))?;
            config.configure(CAPTURE_NAMES);
            Ok(config)
        })();
        match result {
            Ok(config) => {
                grammars.push((g.name.clone(), config));
                for ext in &g.extensions {
                    extensions.push((ext.clone(), g.name.clone()));
                }
            }
            Err(e) => failures.push((plugin.clone(), format!("grammar `{}`: {e}", g.name))),
        }
    }
    *GRAMMARS.lock().unwrap() = if grammars.is_empty() {
        None
    } else {
        Some(GrammarRegistry { store: Some(SendStore(store)), grammars, extensions })
    };
    failures
}

/// Grammar name a plugin registered for this file extension.
pub fn plugin_grammar_for_extension(ext: &str) -> Option<String> {
    let guard = GRAMMARS.lock().unwrap();
    let reg = guard.as_ref()?;
    reg.extensions.iter().find(|(e, _)| e == ext).map(|(_, n)| n.clone())
}

/// Highlight through a plugin grammar; None = not a plugin grammar.
fn plugin_highlight(name: &str, code: &str) -> Option<Vec<(Range<usize>, u8)>> {
    let mut guard = GRAMMARS.lock().unwrap();
    let reg = guard.as_mut()?;
    let ix = reg.grammars.iter().position(|(n, _)| n == name)?;
    let store = reg.store.take()?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut highlighter = RawHighlighter::new();
        if highlighter.parser().set_wasm_store(store.0).is_err() {
            return (None, Vec::new());
        }
        let config = &reg.grammars[ix].1;
        let spans = highlighter
            .highlight(config, code.as_bytes(), None, |_| None)
            .map(|events| collect_spans(events.flatten()))
            .unwrap_or_default();
        (highlighter.parser().take_wasm_store(), spans)
    }));
    match result {
        Ok((store_back, spans)) => {
            reg.store = store_back.map(SendStore);
            Some(spans)
        }
        Err(_) => {
            eprintln!("supermd: plugin grammar '{name}' failed; degrading to plain text");
            // The store was lost with the panicked parser; grammars
            // stop answering until the next Reload Plugins.
            Some(Vec::new())
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
/// first, then extension (static table, then plugin grammar registry).
pub fn language_for_file(path: &std::path::Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    match name {
        "Dockerfile" | "dockerfile" | "Containerfile" => return Some("dockerfile".to_string()),
        "Makefile" | "makefile" | "GNUmakefile" => return Some("make".to_string()),
        "meson.build" => return Some("meson".to_string()),
        _ => {}
    }
    let ext = path.extension()?.to_str()?;
    if let Some(known) = static_language_for_ext(ext) {
        return Some(known.to_string());
    }
    plugin_grammar_for_extension(ext)
}

fn static_language_for_ext(ext: &str) -> Option<&'static str> {
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
        // NOTE: no "graphql" here — it resolves through the plugin
        // grammar registry (the graphql grammar ships as a plugin).
        _ => return None,
    })
}

#[cfg(test)]
mod grammar_tests {
    use super::*;
    use std::path::PathBuf;

    fn graphql_spec() -> (String, PathBuf, crate::extensions::GrammarInfo) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/graphql");
        assert!(dir.join("grammar.wasm").exists(), "graphql artifact missing — see Task 1");
        (
            "graphql".to_string(),
            dir,
            crate::extensions::GrammarInfo {
                name: "graphql".into(),
                extensions: vec!["graphql".into(), "gql".into()],
                files: None,
            },
        )
    }

    /// One test, sequential scenarios: the registry is a process
    /// global and parallel tests would race it.
    #[test]
    fn grammar_registry_scenarios() {
        // 1. load + highlight + extension resolution
        let failures = load_plugin_grammars(&[graphql_spec()]);
        assert!(failures.is_empty(), "{failures:?}");
        let src = "type Query {\n  hero(episode: Episode): Character\n}\n";
        let spans = Languages::new().highlight("graphql", src);
        assert!(!spans.is_empty(), "no spans for graphql");
        assert_eq!(plugin_grammar_for_extension("gql"), Some("graphql".to_string()));
        assert_eq!(plugin_grammar_for_extension("nope"), None);

        // 2. broken highlights.scm → per-plugin failure, no panic
        let (plugin, dir, _) = graphql_spec();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::copy(dir.join("grammar.wasm"), tmp.path().join("grammar.wasm")).unwrap();
        std::fs::write(tmp.path().join("highlights.scm"), "(nonexistent_node_xyz) @x").unwrap();
        let failures = load_plugin_grammars(&[(
            plugin.clone(),
            tmp.path().to_path_buf(),
            crate::extensions::GrammarInfo {
                name: "brokenq".into(),
                extensions: vec![],
                files: None,
            },
        )]);
        assert_eq!(failures.len(), 1, "{failures:?}");

        // 3. builtins win name collisions. A wasm module can only load
        // under its own exported name (tree_sitter_<name>), so forge
        // the collision by renaming the registry entry to "rust": the
        // builtin must still answer for that token.
        let failures = load_plugin_grammars(&[(plugin, dir, {
            crate::extensions::GrammarInfo {
                name: "graphql".into(),
                extensions: vec![],
                files: None,
            }
        })]);
        assert!(failures.is_empty(), "{failures:?}");
        GRAMMARS.lock().unwrap().as_mut().unwrap().grammars[0].0 = "rust".to_string();
        let spans = Languages::new().highlight("rust", "fn main() { let x = 1; }");
        assert!(!spans.is_empty(), "builtin rust must still highlight");

        // 4. file resolution goes through the registry
        let (plugin, dir, g) = graphql_spec();
        load_plugin_grammars(&[(plugin, dir, g)]);
        assert_eq!(
            language_for_file(std::path::Path::new("schema.graphql")),
            Some("graphql".to_string())
        );
        assert_eq!(
            language_for_file(std::path::Path::new("q.gql")),
            Some("graphql".to_string())
        );

        // 5. reload with nothing clears resolution + spans
        load_plugin_grammars(&[]);
        assert_eq!(plugin_grammar_for_extension("graphql"), None);
        assert_eq!(language_for_file(std::path::Path::new("schema.graphql")), None);
        assert!(Languages::new().highlight("graphql", src).is_empty());
    }
}

#[cfg(test)]
mod mapping_tests {
    use std::path::Path;

    fn f(p: &str) -> Option<String> {
        super::language_for_file(Path::new(p))
    }

    #[test]
    fn maps_filenames_and_new_extensions() {
        assert_eq!(f("Dockerfile").as_deref(), Some("dockerfile"));
        assert_eq!(f("Makefile").as_deref(), Some("make"));
        assert_eq!(f("meson.build").as_deref(), Some("meson"));
        assert_eq!(f("App.kt").as_deref(), Some("kotlin"));
        assert_eq!(f("sim.jl").as_deref(), Some("julia"));
        assert_eq!(f("core.clj").as_deref(), Some("clojure"));
        assert_eq!(f("theme.scss").as_deref(), Some("scss"));
        assert_eq!(f("main.tf").as_deref(), Some("hcl"));
        assert_eq!(f("shader.wgsl").as_deref(), Some("wgsl"));
        assert_eq!(f("view.m").as_deref(), Some("objc"));
        assert_eq!(f("x.fs"), None); // F# vs Forth ambiguity
        assert_eq!(f("noext"), None);
    }

    #[test]
    fn legacy_extensions_still_map() {
        assert_eq!(f("main.rs").as_deref(), Some("rust"));
        assert_eq!(f("app.tsx").as_deref(), Some("tsx"));
        assert_eq!(f("q.sql").as_deref(), Some("sql"));
        assert_eq!(f("conf.yaml").as_deref(), Some("yaml"));
    }

    #[test]
    fn plaintext_produces_no_spans() {
        let langs = super::Languages::new();
        assert!(langs.highlight("plaintext", "let x = 1;").is_empty());
    }

    #[test]
    fn highlight_document_reaches_nested_blocks() {
        use crate::markdown::{parse, Block};
        let md = "```rust\nfn main() {}\n```\n\n\
                  > ```rust\n> let q = 1;\n> ```\n\n\
                  - ```rust\n  let l = 2;\n  ```\n\n\
                  plain paragraph\n\n\
                  ```\nno lang\n```\n";
        let mut doc = parse(md);
        super::Languages::new().highlight_document(&mut doc);
        let Block::Code { spans, .. } = &doc.blocks[0] else { panic!("expected top-level code") };
        assert!(!spans.is_empty());
        let Block::Quote(inner) = &doc.blocks[1] else { panic!("expected quote") };
        let Block::Code { spans, .. } = &inner[0] else { panic!("expected quoted code") };
        assert!(!spans.is_empty());
        let Block::List { items, .. } = &doc.blocks[2] else { panic!("expected list") };
        let Block::Code { spans, .. } = &items[0].blocks[0] else { panic!("expected list code") };
        assert!(!spans.is_empty());
        let Block::Code { lang, spans, .. } = &doc.blocks[4] else { panic!("expected bare code") };
        assert!(lang.is_none() && spans.is_empty());
    }
}

pub struct SyntaxLanguages(pub Arc<Languages>);

impl Global for SyntaxLanguages {}

/// The shared language registry. Cheap to clone (Arc).
pub fn languages(cx: &App) -> Arc<Languages> {
    cx.global::<SyntaxLanguages>().0.clone()
}

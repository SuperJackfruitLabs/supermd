//! The Editor: GPUI shell around the tested core. Renders one logical
//! line per virtualized list item with styled-source typography.

pub mod autosave;
pub mod buffer;
pub mod core;
pub mod movement;
pub mod spans;

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui::prelude::*;
use gpui::{
    div, list, px, relative, App, FocusHandle, Focusable, Font, FontFeatures, FontStyle,
    FontWeight, Hsla, IntoElement, ListAlignment, ListOffset, ListState, Render, SharedString,
    StrikethroughStyle, StyledText, TextRun, UnderlineStyle, Window,
};

use crate::highlight::Languages;
use crate::reader::language_for_path;
use crate::theme::{theme, Theme};
use autosave::SavePolicy;
use core::EditorCore;
use spans::{LineKind, StyleKind, StyleSpan};

enum Provider {
    Markdown,
    Code(&'static str),
    Plain,
}

pub struct Editor {
    core: EditorCore,
    provider: Provider,
    spans: Vec<StyleSpan>,
    line_kinds: Vec<LineKind>,
    path: PathBuf,
    pub save: SavePolicy,
    pub disk_mtime: Option<SystemTime>,
    list_state: ListState,
    focus_handle: FocusHandle,
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "markdown" | "mdown" | "mdx")
    )
}

impl Editor {
    /// Read a file and build an editor for it. Call `from_text` directly
    /// when constructing inside `cx.new` (which cannot be fallible).
    pub fn read_file(path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub fn from_text(path: &Path, text: String, langs: &Languages, cx: &mut Context<Self>) -> Self {
        let provider = if is_markdown(path) {
            Provider::Markdown
        } else if let Some(lang) = language_for_path(path) {
            Provider::Code(lang)
        } else {
            Provider::Plain
        };
        let core = EditorCore::new(&text);
        let line_count = core.buffer.line_count();
        let mut editor = Self {
            core,
            provider,
            spans: Vec::new(),
            line_kinds: Vec::new(),
            path: path.to_path_buf(),
            save: SavePolicy::default(),
            disk_mtime: autosave::disk_mtime(path),
            list_state: ListState::new(line_count, ListAlignment::Top, px(512.)),
            focus_handle: cx.focus_handle(),
        };
        editor.restyle(langs);
        editor
    }

    pub fn text(&self) -> String {
        self.core.buffer.text()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn title(&self) -> SharedString {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
            .into()
    }

    fn restyle(&mut self, langs: &Languages) {
        let text = self.core.buffer.text();
        self.spans = match self.provider {
            Provider::Markdown => spans::markdown_spans_highlighted(&text, langs),
            Provider::Code(lang) => spans::code_spans(&text, lang, langs),
            Provider::Plain => Vec::new(),
        };
        self.line_kinds = spans::line_kinds(&text, &self.spans);
        self.list_state.reset(self.core.buffer.line_count());
    }

    pub fn heading_lines(&self) -> Vec<(u8, String, usize)> {
        self.spans
            .iter()
            .filter_map(|s| match s.kind {
                StyleKind::Heading(level) => {
                    let line = self.core.buffer.line_of_byte(s.range.start);
                    let text = self
                        .core
                        .buffer
                        .slice(s.range.clone())
                        .trim_start_matches('#')
                        .trim()
                        .to_string();
                    Some((level, text, line))
                }
                _ => None,
            })
            .collect()
    }

    pub fn scroll_to_line(&mut self, ix: usize) {
        self.list_state
            .scroll_to(ListOffset { item_ix: ix, offset_in_item: px(0.) });
    }

    // ── styling ────────────────────────────────────────────────────────

    /// (font size, base weight, family, line height multiple) for a line.
    fn line_typography(&self, ix: usize, t: &Theme) -> (f32, FontWeight, SharedString, f32) {
        match self.line_kinds.get(ix) {
            Some(LineKind::Heading(n)) => {
                let weight = if *n <= 2 { FontWeight::BOLD } else { FontWeight::SEMIBOLD };
                (t.heading_size(*n), weight, t.body_family.clone(), 1.35)
            }
            Some(LineKind::Code) => (t.code_size, FontWeight::NORMAL, t.mono_family.clone(), 1.55),
            _ => (t.body_size, FontWeight::NORMAL, t.body_family.clone(), 1.65),
        }
    }

    fn syntax_color(capture: u8, t: &Theme) -> Option<Hsla> {
        let name = crate::highlight::CAPTURE_NAMES.get(capture as usize)?;
        let root = name.split('.').next().unwrap_or(name);
        let s = &t.syntax;
        Some(match root {
            "attribute" => s.attribute,
            "comment" => s.comment,
            "constant" | "number" => s.constant,
            "constructor" | "type" => s.kind,
            "function" => s.function,
            "keyword" => s.keyword,
            "operator" | "punctuation" => s.operator,
            "property" => s.property,
            "string" => s.string,
            "tag" => s.tag,
            _ => return None,
        })
    }

    /// The styled TextRuns for one line (covering every byte of it).
    fn line_runs(&self, ix: usize, t: &Theme) -> (SharedString, Vec<TextRun>) {
        let range = self.core.buffer.line_range(ix);
        let text = self.core.buffer.line_text(ix);
        let (_, base_weight, family, _) = self.line_typography(ix, t);

        #[derive(Clone, PartialEq)]
        struct Attr {
            color: Hsla,
            weight: FontWeight,
            italic: bool,
            family: SharedString,
            bg: Option<Hsla>,
            underline: bool,
            strike: bool,
        }
        let default_attr = Attr {
            color: t.fg,
            weight: base_weight,
            italic: false,
            family: family.clone(),
            bg: None,
            underline: false,
            strike: false,
        };
        let mut attrs: Vec<Attr> = vec![default_attr; text.len()];
        for span in &self.spans {
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            if start >= end {
                continue;
            }
            for a in &mut attrs[start - range.start..end - range.start] {
                match &span.kind {
                    StyleKind::Heading(_) => a.color = t.fg_strong,
                    StyleKind::Strong => a.weight = FontWeight::BOLD,
                    StyleKind::Emphasis => a.italic = true,
                    StyleKind::Strikethrough => a.strike = true,
                    StyleKind::InlineCode => {
                        a.family = t.mono_family.clone();
                        a.bg = Some(t.code_bg);
                        a.color = t.code_fg;
                    }
                    StyleKind::Link => {
                        a.color = t.link;
                        a.underline = true;
                    }
                    StyleKind::ListMarker | StyleKind::QuoteMarker => a.color = t.accent,
                    StyleKind::Rule => a.color = t.fg_muted,
                    StyleKind::FenceContent => a.color = t.code_fg,
                    StyleKind::Syntax(capture) => {
                        if let Some(c) = Self::syntax_color(*capture, t) {
                            a.color = c;
                        }
                        if crate::highlight::CAPTURE_NAMES
                            .get(*capture as usize)
                            .is_some_and(|n| n.starts_with("comment"))
                        {
                            a.italic = true;
                        }
                    }
                }
            }
        }

        let font_of = |a: &Attr| Font {
            family: a.family.clone(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: a.weight,
            style: if a.italic { FontStyle::Italic } else { FontStyle::Normal },
        };

        let mut runs: Vec<TextRun> = Vec::new();
        let mut i = 0;
        while i < attrs.len() {
            let mut j = i + 1;
            while j < attrs.len() && attrs[j] == attrs[i] {
                j += 1;
            }
            let a = &attrs[i];
            runs.push(TextRun {
                len: j - i,
                font: font_of(a),
                color: a.color,
                background_color: a.bg,
                underline: a.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(a.color),
                    wavy: false,
                }),
                strikethrough: a.strike.then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(t.fg_muted),
                }),
            });
            i = j;
        }
        (SharedString::from(text), runs)
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.weak_entity();
        let t = theme(cx);
        div()
            .size_full()
            .bg(t.bg)
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .child(
                list(self.list_state.clone(), move |ix, _window, cx| {
                    let Some(editor) = entity.upgrade() else {
                        return div().into_any_element();
                    };
                    let t = theme(cx);
                    let editor = editor.read(cx);
                    let (size, _, family, line_height) = editor.line_typography(ix, &t);
                    let (text, runs) = editor.line_runs(ix, &t);
                    let is_code = matches!(editor.line_kinds.get(ix), Some(LineKind::Code));
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .child(
                            div()
                                .w_full()
                                .max_w(px(760.))
                                .px(px(48.))
                                .when(ix == 0, |d| d.pt(px(40.)))
                                .when(ix + 1 == editor.core.buffer.line_count(), |d| {
                                    d.pb(px(96.))
                                })
                                .font_family(family)
                                .text_size(px(size))
                                .line_height(relative(line_height))
                                .when(is_code, |d| d.bg(t.code_bg))
                                .child(if text.is_empty() {
                                    div().h(px(size * line_height)).into_any_element()
                                } else {
                                    StyledText::new(text).with_runs(runs).into_any_element()
                                }),
                        )
                        .into_any_element()
                })
                .size_full(),
            )
    }
}

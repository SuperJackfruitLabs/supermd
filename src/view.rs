//! Block model → GPUI elements. Pure presentation; no editing state yet.

use gpui::prelude::*;
use gpui::{
    div, px, relative, AnyElement, Font, FontFeatures, FontStyle, FontWeight, Hsla,
    IntoElement, ParentElement, SharedString, StrikethroughStyle, Styled, StyledText, TextRun,
    UnderlineStyle,
};

use crate::markdown::{Block, Document, InlineText, ListItem, SpanStyle};
use crate::theme::Theme;

/// Base typography for an inline run, before span styles apply.
#[derive(Clone, Copy)]
struct BaseStyle {
    weight: FontWeight,
    color: Hsla,
}

fn font(family: &SharedString, weight: FontWeight, style: FontStyle) -> Font {
    Font {
        family: family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight,
        style,
    }
}

/// Build the exact sequence of `TextRun`s covering `inline.text`.
fn runs_for(inline: &InlineText, base: BaseStyle, t: &Theme) -> Vec<TextRun> {
    let plain = |len: usize| TextRun {
        len,
        font: font(&t.body_family, base.weight, FontStyle::Normal),
        color: base.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };

    let styled = |len: usize, s: SpanStyle| {
        let family = if s.code { &t.mono_family } else { &t.body_family };
        let weight = if s.bold && base.weight < FontWeight::BOLD {
            FontWeight::BOLD
        } else {
            base.weight
        };
        let style = if s.italic { FontStyle::Italic } else { FontStyle::Normal };
        let color = if s.link {
            t.link
        } else if s.code {
            t.code_fg
        } else {
            base.color
        };
        TextRun {
            len,
            font: font(family, weight, style),
            color,
            background_color: s.code.then_some(t.code_bg),
            underline: s.link.then_some(UnderlineStyle {
                thickness: px(1.),
                color: Some(t.link),
                wavy: false,
            }),
            strikethrough: s.strike.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(t.fg_muted),
            }),
        }
    };

    let mut runs = Vec::new();
    let mut cursor = 0;
    for (range, span) in &inline.spans {
        if cursor < range.start {
            runs.push(plain(range.start - cursor));
        }
        runs.push(styled(range.len(), *span));
        cursor = range.end;
    }
    if cursor < inline.text.len() {
        runs.push(plain(inline.text.len() - cursor));
    }
    runs
}

fn inline_text(inline: &InlineText, base: BaseStyle, t: &Theme) -> StyledText {
    let runs = runs_for(inline, base, t);
    StyledText::new(inline.text.clone()).with_runs(runs)
}

fn paragraph(inline: &InlineText, t: &Theme) -> AnyElement {
    div()
        .text_size(px(t.body_size))
        .line_height(relative(t.body_line_height))
        .child(inline_text(
            inline,
            BaseStyle { weight: FontWeight::NORMAL, color: t.fg },
            t,
        ))
        .into_any_element()
}

fn heading(level: u8, content: &InlineText, t: &Theme) -> AnyElement {
    let weight = if level <= 2 { FontWeight::BOLD } else { FontWeight::SEMIBOLD };
    let margin_top = match level {
        1 => px(28.),
        2 => px(22.),
        _ => px(14.),
    };
    div()
        .mt(margin_top)
        .text_size(px(t.heading_size(level)))
        .line_height(relative(1.3))
        .child(inline_text(content, BaseStyle { weight, color: t.fg_strong }, t))
        .into_any_element()
}

/// Map a tree-sitter capture name to its color; None means default code color.
fn capture_color(capture: u8, t: &Theme) -> Option<Hsla> {
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
        "variable" => match *name {
            "variable.builtin" => s.property,
            _ => return None,
        },
        _ => return None,
    })
}

/// Build text runs for highlighted code. Spans may nest; later spans that
/// overlap already-covered bytes are skipped.
fn code_runs(code: &str, spans: &[(std::ops::Range<usize>, u8)], t: &Theme) -> Vec<TextRun> {
    let mono = |color: Hsla, italic: bool| Font {
        family: t.mono_family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: if italic { FontStyle::Italic } else { FontStyle::Normal },
    };
    let run = |len: usize, color: Hsla, italic: bool| TextRun {
        len,
        font: mono(color, italic),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };

    let mut runs = Vec::new();
    let mut cursor = 0;
    for (range, capture) in spans {
        if range.start < cursor || range.end > code.len() {
            continue;
        }
        if cursor < range.start {
            runs.push(run(range.start - cursor, t.code_fg, false));
        }
        let is_comment = crate::highlight::CAPTURE_NAMES
            .get(*capture as usize)
            .is_some_and(|n| n.starts_with("comment"));
        let color = capture_color(*capture, t).unwrap_or(t.code_fg);
        runs.push(run(range.len(), color, is_comment));
        cursor = range.end;
    }
    if cursor < code.len() {
        runs.push(run(code.len() - cursor, t.code_fg, false));
    }
    runs
}

fn code_block(
    lang: Option<&str>,
    code: &str,
    spans: &[(std::ops::Range<usize>, u8)],
    t: &Theme,
) -> AnyElement {
    let mut container = div()
        .rounded_lg()
        .bg(t.code_bg)
        .px_4()
        .py_3()
        .flex()
        .flex_col()
        .gap_1();

    if let Some(lang) = lang {
        container = container.child(
            div()
                .text_size(px(11.))
                .text_color(t.fg_muted)
                .child(SharedString::from(lang.to_uppercase())),
        );
    }

    let text = StyledText::new(code.to_string()).with_runs(code_runs(code, spans, t));

    container
        .child(
            div()
                .font_family(t.mono_family.clone())
                .text_size(px(t.code_size))
                .line_height(relative(1.55))
                .text_color(t.code_fg)
                .child(text),
        )
        .into_any_element()
}

fn quote(blocks: &[Block], t: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .gap_4()
        .child(div().w(px(3.)).rounded_full().bg(t.accent).flex_none())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .text_color(t.fg_muted)
                .children(blocks.iter().map(|b| block(b, t))),
        )
        .into_any_element()
}

fn list(start: Option<u64>, items: &[ListItem], t: &Theme) -> AnyElement {
    let rows = items.iter().enumerate().map(|(index, item)| {
        let marker: AnyElement = match (item.checked, start) {
            (Some(done), _) => div()
                .text_size(px(t.body_size))
                .text_color(if done { t.accent } else { t.fg_muted })
                .child(if done { "✓" } else { "○" })
                .into_any_element(),
            (None, Some(first)) => div()
                .text_size(px(t.body_size))
                .text_color(t.fg_muted)
                .child(SharedString::from(format!("{}.", first + index as u64)))
                .into_any_element(),
            (None, None) => div()
                .text_size(px(t.body_size))
                .text_color(t.accent)
                .child("•")
                .into_any_element(),
        };

        div()
            .flex()
            .flex_row()
            .gap_2()
            .child(
                div()
                    .min_w(px(22.))
                    .flex_none()
                    .line_height(relative(t.body_line_height))
                    .child(marker),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(item.blocks.iter().map(|b| block(b, t))),
            )
    });

    div().flex().flex_col().gap_1().children(rows).into_any_element()
}

fn table(head: &[InlineText], rows: &[Vec<InlineText>], t: &Theme) -> AnyElement {
    let cell_base = BaseStyle { weight: FontWeight::NORMAL, color: t.fg };
    let head_base = BaseStyle { weight: FontWeight::SEMIBOLD, color: t.fg_strong };

    let render_row = |cells: &[InlineText], base: BaseStyle, t: &Theme| {
        div()
            .flex()
            .flex_row()
            .children(cells.iter().map(|cell| {
                div()
                    .flex_1()
                    .px_3()
                    .py_2()
                    .text_size(px(t.body_size - 1.))
                    .line_height(relative(1.45))
                    .child(inline_text(cell, base, t))
            }))
    };

    div()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .flex()
        .flex_col()
        .child(render_row(head, head_base, t).bg(t.code_bg).rounded_t_lg())
        .children(
            rows.iter()
                .map(|row| render_row(row, cell_base, t).border_t_1().border_color(t.border)),
        )
        .into_any_element()
}

fn rule(t: &Theme) -> AnyElement {
    div().my_2().h(px(1.)).w_full().bg(t.border).into_any_element()
}

fn block(b: &Block, t: &Theme) -> AnyElement {
    match b {
        Block::Paragraph(inline) => paragraph(inline, t),
        Block::Heading { level, content } => heading(*level, content, t),
        Block::Code { lang, code, spans } => code_block(lang.as_deref(), code, spans, t),
        Block::Quote(blocks) => quote(blocks, t),
        Block::List { start, items } => list(*start, items, t),
        Block::Table { head, rows } => table(head, rows, t),
        Block::Rule => rule(t),
    }
}

/// One top-level block as a list item, constrained to the reading column.
pub fn list_item(doc: &Document, ix: usize, t: &Theme) -> AnyElement {
    let Some(b) = doc.blocks.get(ix) else {
        return div().into_any_element();
    };
    let first = ix == 0;
    let last = ix + 1 == doc.blocks.len();
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
                .when(first, |d| d.pt(px(40.)))
                .pb(if last { px(96.) } else { px(12.) })
                .child(block(b, t)),
        )
        .into_any_element()
}

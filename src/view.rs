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
        "namespace" => s.kind,
        "label" => s.constant,
        "special" => s.string,
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
    let mono = |italic: bool| Font {
        family: t.mono_family.clone(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: if italic { FontStyle::Italic } else { FontStyle::Normal },
    };
    let run = |len: usize, color: Hsla, italic: bool| TextRun {
        len,
        font: mono(italic),
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
    cx: &mut gpui::App,
) -> AnyElement {
    if lang == Some("mermaid") {
        return diagram_block(code, t, cx);
    }
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

/// Mermaid fences in the reader render through the shared diagram
/// cache: image when ready, quiet box while pending, error strip +
/// plain code on failure.
fn diagram_block(code: &str, t: &Theme, cx: &mut gpui::App) -> AnyElement {
    match crate::diagram::diagram_state(code, 664.0, cx) {
        crate::diagram::DiagramState::Ready(image) => div()
            .w_full()
            .flex()
            .justify_center()
            .child(gpui::img(image).max_w_full().rounded_md())
            .into_any_element(),
        crate::diagram::DiagramState::Pending => div()
            .w_full()
            .min_h(px(120.))
            .rounded_lg()
            .bg(t.code_bg)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(t.fg_muted)
            .child("diagram…")
            .into_any_element(),
        crate::diagram::DiagramState::Failed(msg) => div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_t_lg()
                    .bg(t.diff_deleted_bg)
                    .text_size(px(11.))
                    .text_color(t.diff_deleted_fg)
                    .child(SharedString::from(format!("diagram error: {msg}"))),
            )
            .child(
                div()
                    .px_4()
                    .py_3()
                    .rounded_b_lg()
                    .bg(t.code_bg)
                    .font_family(t.mono_family.clone())
                    .text_size(px(t.code_size))
                    .line_height(relative(1.55))
                    .text_color(t.code_fg)
                    .child(SharedString::from(code.to_string())),
            )
            .into_any_element(),
    }
}

fn quote(blocks: &[Block], t: &Theme, cx: &mut gpui::App) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .gap_4()
        .child(div().w(px(3.)).rounded_full().bg(t.accent).flex_none())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .text_color(t.fg_muted)
                .children(blocks.iter().map(|b| block(b, t, cx)).collect::<Vec<_>>()),
        )
        .into_any_element()
}

fn list(start: Option<u64>, items: &[ListItem], t: &Theme, cx: &mut gpui::App) -> AnyElement {
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
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(item.blocks.iter().map(|b| block(b, t, cx)).collect::<Vec<_>>()),
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
                    .min_w_0()
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

fn block(b: &Block, t: &Theme, cx: &mut gpui::App) -> AnyElement {
    match b {
        Block::Paragraph(inline) => paragraph(inline, t),
        Block::Heading { level, content } => heading(*level, content, t),
        Block::Code { lang, code, spans } => code_block(lang.as_deref(), code, spans, t, cx),
        Block::Quote(blocks) => quote(blocks, t, cx),
        Block::List { start, items } => list(*start, items, t, cx),
        Block::Table { head, rows } => table(head, rows, t),
        Block::Rule => rule(t),
    }
}

/// One top-level block as a list item, constrained to the reading column.
pub fn list_item(doc: &Document, ix: usize, t: &Theme, cx: &mut gpui::App) -> AnyElement {
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
                .child(block(b, t, cx)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::CAPTURE_NAMES;
    use crate::markdown;
    use crate::theme::Theme;
    use gpui::TestAppContext;
    use std::sync::Arc;

    fn cap(name: &str) -> u8 {
        CAPTURE_NAMES
            .iter()
            .position(|n| *n == name)
            .unwrap_or_else(|| panic!("capture name {name} missing")) as u8
    }

    fn inline(text: &str, spans: Vec<(std::ops::Range<usize>, SpanStyle)>) -> InlineText {
        InlineText { text: text.to_string(), spans }
    }

    fn body(t: &Theme) -> BaseStyle {
        BaseStyle { weight: FontWeight::NORMAL, color: t.fg }
    }

    /// Every run sequence must tile the text exactly: contiguous, no gaps.
    fn assert_covers(runs: &[TextRun], len: usize) {
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), len);
        assert!(runs.iter().all(|r| r.len > 0), "no empty runs");
    }

    // ── runs_for ───────────────────────────────────────────────────────

    #[test]
    fn plain_text_is_a_single_body_run() {
        let t = Theme::dark();
        let runs = runs_for(&inline("hello", vec![]), body(&t), &t);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, 5);
        assert_eq!(runs[0].font.family, t.body_family);
        assert_eq!(runs[0].color, t.fg);
        assert!(runs[0].underline.is_none());
        assert!(runs[0].strikethrough.is_none());
        assert!(runs[0].background_color.is_none());
    }

    #[test]
    fn interior_bold_span_splits_plain_bold_plain() {
        let t = Theme::dark();
        let s = SpanStyle { bold: true, ..Default::default() };
        let text = "plain bold tail";
        let runs = runs_for(&inline(text, vec![(6..10, s)]), body(&t), &t);
        assert_eq!(runs.len(), 3);
        assert_eq!([runs[0].len, runs[1].len, runs[2].len], [6, 4, 5]);
        assert_eq!(runs[0].font.weight, FontWeight::NORMAL);
        assert_eq!(runs[1].font.weight, FontWeight::BOLD);
        assert_eq!(runs[2].font.weight, FontWeight::NORMAL);
        assert_covers(&runs, text.len());
    }

    #[test]
    fn leading_span_omits_empty_plain_prefix() {
        let t = Theme::dark();
        let s = SpanStyle { italic: true, ..Default::default() };
        let runs = runs_for(&inline("it tail", vec![(0..2, s)]), body(&t), &t);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].font.style, FontStyle::Italic);
        assert_eq!(runs[1].font.style, FontStyle::Normal);
        assert_covers(&runs, 7);
    }

    #[test]
    fn code_span_uses_mono_family_and_code_colors() {
        let t = Theme::dark();
        let s = SpanStyle { code: true, ..Default::default() };
        let runs = runs_for(&inline("xy", vec![(0..2, s)]), body(&t), &t);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].font.family, t.mono_family);
        assert_eq!(runs[0].color, t.code_fg);
        assert_eq!(runs[0].background_color, Some(t.code_bg));
    }

    #[test]
    fn link_span_is_underlined_in_link_color() {
        let t = Theme::dark();
        let s = SpanStyle { link: true, ..Default::default() };
        let runs = runs_for(&inline("go", vec![(0..2, s)]), body(&t), &t);
        assert_eq!(runs[0].color, t.link);
        let Some(underline) = &runs[0].underline else { panic!("link not underlined") };
        assert_eq!(underline.color, Some(t.link));
    }

    #[test]
    fn strike_span_carries_strikethrough() {
        let t = Theme::dark();
        let s = SpanStyle { strike: true, ..Default::default() };
        let runs = runs_for(&inline("old", vec![(0..3, s)]), body(&t), &t);
        let Some(strike) = &runs[0].strikethrough else { panic!("no strikethrough") };
        assert_eq!(strike.color, Some(t.fg_muted));
    }

    #[test]
    fn bold_span_never_lightens_an_already_bold_base() {
        let t = Theme::dark();
        let s = SpanStyle { bold: true, ..Default::default() };
        // Normal base upgrades to bold…
        let runs = runs_for(
            &inline("x", vec![(0..1, s)]),
            BaseStyle { weight: FontWeight::NORMAL, color: t.fg },
            &t,
        );
        assert_eq!(runs[0].font.weight, FontWeight::BOLD);
        // …but a heavier base (e.g. an H1) keeps its weight.
        let runs = runs_for(
            &inline("x", vec![(0..1, s)]),
            BaseStyle { weight: FontWeight::EXTRA_BOLD, color: t.fg },
            &t,
        );
        assert_eq!(runs[0].font.weight, FontWeight::EXTRA_BOLD);
    }

    #[test]
    fn parsed_paragraph_with_crossing_styles_tiles_exactly() {
        let t = Theme::dark();
        let doc = markdown::parse(
            "Mix **bold**, *it*, `code`, ~~strike~~, [link](https://e.com), ![alt](x.png) end.\n",
        );
        let markdown::Block::Paragraph(para) = &doc.blocks[0] else { panic!("expected paragraph") };
        let runs = runs_for(para, body(&t), &t);
        assert_covers(&runs, para.text.len());
        assert!(runs.iter().any(|r| r.font.weight == FontWeight::BOLD));
        assert!(runs.iter().any(|r| r.font.style == FontStyle::Italic));
        assert!(runs.iter().any(|r| r.background_color == Some(t.code_bg)));
        assert!(runs.iter().any(|r| r.underline.is_some()));
        assert!(runs.iter().any(|r| r.strikethrough.is_some()));
    }

    // ── capture_color ──────────────────────────────────────────────────

    #[test]
    fn capture_roots_map_to_syntax_palette() {
        let t = Theme::dark();
        let s = &t.syntax;
        assert_eq!(capture_color(cap("keyword"), &t), Some(s.keyword));
        assert_eq!(capture_color(cap("keyword.control.import"), &t), Some(s.keyword));
        assert_eq!(capture_color(cap("comment.line"), &t), Some(s.comment));
        assert_eq!(capture_color(cap("constant.numeric"), &t), Some(s.constant));
        assert_eq!(capture_color(cap("type.builtin"), &t), Some(s.kind));
        assert_eq!(capture_color(cap("constructor"), &t), Some(s.kind));
        assert_eq!(capture_color(cap("namespace"), &t), Some(s.kind));
        assert_eq!(capture_color(cap("function.macro"), &t), Some(s.function));
        assert_eq!(capture_color(cap("operator"), &t), Some(s.operator));
        assert_eq!(capture_color(cap("punctuation.bracket"), &t), Some(s.operator));
        assert_eq!(capture_color(cap("string.special"), &t), Some(s.string));
        assert_eq!(capture_color(cap("special"), &t), Some(s.string));
        assert_eq!(capture_color(cap("tag"), &t), Some(s.tag));
        assert_eq!(capture_color(cap("attribute"), &t), Some(s.attribute));
        assert_eq!(capture_color(cap("label"), &t), Some(s.constant));
        assert_eq!(capture_color(cap("variable.builtin"), &t), Some(s.property));
    }

    #[test]
    fn unstyled_captures_fall_back_to_default_code_color() {
        let t = Theme::dark();
        // A bare variable and unknown roots take the default code color.
        assert_eq!(capture_color(cap("variable"), &t), None);
        assert_eq!(capture_color(cap("markup.heading"), &t), None);
        // Out-of-range capture indices are ignored gracefully.
        assert_eq!(capture_color(200, &t), None);
    }

    // ── code_runs ──────────────────────────────────────────────────────

    #[test]
    fn unhighlighted_code_is_one_default_run() {
        let t = Theme::dark();
        let code = "plain text";
        let runs = code_runs(code, &[], &t);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, code.len());
        assert_eq!(runs[0].color, t.code_fg);
        assert_eq!(runs[0].font.family, t.mono_family);
    }

    #[test]
    fn highlight_spans_color_their_ranges_and_gaps_stay_default() {
        let t = Theme::dark();
        let code = "let x = 1; // c";
        let spans = vec![(0..3, cap("keyword")), (11..15, cap("comment"))];
        let runs = code_runs(code, &spans, &t);
        assert_eq!(runs.len(), 3);
        assert_covers(&runs, code.len());
        assert_eq!(runs[0].color, t.syntax.keyword);
        assert_eq!(runs[0].font.style, FontStyle::Normal);
        assert_eq!(runs[1].color, t.code_fg, "gap between spans is plain");
        assert_eq!(runs[2].color, t.syntax.comment);
        assert_eq!(runs[2].font.style, FontStyle::Italic, "comments italicize");
    }

    #[test]
    fn overlapping_and_out_of_bounds_spans_are_skipped() {
        let t = Theme::dark();
        let code = "0123456789";
        let spans = vec![
            (0..4, cap("keyword")),
            (2..6, cap("keyword")), // overlaps the first: skipped
            (8..99, cap("keyword")), // past the end: skipped
        ];
        let runs = code_runs(code, &spans, &t);
        assert_eq!(runs.len(), 2);
        assert_eq!([runs[0].len, runs[1].len], [4, 6]);
        assert_covers(&runs, code.len());
    }

    #[test]
    fn unknown_capture_defaults_to_code_color() {
        let t = Theme::dark();
        let runs = code_runs("abc", &[(0..3, cap("markup"))], &t);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].color, t.code_fg);
    }

    // ── element construction over a full document ──────────────────────

    const KITCHEN_SINK: &str = r#"# Alpha *lead*

## Beta

### Gamma

#### Delta

Mix of **bold**, *italic*, `code`, ~~strike~~, [link](https://example.com), ![alt](x.png).
Soft wrapped line
Hard break\
end.

---

> quoted paragraph
>
> > nested quote

Bullets:

- one
- two **strong**

Ordered from five:

5. five
6. six

Tasks:

- [x] done
- [ ] todo

Loose:

* loose head

  second paragraph in item

| Head **A** | Head B |
| --- | --- |
| cell `x` | plain |

```rust
fn main() { let x = 1; }
```

```
no language
```
"#;

    fn parsed_sink() -> Document {
        let mut doc = markdown::parse(KITCHEN_SINK);
        crate::highlight::Languages::new().highlight_document(&mut doc);
        doc
    }

    #[test]
    fn kitchen_sink_parses_every_block_kind_the_view_renders() {
        let doc = parsed_sink();
        let blocks = &doc.blocks;

        let heading_levels: Vec<u8> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { level, .. } => Some(*level),
                _ => None,
            })
            .collect();
        assert_eq!(heading_levels, [1, 2, 3, 4]);

        // The mixed paragraph keeps soft break as space and hard break as newline.
        let Some(Block::Paragraph(mix)) = blocks
            .iter()
            .find(|b| matches!(b, Block::Paragraph(p) if p.text.starts_with("Mix of")))
        else {
            panic!("mixed paragraph missing")
        };
        assert!(mix.text.contains(". Soft wrapped line Hard break"), "{}", mix.text);
        assert!(mix.text.contains('\n'), "hard break lost");
        assert!(mix.text.contains("🖼"), "image placeholder missing");
        assert!(mix.spans.iter().any(|(_, s)| s.link));

        assert!(blocks.iter().any(|b| matches!(b, Block::Rule)));

        let Some(Block::Quote(outer)) = blocks.iter().find(|b| matches!(b, Block::Quote(_)))
        else {
            panic!("quote missing")
        };
        assert!(outer.iter().any(|b| matches!(b, Block::Quote(_))), "nested quote missing");

        let lists: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::List { start, items } => Some((start, items)),
                _ => None,
            })
            .collect();
        assert!(lists.iter().any(|(start, items)| {
            start.is_none() && items.iter().all(|i| i.checked.is_none()) && items.len() == 2
        }));
        assert!(lists.iter().any(|(start, _)| **start == Some(5)), "ordered start offset lost");
        assert!(lists.iter().any(|(_, items)| {
            items.iter().map(|i| i.checked).eq([Some(true), Some(false)])
        }));
        assert!(
            lists.iter().any(|(start, items)| start.is_none()
                && items.len() == 1
                && items[0].blocks.len() == 2),
            "loose item should hold two paragraphs"
        );

        let Some(Block::Table { head, rows }) =
            blocks.iter().find(|b| matches!(b, Block::Table { .. }))
        else {
            panic!("table missing")
        };
        assert_eq!(head.len(), 2);
        assert!(head[0].spans.iter().any(|(_, s)| s.bold), "styled header cell lost");
        assert_eq!(rows.len(), 1);
        assert!(rows[0][0].spans.iter().any(|(_, s)| s.code), "code span in cell lost");

        let code_blocks: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code { lang, code, spans } => Some((lang.as_deref(), code, spans)),
                _ => None,
            })
            .collect();
        let Some((_, _, rust_spans)) =
            code_blocks.iter().find(|(lang, ..)| *lang == Some("rust"))
        else {
            panic!("rust fence missing")
        };
        assert!(!rust_spans.is_empty(), "rust code should highlight");
        let Some((_, plain_code, plain_spans)) =
            code_blocks.iter().find(|(lang, ..)| lang.is_none())
        else {
            panic!("bare fence missing")
        };
        assert_eq!(plain_code.as_str(), "no language");
        assert!(plain_spans.is_empty(), "bare fence must stay unhighlighted");
    }

    #[gpui::test]
    fn every_block_renders_to_an_element(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(Theme::dark())));
        });
        let doc = parsed_sink();
        let t = Theme::dark();
        cx.update(|cx| {
            // First, middle, and last positions all build; the loop walks every
            // branch of `block` (headings, code, quote, lists, table, rule).
            for ix in 0..doc.blocks.len() {
                let _ = list_item(&doc, ix, &t, cx);
            }
            // Out-of-range index degrades to an empty element instead of panicking.
            let _ = list_item(&doc, doc.blocks.len(), &t, cx);
        });
    }

    #[gpui::test]
    fn mermaid_fence_renders_pending_then_ready(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(Theme::dark())));
        });
        let doc = markdown::parse("```mermaid\nflowchart LR\n  a[Start] --> b[End]\n```\n");
        let Block::Code { lang, code, .. } = &doc.blocks[0] else { panic!("expected code block") };
        assert_eq!(lang.as_deref(), Some("mermaid"));
        let t = Theme::dark();
        let code = code.clone();

        // First render: cache miss → pending placeholder, render job spawned.
        cx.update(|cx| {
            let _ = list_item(&doc, 0, &t, cx);
            assert!(matches!(
                crate::diagram::diagram_state(&code, 664.0, cx),
                crate::diagram::DiagramState::Pending
            ));
        });
        cx.run_until_parked();
        // Job finished: the same fence now renders the ready image branch.
        cx.update(|cx| {
            assert!(matches!(
                crate::diagram::diagram_state(&code, 664.0, cx),
                crate::diagram::DiagramState::Ready(_)
            ));
            let _ = list_item(&doc, 0, &t, cx);
        });
    }

    #[gpui::test]
    fn broken_mermaid_fence_renders_the_error_strip(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(Theme::dark())));
        });
        let doc = markdown::parse("```mermaid\nnot_a_diagram_type_xyz\n```\n");
        let Block::Code { code, .. } = &doc.blocks[0] else { panic!("expected code block") };
        let t = Theme::dark();
        let code = code.clone();

        cx.update(|cx| {
            let _ = list_item(&doc, 0, &t, cx); // spawns the render, shows pending
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let state = crate::diagram::diagram_state(&code, 664.0, cx);
            let crate::diagram::DiagramState::Failed(msg) = state else { panic!("expected failure") };
            assert!(!msg.is_empty());
            let _ = list_item(&doc, 0, &t, cx); // error strip + plain code branch
        });
    }
}

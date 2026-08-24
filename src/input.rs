//! Single-line text input with full IME support, adapted from gpui's input
//! example and themed for supermd. Used by the fuzzy finder; grows into
//! search/rename fields later.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    actions, div, fill, point, px, size, App, Bounds, ClipboardItem, Context, CursorStyle,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable,
    GlobalElementId, Hsla, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ShapedLine, SharedString, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::theme;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        ShowCharacterPalette,
    ]
);

pub struct TextInput {
    pub focus_handle: FocusHandle,
    pub content: SharedString,
    pub placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: "".into(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left())
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + last_layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + last_layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        let utf8_index = last_layout.index_for_x(point.x - line_point.x)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct TextElement {
    input: Entity<TextInput>,
    cursor_color: Hsla,
    selection_color: Hsla,
    placeholder_color: Hsla,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style = window.text_style();

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), self.placeholder_color)
        } else {
            (content, style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = self.input.read(cx).marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let cursor_pos = line.x_for_index(cursor);
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_pos, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    self.cursor_color,
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    self.selection_color,
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }
        let line = prepaint.line.take().unwrap();
        line.paint(bounds.origin, window.line_height(), window, cx)
            .unwrap();

        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .flex()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .text_size(px(15.))
            .text_color(t.fg_strong)
            .line_height(px(24.))
            .child(TextElement {
                input: cx.entity(),
                cursor_color: t.accent,
                selection_color: Hsla {
                    a: 0.25,
                    ..t.accent
                },
                placeholder_color: t.fg_muted,
            })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyBinding, Modifiers, TestAppContext, VisualTestContext};
    use std::sync::Arc;

    const PLACEHOLDER: &str = "Search notes";

    fn open_input(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::ActiveTheme(Arc::new(
                crate::theme::Theme::dark(),
            )))
        });
        let (input, cx) = cx.add_window_view(|_, cx| TextInput::new(PLACEHOLDER, cx));
        cx.update(|window, app| {
            let handle = input.read(app).focus_handle.clone();
            window.focus(&handle);
        });
        cx.run_until_parked();
        (input, cx)
    }

    fn content(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> String {
        cx.update(|_, app| input.read(app).content.to_string())
    }

    fn selection(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> Range<usize> {
        cx.update(|_, app| input.read(app).selected_range.clone())
    }

    /// Replace the whole content through the real input-handler entry point
    /// (which takes UTF-16 ranges), so multibyte text lands the same way the
    /// platform IME would deliver it.
    fn set_text(input: &Entity<TextInput>, cx: &mut VisualTestContext, text: &str) {
        input.update_in(cx, |input, window, cx| {
            let end_utf16 = input.offset_to_utf16(input.content.len());
            input.replace_text_in_range(Some(0..end_utf16), text, window, cx);
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn placeholder_is_rendered_until_content_arrives(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.update(|_, app| {
            let shaped = input.read(app).last_layout.as_ref();
            let Some(line) = shaped else { panic!("first frame should shape a line") };
            assert_eq!(line.text.as_ref(), PLACEHOLDER, "empty input shapes the placeholder");
            assert!(input.read(app).last_bounds.is_some(), "paint records element bounds");
        });
        cx.simulate_input("hi");
        cx.run_until_parked();
        cx.update(|_, app| {
            let Some(line) = input.read(app).last_layout.as_ref() else { panic!("no layout") };
            assert_eq!(line.text.as_ref(), "hi", "content replaces the placeholder");
        });
    }

    #[gpui::test]
    fn typing_inserts_at_cursor_and_replaces_selection(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("hello");
        assert_eq!(content(&input, cx), "hello");
        assert_eq!(selection(&input, cx), 5..5, "cursor follows the insertion");

        cx.dispatch_action(Home);
        cx.simulate_input("X");
        assert_eq!(content(&input, cx), "Xhello", "typing inserts at the cursor");

        cx.dispatch_action(SelectAll);
        cx.simulate_input("y");
        assert_eq!(content(&input, cx), "y", "typing replaces an active selection");
        assert_eq!(selection(&input, cx), 1..1);
    }

    #[gpui::test]
    fn arrow_keys_move_by_grapheme_cluster(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        set_text(&input, cx, "a\u{1F44D}b"); // "a👍b": 1 + 4 + 1 utf-8 bytes
        cx.dispatch_action(End);
        assert_eq!(selection(&input, cx), 6..6);

        cx.dispatch_action(Left);
        assert_eq!(selection(&input, cx), 5..5);
        cx.dispatch_action(Left);
        assert_eq!(selection(&input, cx), 1..1, "left crosses the emoji as one step");
        cx.dispatch_action(Left);
        assert_eq!(selection(&input, cx), 0..0);
        cx.dispatch_action(Left);
        assert_eq!(selection(&input, cx), 0..0, "left at the start is a no-op");

        cx.dispatch_action(Right);
        assert_eq!(selection(&input, cx), 1..1);
        cx.dispatch_action(Right);
        assert_eq!(selection(&input, cx), 5..5, "right crosses the emoji as one step");
        cx.dispatch_action(Right);
        assert_eq!(selection(&input, cx), 6..6);
        cx.dispatch_action(Right);
        assert_eq!(selection(&input, cx), 6..6, "right at the end is a no-op");
    }

    #[gpui::test]
    fn left_and_right_collapse_an_active_selection(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("hello");
        cx.dispatch_action(Home);
        cx.dispatch_action(SelectRight);
        cx.dispatch_action(SelectRight);
        assert_eq!(selection(&input, cx), 0..2);

        cx.dispatch_action(Left);
        assert_eq!(selection(&input, cx), 0..0, "left collapses to the selection start");

        cx.dispatch_action(SelectRight);
        cx.dispatch_action(SelectRight);
        cx.dispatch_action(Right);
        assert_eq!(selection(&input, cx), 2..2, "right collapses to the selection end");
    }

    #[gpui::test]
    fn shift_selection_extends_and_reverses(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("abc");
        cx.dispatch_action(Home);
        cx.dispatch_action(Right);
        assert_eq!(selection(&input, cx), 1..1);

        cx.dispatch_action(SelectLeft);
        cx.update(|_, app| {
            let i = input.read(app);
            assert_eq!(i.selected_range, 0..1);
            assert!(i.selection_reversed, "selecting backwards flags the range reversed");
        });

        cx.dispatch_action(SelectRight);
        cx.update(|_, app| {
            let i = input.read(app);
            assert_eq!(i.selected_range, 1..1, "shift-right shrinks a reversed selection");
            assert!(i.selection_reversed);
        });

        cx.dispatch_action(SelectRight);
        cx.update(|_, app| {
            let i = input.read(app);
            assert_eq!(i.selected_range, 1..2, "extending past the anchor flips direction");
            assert!(!i.selection_reversed);
        });
    }

    #[gpui::test]
    fn home_end_and_select_all(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("hello");
        cx.dispatch_action(Home);
        assert_eq!(selection(&input, cx), 0..0);
        cx.dispatch_action(End);
        assert_eq!(selection(&input, cx), 5..5);
        cx.dispatch_action(SelectAll);
        assert_eq!(selection(&input, cx), 0..5);
    }

    #[gpui::test]
    fn backspace_and_delete_remove_graphemes(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        set_text(&input, cx, "a\u{1F44D}b");
        cx.dispatch_action(End);
        cx.dispatch_action(Backspace);
        assert_eq!(content(&input, cx), "a\u{1F44D}");
        assert_eq!(selection(&input, cx), 5..5);

        cx.dispatch_action(Home);
        cx.dispatch_action(Delete);
        assert_eq!(content(&input, cx), "\u{1F44D}");
        cx.dispatch_action(Delete);
        assert_eq!(content(&input, cx), "", "delete removes a whole emoji cluster");
        cx.dispatch_action(Delete);
        cx.dispatch_action(Backspace);
        assert_eq!(content(&input, cx), "", "backspace/delete on empty input are no-ops");

        cx.simulate_input("hello");
        cx.dispatch_action(Home);
        cx.dispatch_action(SelectRight);
        cx.dispatch_action(SelectRight);
        cx.dispatch_action(Backspace);
        assert_eq!(content(&input, cx), "llo", "backspace removes only the selection");
        assert_eq!(selection(&input, cx), 0..0);
    }

    #[gpui::test]
    fn copy_cut_paste_roundtrip_through_clipboard(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("hello world");
        cx.dispatch_action(SelectAll);
        cx.dispatch_action(Copy);
        assert_eq!(content(&input, cx), "hello world", "copy leaves the content alone");
        let clip = cx.update(|_, app| app.read_from_clipboard().and_then(|i| i.text()));
        assert_eq!(clip.as_deref(), Some("hello world"));

        cx.dispatch_action(Cut);
        assert_eq!(content(&input, cx), "");
        cx.dispatch_action(Paste);
        assert_eq!(content(&input, cx), "hello world");
        assert_eq!(selection(&input, cx), 11..11, "cursor lands after the pasted text");
    }

    #[gpui::test]
    fn empty_selection_copy_and_cut_are_noops(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("abc");
        assert_eq!(selection(&input, cx), 3..3);
        cx.update(|_, app| app.write_to_clipboard(ClipboardItem::new_string("sentinel".into())));
        cx.dispatch_action(Copy);
        let clip = cx.update(|_, app| app.read_from_clipboard().and_then(|i| i.text()));
        assert_eq!(clip.as_deref(), Some("sentinel"), "copy without selection writes nothing");
        cx.dispatch_action(Cut);
        assert_eq!(content(&input, cx), "abc", "cut without selection removes nothing");
    }

    #[gpui::test]
    fn paste_flattens_newlines_to_spaces(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("ab");
        cx.update(|_, app| app.write_to_clipboard(ClipboardItem::new_string("x\ny\nz".into())));
        cx.dispatch_action(Paste);
        assert_eq!(content(&input, cx), "abx y z", "single-line field spaces out newlines");
    }

    #[gpui::test]
    fn ime_composition_marks_updates_and_commits(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);

        // Start composing: text is inserted and marked, not yet committed.
        input.update_in(cx, |input, window, cx| {
            input.replace_and_mark_text_in_range(None, "\u{306B}\u{307B}", None, window, cx); // にほ
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let i = input.read(app);
            assert_eq!(i.content.as_ref(), "\u{306B}\u{307B}");
            assert_eq!(i.marked_range, Some(0..6), "composition is marked in utf-8 offsets");
            assert_eq!(i.selected_range, 6..6);
        });
        let marked_utf16 = input.update_in(cx, |input, window, cx| {
            input.marked_text_range(window, cx)
        });
        assert_eq!(marked_utf16, Some(0..2), "marked range reported to the platform in utf-16");

        // Continue composing: the marked run is replaced wholesale.
        input.update_in(cx, |input, window, cx| {
            input.replace_and_mark_text_in_range(None, "\u{306B}\u{307B}\u{3093}", None, window, cx); // にほん
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let i = input.read(app);
            assert_eq!(i.content.as_ref(), "\u{306B}\u{307B}\u{3093}");
            assert_eq!(i.marked_range, Some(0..9));
            assert_eq!(i.selected_range, 9..9);
        });

        // Commit: replace_text_in_range consumes the marked range.
        input.update_in(cx, |input, window, cx| {
            input.replace_text_in_range(None, "\u{65E5}\u{672C}", window, cx); // 日本
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            let i = input.read(app);
            assert_eq!(i.content.as_ref(), "\u{65E5}\u{672C}");
            assert_eq!(i.marked_range, None, "commit clears the composition");
            assert_eq!(i.selected_range, 6..6);
        });
    }

    #[gpui::test]
    fn unmark_and_empty_replacement_clear_composition(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        input.update_in(cx, |input, window, cx| {
            input.replace_and_mark_text_in_range(None, "\u{3042}", None, window, cx); // あ
        });
        cx.run_until_parked();
        input.update_in(cx, |input, window, cx| {
            assert_eq!(input.marked_text_range(window, cx), Some(0..1));
            input.unmark_text(window, cx);
            assert_eq!(input.marked_text_range(window, cx), None);
        });
        assert_eq!(content(&input, cx), "\u{3042}", "unmark keeps the composed text");

        // Cancelling a composition with an empty replacement removes the text
        // and the mark together.
        set_text(&input, cx, "");
        input.update_in(cx, |input, window, cx| {
            input.replace_and_mark_text_in_range(None, "x", None, window, cx);
            input.replace_and_mark_text_in_range(None, "", None, window, cx);
        });
        cx.update(|_, app| {
            let i = input.read(app);
            assert_eq!(i.content.as_ref(), "");
            assert_eq!(i.marked_range, None);
            assert_eq!(i.selected_range, 0..0);
        });
    }

    #[gpui::test]
    fn text_for_range_and_selected_range_use_utf16_offsets(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        set_text(&input, cx, "a\u{306B}b"); // "aにb": utf-8 len 5, utf-16 len 3
        cx.dispatch_action(End);

        input.update_in(cx, |input, window, cx| {
            let mut actual = None;
            let text = input.text_for_range(1..2, &mut actual, window, cx);
            assert_eq!(text.as_deref(), Some("\u{306B}"));
            assert_eq!(actual, Some(1..2));

            // Out-of-range utf-16 offsets clamp to the end of the content.
            let mut actual = None;
            let text = input.text_for_range(0..99, &mut actual, window, cx);
            assert_eq!(text.as_deref(), Some("a\u{306B}b"));
            assert_eq!(actual, Some(0..3));

            let Some(sel) = input.selected_text_range(false, window, cx) else { panic!("no selection") };
            assert_eq!(sel.range, 3..3, "cursor at utf-8 offset 5 reports utf-16 offset 3");
            assert!(!sel.reversed);
        });
    }

    #[gpui::test]
    fn mouse_click_places_cursor_and_drag_selects(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("hello");
        assert_eq!(selection(&input, cx), 5..5);
        let Some(bounds) = cx.update(|_, app| input.read(app).last_bounds) else { panic!("no bounds") };
        let y = bounds.center().y;

        // Plain click at the left edge parks the cursor at 0.
        cx.simulate_mouse_down(point(bounds.left() + px(1.), y), MouseButton::Left, Modifiers::none());
        assert_eq!(selection(&input, cx), 0..0);

        // Dragging to the right edge sweeps out a selection; the text is far
        // narrower than the full-width element, so this lands past the end.
        cx.simulate_mouse_move(point(bounds.right() - px(1.), y), MouseButton::Left, Modifiers::none());
        assert_eq!(selection(&input, cx), 0..5, "drag extends the selection to the end");
        cx.simulate_mouse_up(point(bounds.right() - px(1.), y), MouseButton::Left, Modifiers::none());

        // With the button released, moving the mouse no longer selects.
        cx.simulate_mouse_move(point(bounds.left() + px(1.), y), None, Modifiers::none());
        assert_eq!(selection(&input, cx), 0..5, "hover after mouse-up leaves the selection");

        // Click to collapse, then shift-click to select from the new cursor.
        cx.simulate_click(point(bounds.left() + px(1.), y), Modifiers::none());
        assert_eq!(selection(&input, cx), 0..0);
        cx.simulate_mouse_down(point(bounds.right() - px(1.), y), MouseButton::Left, Modifiers::shift());
        assert_eq!(selection(&input, cx), 0..5, "shift-click extends instead of moving");
    }

    #[gpui::test]
    fn index_for_mouse_position_edge_cases(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.update(|_, app| {
            assert_eq!(
                input.read(app).index_for_mouse_position(point(px(50.), px(5.))),
                0,
                "empty content always maps to 0"
            );
        });

        cx.simulate_input("hello");
        cx.update(|_, app| {
            let i = input.read(app);
            let Some(bounds) = i.last_bounds else { panic!("no bounds") };
            let above = point(bounds.center().x, bounds.top() - px(1.));
            let below = point(bounds.center().x, bounds.bottom() + px(1.));
            assert_eq!(i.index_for_mouse_position(above), 0, "above the line maps to the start");
            assert_eq!(i.index_for_mouse_position(below), 5, "below the line maps to the end");
        });

        // A never-painted input has no layout to hit-test against.
        cx.update(|_, app| {
            let orphan = app.new(|cx| {
                let mut input = TextInput::new("p", cx);
                input.content = "abc".into();
                input
            });
            assert_eq!(orphan.read(app).index_for_mouse_position(point(px(1.), px(1.))), 0);
        });
    }

    #[gpui::test]
    fn bounds_for_range_and_character_index_for_point(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.simulate_input("hello");
        let Some(bounds) = cx.update(|_, app| input.read(app).last_bounds) else { panic!("no bounds") };

        input.update_in(cx, |input, window, cx| {
            let Some(b) = input.bounds_for_range(0..1, bounds, window, cx) else { panic!("no bounds for range") };
            assert_eq!(b.left(), bounds.left(), "range starting at 0 begins at the text origin");
            assert!(b.right() > b.left(), "a one-character range has positive width");
            assert_eq!(b.top(), bounds.top());
            assert_eq!(b.bottom(), bounds.bottom());

            // The root element starts at the window origin, so any point
            // inside the bounds resolves against the left edge: index 0.
            let inside = bounds.center();
            assert_eq!(input.character_index_for_point(inside, window, cx), Some(0));

            let outside = point(bounds.right() + px(10.), bounds.bottom() + px(10.));
            assert_eq!(input.character_index_for_point(outside, window, cx), None);
        });
    }

    #[gpui::test]
    fn bound_keystrokes_drive_text_input_actions(cx: &mut TestAppContext) {
        let (input, cx) = open_input(cx);
        cx.update(|_, app| {
            app.bind_keys([
                KeyBinding::new("left", Left, Some("TextInput")),
                KeyBinding::new("backspace", Backspace, Some("TextInput")),
            ]);
        });
        cx.simulate_input("abc");
        cx.simulate_keystrokes("left backspace");
        assert_eq!(content(&input, cx), "ac", "keystrokes reach the render-registered actions");
        assert_eq!(selection(&input, cx), 1..1);
    }
}

//! A text field: one line, or several for a message.
//!
//! gpui draws text and delivers keystrokes, but ships no text field: an
//! application brings its own. This one is adapted from the `input` example in
//! gpui's own source (Zed Industries, Apache-2.0), which implements the platform
//! input protocol correctly: IME composition, UTF-16 ranges for the system,
//! grapheme-aware movement, mouse selection and the clipboard.
//!
//! What was added for devd: colours from the theme, events (changed, submitted,
//! cancelled) so a parent can react without polling, word-wise movement and
//! deletion, and a constructor, then, for the message box: several lines (Shift+Enter
//! starts one; Enter sends), and spelling slips underlined, with Ctrl+. to correct one.

use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

use devd_app_core::spell::Dictionary;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, Context, CursorStyle, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString,
    Style, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use unicode_segmentation::*;

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
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        WordLeft,
        WordRight,
        DeleteWordLeft,
        Submit,
        Cancel,
        Newline,
        Up,
        Down,
        FixSpelling,
    ]
);

/// What a text field tells its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// The text is different now.
    Changed,
    /// Enter was pressed.
    Submitted,
    /// Escape was pressed.
    Cancelled,
}

impl EventEmitter<InputEvent> for TextInput {}

/// The keys a text field answers to while it has the focus.
pub fn bind_keys(cx: &mut App) {
    let context = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("ctrl-left", WordLeft, context),
        KeyBinding::new("ctrl-right", WordRight, context),
        KeyBinding::new("alt-left", WordLeft, context),
        KeyBinding::new("alt-right", WordRight, context),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, context),
        KeyBinding::new("alt-backspace", DeleteWordLeft, context),
        KeyBinding::new("enter", Submit, context),
        KeyBinding::new("shift-enter", Newline, context),
        KeyBinding::new("alt-enter", Newline, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("secondary-.", FixSpelling, context),
        KeyBinding::new("escape", Cancel, context),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, context),
    ]);
}

/// A text field.
pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    /// The lines as last drawn, each with where it starts in the text.
    lines: Vec<(usize, ShapedLine)>,
    last_bounds: Option<Bounds<Pixels>>,
    /// The most lines shown at once, when Shift+Enter may start another.
    multiline: Option<usize>,
    /// What to check spelling against, when it is checked.
    dictionary: Option<Arc<Dictionary>>,
    /// The text last checked and the slips found in it.
    checked: RefCell<Option<(SharedString, Vec<Range<usize>>)>>,
    is_selecting: bool,
    secret: bool,
}

impl TextInput {
    /// An empty field showing `placeholder` until something is typed.
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: "".into(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            lines: Vec::new(),
            last_bounds: None,
            multiline: None,
            dictionary: None,
            checked: RefCell::new(None),
            is_selecting: false,
            secret: false,
        }
    }

    /// Draw every character as a dot and refuse to copy the text out: a field
    /// for a key or a password.
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    /// Allow several lines, showing up to `most` before it scrolls.
    pub fn multiline(mut self, most: usize) -> Self {
        self.multiline = Some(most.max(2));
        self
    }

    /// Underline words that look misspelt, and let Ctrl+. correct one.
    pub fn spelling(mut self, dictionary: Arc<Dictionary>) -> Self {
        self.dictionary = Some(dictionary);
        self
    }

    /// The slips in the text now, by byte range. Checked once per change.
    fn slips(&self) -> Vec<Range<usize>> {
        let Some(dictionary) = &self.dictionary else { return Vec::new() };
        let mut checked = self.checked.borrow_mut();
        if checked.as_ref().is_none_or(|(text, _)| *text != self.content) {
            *checked = Some((self.content.clone(), dictionary.check(&self.content)));
        }
        checked.as_ref().map(|(_, slips)| slips.clone()).unwrap_or_default()
    }

    /// What has been typed.
    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replace the text and put the cursor at its end.
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    /// Empty the field.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submitted);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancelled);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline.is_some() {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    /// Move to the same place on the line above or below, or to the ends of the
    /// text when there is no such line.
    fn vertical(&mut self, up: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let at = self.lines.iter().rposition(|(start, _)| *start <= cursor);
        let Some(at) = at else { return self.move_to(if up { 0 } else { self.content.len() }, cx) };
        let x = self.lines[at].1.x_for_index(cursor - self.lines[at].0);
        let target = if up { at.checked_sub(1) } else { Some(at + 1).filter(|n| *n < self.lines.len()) };
        match target {
            Some(n) => {
                let (start, line) = &self.lines[n];
                let offset = start + line.closest_index_for_x(x);
                self.move_to(offset, cx);
            }
            None => self.move_to(if up { 0 } else { self.content.len() }, cx),
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, cx);
    }

    /// Correct the misspelt word at the cursor with the nearest known word.
    fn fix_spelling(&mut self, _: &FixSpelling, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(dictionary), cursor) = (self.dictionary.clone(), self.cursor_offset()) else { return };
        let Some(slip) = self.slips().into_iter().find(|r| r.start <= cursor && cursor <= r.end) else { return };
        let Some(better) = dictionary.suggest(&self.content[slip.clone()], 1).into_iter().next() else { return };
        let utf16 = self.range_to_utf16(&slip);
        self.replace_text_in_range(Some(utf16), &better, window, cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor_offset()), cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// The start of the word before `offset`.
    fn previous_word(&self, offset: usize) -> usize {
        self.content[..offset].unicode_word_indices().next_back().map(|(index, _)| index).unwrap_or(0)
    }

    /// The end of the word after `offset`.
    fn next_word(&self, offset: usize) -> usize {
        self.content[offset..]
            .unicode_word_indices()
            .next()
            .map(|(index, word)| offset + index + word.len())
            .unwrap_or(self.content.len())
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

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
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

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = if self.multiline.is_some() { text.replace("\r\n", "\n") } else { text.replace(['\r', '\n'], " ") };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.secret {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.secret {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
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

        let Some(bounds) = self.last_bounds.as_ref() else { return 0 };
        if self.lines.is_empty() || position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        let height = (bounds.bottom() - bounds.top()) / self.lines.len() as f32;
        let at = (((position.y - bounds.top()) / height) as usize).min(self.lines.len() - 1);
        let (start, line) = &self.lines[at];
        start + line.closest_index_for_x(position.x - bounds.left())
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

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
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
        cx.emit(InputEvent::Changed);
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
        let range = self.range_from_utf16(&range_utf16);
        let at = self.lines.iter().rposition(|(start, _)| *start <= range.start)?;
        let (start, line) = &self.lines[at];
        let height = (bounds.bottom() - bounds.top()) / self.lines.len() as f32;
        let top = bounds.top() + height * at as f32;
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start - start), top),
            point(bounds.left() + line.x_for_index(range.end.max(range.start).saturating_sub(*start).min(line.len())), top + height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let line_point = bounds.localize(&point)?;
        if self.lines.is_empty() {
            return None;
        }
        let height = (bounds.bottom() - bounds.top()) / self.lines.len() as f32;
        let at = ((line_point.y / height) as usize).min(self.lines.len() - 1);
        let (start, line) = &self.lines[at];
        let utf8_index = start + line.index_for_x(point.x - bounds.left())?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    lines: Vec<(usize, ShapedLine)>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
}

/// The runs for one line: the base run, underlined wherever `marks` say.
fn line_runs(base: &TextRun, line: Range<usize>, marks: &[(Range<usize>, UnderlineStyle)]) -> Vec<TextRun> {
    let mut edges = vec![line.start, line.end];
    for (range, _) in marks {
        for edge in [range.start, range.end] {
            if edge > line.start && edge < line.end {
                edges.push(edge);
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
        .windows(2)
        .map(|pair| {
            let underline = marks.iter().find(|(range, _)| range.start <= pair[0] && pair[1] <= range.end).map(|(_, style)| *style);
            TextRun { len: pair[1] - pair[0], underline, ..base.clone() }
        })
        .filter(|run| run.len > 0)
        .collect()
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
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
        style.size.width = relative(1.).into();
        // As many lines as the text has, up to the most this field shows.
        let input = self.input.read(cx);
        let shown = match input.multiline {
            Some(most) => input.content.split('\n').count().clamp(1, most),
            None => 1,
        };
        style.size.height = (window.line_height() * shown as f32).into();
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
            (input.placeholder.clone(), theme(cx).faint)
        } else if input.secret {
            // One mark per byte, so every offset into the real text is an
            // offset into what is drawn. Keys are ASCII; nothing is widened.
            ("*".repeat(content.len()).into(), style.color)
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
        // What is underlined: the text being composed, and probable slips.
        let mut marks: Vec<(Range<usize>, UnderlineStyle)> = Vec::new();
        if let Some(marked) = input.marked_range.as_ref() {
            marks.push((marked.clone(), UnderlineStyle { color: Some(run.color), thickness: px(1.0), wavy: false }));
        }
        if !input.secret && !input.content.is_empty() {
            let squiggle = UnderlineStyle { color: Some(theme(cx).danger), thickness: px(1.0), wavy: true };
            marks.extend(input.slips().into_iter().map(|range| (range, squiggle)));
        }

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut lines: Vec<(usize, ShapedLine)> = Vec::new();
        let mut start = 0usize;
        for text in display_text.split('\n') {
            let end = start + text.len();
            let runs = if text.is_empty() { Vec::new() } else { line_runs(&run, start..end, &marks) };
            // `line_runs` gives offsets in the whole text; a line's runs only need their lengths.
            let shaped = window.text_system().shape_line(SharedString::from(text.to_string()), font_size, &runs, None);
            lines.push((start, shaped));
            start = end + 1;
        }

        // The line a byte offset is on: a line owns the offset at its end, where the cursor sits before a newline.
        let line_of = |offset: usize| -> usize { lines.iter().position(|(s, l)| offset >= *s && offset <= *s + l.len()).unwrap_or(lines.len() - 1) };
        let at = line_of(cursor);
        let x_of = |index: usize, lines: &[(usize, ShapedLine)], at: usize| lines[at].1.x_for_index(index.saturating_sub(lines[at].0).min(lines[at].1.len()));
        let (selection, cursor) = if selected_range.is_empty() {
            let top = bounds.top() + line_height * at as f32;
            (
                Vec::new(),
                Some(fill(Bounds::new(point(bounds.left() + x_of(cursor, &lines, at), top), size(px(2.), line_height)), theme(cx).accent)),
            )
        } else {
            let (first, last) = (line_of(selected_range.start), line_of(selected_range.end));
            let quads = (first..=last)
                .map(|n| {
                    let (line_start, line) = &lines[n];
                    let from = if n == first { selected_range.start - line_start } else { 0 };
                    // A selection that crosses a line break shows the break as a little extra width.
                    let (to, extra) = if n == last { (selected_range.end - line_start, px(0.)) } else { (line.len(), px(6.)) };
                    let top = bounds.top() + line_height * n as f32;
                    fill(
                        Bounds::from_corners(point(bounds.left() + line.x_for_index(from), top), point(bounds.left() + line.x_for_index(to) + extra, top + line_height)),
                        theme(cx).selection,
                    )
                })
                .collect();
            (quads, None)
        };
        PrepaintState { lines, cursor, selection }
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
        for quad in prepaint.selection.drain(..) {
            window.paint_quad(quad)
        }
        let line_height = window.line_height();
        let lines = std::mem::take(&mut prepaint.lines);
        for (n, (_, line)) in lines.iter().enumerate() {
            let origin = point(bounds.origin.x, bounds.origin.y + line_height * n as f32);
            line.paint(origin, line_height, window, cx).unwrap();
        }

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.lines = lines;
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .min_w_0()
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
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::fix_spelling))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(div().w_full().overflow_hidden().child(TextElement { input: cx.entity() }))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

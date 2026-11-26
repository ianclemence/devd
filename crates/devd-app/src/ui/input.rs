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
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ScrollHandle, ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
    actions, div, fill, point, prelude::*, px, relative, size,
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
        TextStart,
        TextEnd,
        SelectHome,
        SelectEnd,
        SelectTextStart,
        SelectTextEnd,
        SelectUp,
        SelectDown,
        Undo,
        Redo,
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
        Indent,
        Outdent,
        Save,
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
    /// Ctrl+S (Cmd+S on a Mac) was pressed in a code field.
    Saved,
    /// The up arrow was pressed while the field is steered by a menu.
    Up,
    /// The down arrow was pressed while the field is steered by a menu.
    Down,
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
        // Home and End go to the ends of the line the cursor is on; with Ctrl, to the ends of the whole text. Shift
        // selects on the way, as it does with every movement key.
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("secondary-home", TextStart, context),
        KeyBinding::new("secondary-end", TextEnd, context),
        KeyBinding::new("shift-home", SelectHome, context),
        KeyBinding::new("shift-end", SelectEnd, context),
        KeyBinding::new("secondary-shift-home", SelectTextStart, context),
        KeyBinding::new("secondary-shift-end", SelectTextEnd, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("secondary-z", Undo, context),
        KeyBinding::new("secondary-shift-z", Redo, context),
        KeyBinding::new("secondary-y", Redo, context),
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
    // Only a field made with `.code()` answers to these: elsewhere Tab still moves to the next field.
    let code = Some("CodeInput");
    cx.bind_keys([
        KeyBinding::new("tab", Indent, code),
        KeyBinding::new("shift-tab", Outdent, code),
        KeyBinding::new("secondary-s", Save, code),
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
    /// A menu is open over the field and wants the arrows.
    steered: bool,
    /// Files and addresses in the text are underlined and open on Ctrl+click.
    linking: bool,
    /// An editor for code: Enter keeps the indentation, Tab indents, nothing is clipped, and the view follows the cursor.
    code: bool,
    /// What scrolls the field, so it can bring the cursor into view.
    scroller: Option<ScrollHandle>,
    /// Where the cursor was when the view was last brought to it, so a person who scrolls away is left there.
    viewed_cursor: Option<usize>,
    /// How far the text is scrolled up, in pixels, for a field with more rows than it shows. Pixels, not rows: a
    /// wheel moves the text as far as the wheel turned, and a row can be half in view, as in any editor.
    scroll_y: Pixels,
    /// The height of one row as last drawn, so a click or the system's text input can find a row.
    row_height: Pixels,
    /// What Ctrl+Z goes back to, and what Ctrl+Y brings back.
    history: History,
}

/// One state of the text: what it said and where the selection was.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    content: SharedString,
    selected: Range<usize>,
}

/// What kind of edit a change was, so typing a word is one step back and not one per letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    /// A character typed, not a space or a line break.
    Typing,
    /// A character deleted with Backspace or Delete.
    Deleting,
    /// Anything else: a paste, a cut, a space, a line, a correction. Always its own step.
    Other,
}

/// Undo and redo for one field.
#[derive(Debug, Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The kind of the last edit and when it was made, to decide whether the next one joins its step.
    last: Option<(EditKind, std::time::Instant)>,
}

impl History {
    /// The most steps kept back: enough for any message, bounded for a file left open all day.
    const DEPTH: usize = 400;
    /// Edits of one kind closer together than this are one step.
    const JOIN: std::time::Duration = std::time::Duration::from_millis(900);

    /// Record the state before an edit of `kind`, unless it continues the step the last edit began.
    fn before(&mut self, state: Snapshot, kind: EditKind, now: std::time::Instant) {
        let joins = kind != EditKind::Other
            && self
                .last
                .is_some_and(|(last, at)| last == kind && now.duration_since(at) < Self::JOIN);
        if !joins {
            self.undo.push(state);
            if self.undo.len() > Self::DEPTH {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = Some((kind, now));
    }

    /// Step back from `current`, if there is anywhere to go.
    fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        self.last = None;
        Some(previous)
    }

    /// Step forward from `current`, if something was undone.
    fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        self.last = None;
        Some(next)
    }

    fn forget(&mut self) {
        *self = Self::default();
    }
}

/// What kind of edit putting `new_text` over `replaced` bytes is.
fn kind_of(new_text: &str, replaced: usize) -> EditKind {
    let mut chars = new_text.chars();
    match (chars.next(), chars.next(), replaced) {
        // One character's worth of bytes: Backspace or Delete. Deleting a whole selection is a step of its own.
        (None, None, 1..=4) => EditKind::Deleting,
        (Some(c), None, 0) if !c.is_whitespace() => EditKind::Typing,
        _ => EditKind::Other,
    }
}

impl TextInput {
    /// An empty field showing `placeholder` until something is typed.
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            // A tab stop, so Tab moves from one form field to the next.
            focus_handle: cx.focus_handle().tab_stop(true),
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
            steered: false,
            linking: false,
            code: false,
            scroller: None,
            viewed_cursor: None,
            scroll_y: px(0.),
            row_height: px(0.),
            history: History::default(),
        }
    }

    /// The text and selection now, as a step of history.
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            content: self.content.clone(),
            selected: self.selected_range.clone(),
        }
    }

    /// Put back a state from history.
    fn restore(&mut self, state: Snapshot, cx: &mut Context<Self>) {
        let len = state.content.len();
        self.content = state.content;
        self.selected_range = state.selected.start.min(len)..state.selected.end.min(len);
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.history.undo(self.snapshot()) {
            self.restore(previous, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.history.redo(self.snapshot()) {
            self.restore(next, cx);
        }
    }

    /// Make this an editor for code, scrolled by `scroller`. It shows every line (the parent scrolls it), Enter starts a line
    /// at the indentation of the one above, Tab and Shift+Tab indent and outdent, and Ctrl+S says it was saved.
    pub fn code(mut self, scroller: ScrollHandle) -> Self {
        self.code = true;
        self.multiline = Some(usize::MAX / 4);
        self.scroller = Some(scroller);
        self
    }

    /// Replace the text and put the cursor at its start, as when a file is opened.
    pub fn load(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.viewed_cursor = None;
        self.scroll_y = px(0.);
        // A file just opened has nothing to undo back to.
        self.history.forget();
        cx.notify();
    }

    /// The text's widest line in characters, for a field that must be as wide as its text.
    pub fn widest_line(&self) -> usize {
        self.content
            .split('\n')
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0)
    }

    /// How many rows the text took when it was last drawn, wrapped lines included.
    pub fn rows(&self) -> usize {
        self.lines.len().max(1)
    }

    /// How many lines the text has.
    pub fn line_count(&self) -> usize {
        self.content.split('\n').count()
    }

    /// Underline the files and addresses typed or pasted here; Ctrl+click (Cmd on a Mac) opens one.
    pub fn linking(mut self) -> Self {
        self.linking = true;
        self
    }

    /// Hand the up and down arrows to a parent's menu, or take them back.
    pub fn steer(&mut self, on: bool) {
        self.steered = on;
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
        let Some(dictionary) = &self.dictionary else {
            return Vec::new();
        };
        let mut checked = self.checked.borrow_mut();
        if checked
            .as_ref()
            .is_none_or(|(text, _)| *text != self.content)
        {
            *checked = Some((self.content.clone(), dictionary.check(&self.content)));
        }
        checked
            .as_ref()
            .map(|(_, slips)| slips.clone())
            .unwrap_or_default()
    }

    /// What has been typed.
    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replace the text and put the cursor at its end.
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        // Emptied after a message is sent, the box starts again; any other text set from outside (a recalled prompt,
        // a skill picked from the menu) can be undone back to what was typed.
        if text.is_empty() {
            self.history.forget();
            self.scroll_y = px(0.);
        } else if text != self.content {
            self.history.before(self.snapshot(), EditKind::Other, std::time::Instant::now());
        }
        self.content = text;
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

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if self.code {
            // A new line, as indented as the one it follows.
            let cursor = self.cursor_offset();
            let line_start = self.content[..cursor]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let indent: String = self.content[line_start..cursor]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            return self.replace_text_in_range(None, &format!("\n{indent}"), window, cx);
        }
        cx.emit(InputEvent::Submitted);
    }

    /// Tab in a code field: four spaces at the cursor, or at the start of every line a selection touches.
    fn indent(&mut self, _: &Indent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.code {
            return cx.propagate();
        }
        let range = self.selected_range.clone();
        if range.is_empty() || !self.content[range.clone()].contains('\n') {
            return self.replace_text_in_range(None, "    ", window, cx);
        }
        let start = self.content[..range.start]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let block = &self.content[start..range.end];
        let indented: String = block
            .split('\n')
            .map(|l| {
                if l.is_empty() {
                    l.to_string()
                } else {
                    format!("    {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let added = indented.len() - block.len();
        self.history.before(self.snapshot(), EditKind::Other, std::time::Instant::now());
        let mut next = String::with_capacity(self.content.len() + added);
        next.push_str(&self.content[..start]);
        next.push_str(&indented);
        next.push_str(&self.content[range.end..]);
        self.content = next.into();
        self.selected_range = range.start + 4..range.end + added;
        self.marked_range = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    /// Shift+Tab in a code field: up to four spaces off the start of the line, or of every line a selection touches.
    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.code {
            return cx.propagate();
        }
        let range = self.selected_range.clone();
        let start = self.content[..range.start]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let block = self.content[start..range.end].to_string();
        let outdented: Vec<String> = block
            .split('\n')
            .map(|l| {
                let n = l.chars().take(4).take_while(|c| *c == ' ').count();
                l[n..].to_string()
            })
            .collect();
        let outdented = outdented.join("\n");
        let removed = block.len() - outdented.len();
        if removed == 0 {
            return;
        }
        self.history.before(self.snapshot(), EditKind::Other, std::time::Instant::now());
        let mut next = String::with_capacity(self.content.len());
        next.push_str(&self.content[..start]);
        next.push_str(&outdented);
        next.push_str(&self.content[range.end..]);
        self.content = next.into();
        let first_line_removed = block
            .split('\n')
            .next()
            .map(|l| l.chars().take(4).take_while(|c| *c == ' ').count())
            .unwrap_or(0);
        self.selected_range =
            range.start.saturating_sub(first_line_removed).max(start)..range.end - removed;
        self.marked_range = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if self.code {
            cx.emit(InputEvent::Saved);
        } else {
            cx.propagate();
        }
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
        let target = self.vertical_target(up);
        self.move_to(target, cx);
    }

    /// The offset at the same place on the row above or below the cursor, or the end of the text in that direction
    /// when there is no such row.
    fn vertical_target(&self, up: bool) -> usize {
        let cursor = self.cursor_offset();
        let end = if up { 0 } else { self.content.len() };
        let Some(at) = self.lines.iter().rposition(|(start, _)| *start <= cursor) else {
            return end;
        };
        let x = self.lines[at].1.x_for_index(cursor - self.lines[at].0);
        let target = if up {
            at.checked_sub(1)
        } else {
            Some(at + 1).filter(|n| *n < self.lines.len())
        };
        match target {
            Some(n) => {
                let (start, line) = &self.lines[n];
                start + line.closest_index_for_x(x)
            }
            None => end,
        }
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(true);
        self.select_to(target, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.vertical_target(false);
        self.select_to(target, cx);
    }

    /// Where the line holding `offset` starts: after the last line break before it.
    fn line_start(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())]
            .rfind('\n')
            .map_or(0, |i| i + 1)
    }

    /// Where the line holding `offset` ends: at the next line break, or the end of the text.
    fn line_end(&self, offset: usize) -> usize {
        let offset = offset.min(self.content.len());
        self.content[offset..]
            .find('\n')
            .map_or(self.content.len(), |i| offset + i)
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if self.steered {
            return cx.emit(InputEvent::Up);
        }
        self.vertical(true, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.steered {
            return cx.emit(InputEvent::Down);
        }
        self.vertical(false, cx);
    }

    /// Correct the misspelt word at the cursor with the nearest known word.
    fn fix_spelling(&mut self, _: &FixSpelling, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(dictionary), cursor) = (self.dictionary.clone(), self.cursor_offset()) else {
            return;
        };
        let Some(slip) = self
            .slips()
            .into_iter()
            .find(|r| r.start <= cursor && cursor <= r.end)
        else {
            return;
        };
        let Some(better) = dictionary
            .suggest(&self.content[slip.clone()], 1)
            .into_iter()
            .next()
        else {
            return;
        };
        let utf16 = self.range_to_utf16(&slip);
        self.replace_text_in_range(Some(utf16), &better, window, cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor_offset()), cx);
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// The start of the word before `offset`.
    fn previous_word(&self, offset: usize) -> usize {
        self.content[..offset]
            .unicode_word_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0)
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
        self.move_to(self.line_start(self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_end(self.cursor_offset()), cx);
    }

    fn text_start(&mut self, _: &TextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn text_end(&mut self, _: &TextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_start(self.cursor_offset()), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_end(self.cursor_offset()), cx);
    }

    fn select_text_start(&mut self, _: &SelectTextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_text_end(&mut self, _: &SelectTextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    /// The word around `offset`, for a double-click: the word it touches, or the offset itself between words.
    fn word_at(&self, offset: usize) -> Range<usize> {
        self.content
            .unicode_word_indices()
            .map(|(at, word)| at..at + word.len())
            .find(|word| word.start <= offset && offset <= word.end)
            .unwrap_or(offset..offset)
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
        // Ctrl+click on a link opens it instead of moving the cursor.
        if self.linking && event.modifiers.secondary() {
            let at = self.index_for_mouse_position(event.position);
            let root = super::links::root(cx);
            if let Some((_, link)) = super::links::resolve(&self.content, root.as_deref())
                .into_iter()
                .find(|(r, _)| r.start <= at && at <= r.end)
            {
                super::links::open(&link, cx);
                return;
            }
        }
        let at = self.index_for_mouse_position(event.position);
        // Two clicks take the word, three the line: what every editor does, and what a person reaches for to replace
        // a word in a message.
        match event.click_count {
            2 => {
                let word = self.word_at(at);
                self.selection_reversed = false;
                self.selected_range = word;
                return cx.notify();
            }
            n if n >= 3 => {
                self.selection_reversed = false;
                self.selected_range = self.line_start(at)..self.line_end(at);
                return cx.notify();
            }
            _ => {}
        }
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx)
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
            let text = if self.multiline.is_some() {
                text.replace("\r\n", "\n")
            } else {
                text.replace(['\r', '\n'], " ")
            };
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

        let Some(bounds) = self.last_bounds.as_ref() else {
            return 0;
        };
        if self.lines.is_empty() || position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        let at = self.row_at(position.y - bounds.top());
        let (start, line) = &self.lines[at];
        start + line.closest_index_for_x(position.x - bounds.left())
    }

    /// The row under a point `y` pixels below the top of the field, with the scroll counted in.
    fn row_at(&self, y: Pixels) -> usize {
        let height = if self.row_height > px(0.) {
            self.row_height
        } else {
            px(20.)
        };
        let row = ((y + self.scroll_y) / height).max(0.) as usize;
        row.min(self.lines.len().saturating_sub(1))
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

        // Composition (an IME) is recorded once, when it is committed here, not at every intermediate mark.
        let replaced = range.end.saturating_sub(range.start);
        if !(new_text.is_empty() && replaced == 0) {
            self.history.before(self.snapshot(), kind_of(new_text, replaced), std::time::Instant::now());
        }
        let (content, caret) = insert(&self.content, range, new_text);
        self.content = content.into();
        self.selected_range = caret;
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
        let at = self
            .lines
            .iter()
            .rposition(|(start, _)| *start <= range.start)?;
        let (start, line) = &self.lines[at];
        // Where the system draws its composition window: on the row, as it is scrolled.
        let height = if self.row_height > px(0.) {
            self.row_height
        } else {
            (bounds.bottom() - bounds.top()) / self.lines.len() as f32
        };
        let top = bounds.top() + height * at as f32 - self.scroll_y;
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start - start), top),
            point(
                bounds.left()
                    + line.x_for_index(
                        range
                            .end
                            .max(range.start)
                            .saturating_sub(*start)
                            .min(line.len()),
                    ),
                top + height,
            ),
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
        let at = self.row_at(line_point.y);
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
    /// How far the text is scrolled up, as it is drawn this frame.
    scroll: Pixels,
    /// The scrollbar's thumb, when the text is taller than the field: its top and height within the field.
    thumb: Option<(Pixels, Pixels)>,
}

/// How far to scroll so a row `caret` pixels from the top of the text, `row` tall, is inside a view `view` tall
/// that is scrolled by `scroll`. It moves only as far as it must.
fn follow(scroll: Pixels, caret: Pixels, row: Pixels, view: Pixels) -> Pixels {
    if caret < scroll {
        caret
    } else if caret + row > scroll + view {
        caret + row - view
    } else {
        scroll
    }
}

/// The scrollbar's thumb for text `total` tall seen through a view `view` tall at `scroll`: its top and height. Never
/// shorter than a finger's worth, so it can be seen on a very long message.
fn thumb(total: Pixels, view: Pixels, scroll: Pixels) -> Option<(Pixels, Pixels)> {
    if total <= view + px(0.5) || view <= px(0.) {
        return None;
    }
    let height = (view * (view / total)).max(px(18.)).min(view);
    let travel = view - height;
    let reach = total - view;
    let top = if reach > px(0.) { travel * (scroll / reach) } else { px(0.) };
    Some((top, height))
}

/// Where a line of text breaks so no row is wider than `width`: after the last space that fits, or inside a word
/// that is wider than the row by itself. Returns each row's start and end within `text`.
fn breaks(shaped: &ShapedLine, text: &str, width: Pixels) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut from = 0usize;
    while from < text.len() {
        let left = shaped.x_for_index(from);
        if shaped.x_for_index(text.len()) - left <= width {
            rows.push(from..text.len());
            return rows;
        }
        let mut fit = shaped.closest_index_for_x(left + width).clamp(from, text.len());
        while fit > from && !text.is_char_boundary(fit) {
            fit -= 1;
        }
        let spaced = text[from..fit].rfind(' ').map(|i| from + i + 1).filter(|c| *c > from);
        let mut cut = spaced.unwrap_or(fit);
        if cut <= from {
            // One character wider than the row still takes a row, so the loop always ends.
            cut = from + text[from..].chars().next().map_or(1, char::len_utf8);
        }
        rows.push(from..cut);
        from = cut;
    }
    if rows.is_empty() {
        rows.push(0..0);
    }
    rows
}

/// The runs for one line: the base run, underlined wherever `marks` say.
fn line_runs(
    base: &TextRun,
    line: Range<usize>,
    marks: &[(Range<usize>, UnderlineStyle)],
) -> Vec<TextRun> {
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
            let underline = marks
                .iter()
                .find(|(range, _)| range.start <= pair[0] && pair[1] <= range.end)
                .map(|(_, style)| *style);
            TextRun {
                len: pair[1] - pair[0],
                underline,
                ..base.clone()
            }
        })
        .filter(|run| run.len > 0)
        .collect()
}

/// Put `new_text` over `range` and say where the caret goes.
///
/// It takes the whole text and returns the whole text so a burst of input — a
/// paste, or several characters the system delivers in one frame — is applied
/// to the latest text every time, never to a stale copy.
fn insert(content: &str, range: Range<usize>, new_text: &str) -> (String, Range<usize>) {
    let start = range.start.min(content.len());
    let end = range.end.min(content.len()).max(start);
    let mut next = String::with_capacity(content.len() + new_text.len());
    next.push_str(&content[..start]);
    next.push_str(new_text);
    next.push_str(&content[end..]);
    let caret = start + new_text.len();
    (next, caret..caret)
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
            Some(most) if input.code => input.content.split('\n').count().clamp(1, most),
            Some(most) => {
                // The rows the text takes at the width it was last drawn at; a change of width is caught in paint.
                let width = input.last_bounds.map(|b| b.size.width);
                let base = window.text_style();
                let font_size = base.font_size.to_pixels(window.rem_size());
                let mut rows = 0usize;
                for text in input.content.split('\n') {
                    rows += match width {
                        Some(width) if width > px(8.) && !text.is_empty() => {
                            let run = TextRun {
                                len: text.len(),
                                font: base.font(),
                                color: base.color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let shaped = window.text_system().shape_line(SharedString::from(text.to_string()), font_size, &[run], None);
                            breaks(&shaped, text, width).len()
                        }
                        _ => 1,
                    };
                }
                rows.clamp(1, most)
            }
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
            marks.push((
                marked.clone(),
                UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.0),
                    wavy: false,
                },
            ));
        }
        if input.linking && !input.content.is_empty() {
            let info = theme(cx).info;
            let underline = UnderlineStyle {
                color: Some(info),
                thickness: px(1.0),
                wavy: false,
            };
            marks.extend(
                super::links::resolve(&input.content, super::links::root(cx).as_deref())
                    .into_iter()
                    .map(|(range, _)| (range, underline)),
            );
        }
        if !input.secret && !input.content.is_empty() {
            let squiggle = UnderlineStyle {
                color: Some(theme(cx).danger),
                thickness: px(1.0),
                wavy: true,
            };
            marks.extend(input.slips().into_iter().map(|range| (range, squiggle)));
        }

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut lines: Vec<(usize, ShapedLine)> = Vec::new();
        let mut start = 0usize;
        // A field that is not code breaks a long line into rows at its own width, so text never runs under what is
        // beside it. The placeholder is not wrapped.
        let wrap = (input.multiline.is_some() && !input.code && !input.content.is_empty())
            .then_some(bounds.size.width)
            .filter(|w| *w > px(8.));
        for text in display_text.split('\n') {
            let end = start + text.len();
            let runs = if text.is_empty() {
                Vec::new()
            } else {
                line_runs(&run, start..end, &marks)
            };
            // `line_runs` gives offsets in the whole text; a line's runs only need their lengths.
            let shaped = window.text_system().shape_line(
                SharedString::from(text.to_string()),
                font_size,
                &runs,
                None,
            );
            match wrap {
                Some(width) if !text.is_empty() => {
                    for row in breaks(&shaped, text, width) {
                        let (from, to) = (start + row.start, start + row.end);
                        let row_runs = line_runs(&run, from..to, &marks);
                        let piece = window.text_system().shape_line(
                            SharedString::from(display_text[from..to].to_string()),
                            font_size,
                            &row_runs,
                            None,
                        );
                        lines.push((from, piece));
                    }
                }
                _ => lines.push((start, shaped)),
            }
            start = end + 1;
        }

        // The row a byte offset is on: the last that starts at or before it, so at a wrap the caret is at the start of
        // the next row, and before a newline it is at the end of its own.
        let line_of = |offset: usize| -> usize { lines.iter().rposition(|(s, _)| *s <= offset).unwrap_or(0) };
        let at = line_of(cursor);
        // The text is scrolled in pixels inside the rows the field shows. Only a cursor that has moved since the last
        // paint brings the view to it: a wheel that scrolled away from a cursor standing still is left where it
        // scrolled to, and the next keystroke brings the view back.
        let view = bounds.size.height;
        let total = line_height * lines.len() as f32;
        let reach = (total - view).max(px(0.));
        let mut scroll = input.scroll_y.clamp(px(0.), reach);
        if input.viewed_cursor != Some(cursor) {
            scroll = follow(scroll, line_height * at as f32, line_height, view).clamp(px(0.), reach);
        }
        let shows_bar = input.multiline.is_some() && !input.code;
        let thumb = if shows_bar { thumb(total, view, scroll) } else { None };
        // The rows any part of which is in view: the only ones drawn or selected.
        let first_row = (scroll / line_height).max(0.) as usize;
        let last_row = (((scroll + view) / line_height).ceil() as usize)
            .saturating_sub(1)
            .min(lines.len().saturating_sub(1));
        let row_top = |n: usize| bounds.top() + line_height * n as f32 - scroll;
        let x_of = |index: usize, lines: &[(usize, ShapedLine)], at: usize| {
            lines[at]
                .1
                .x_for_index(index.saturating_sub(lines[at].0).min(lines[at].1.len()))
        };
        let (selection, cursor) = if selected_range.is_empty() {
            let top = row_top(at);
            (
                Vec::new(),
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + x_of(cursor, &lines, at), top),
                        size(px(2.), line_height),
                    ),
                    theme(cx).accent,
                )),
            )
        } else {
            let (first, last) = (line_of(selected_range.start), line_of(selected_range.end));
            let quads = (first.max(first_row)..=last.min(last_row))
                .map(|n| {
                    let (line_start, line) = &lines[n];
                    let from = if n == first {
                        selected_range.start - line_start
                    } else {
                        0
                    };
                    // A selection that crosses a line break shows the break as a little extra width.
                    let (to, extra) = if n == last {
                        (selected_range.end - line_start, px(0.))
                    } else {
                        (line.len(), px(6.))
                    };
                    let top = row_top(n);
                    fill(
                        Bounds::from_corners(
                            point(bounds.left() + line.x_for_index(from), top),
                            point(
                                bounds.left() + line.x_for_index(to) + extra,
                                top + line_height,
                            ),
                        ),
                        theme(cx).selection,
                    )
                })
                .collect();
            (quads, None)
        };
        PrepaintState {
            lines,
            cursor,
            selection,
            scroll,
            thumb,
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
        let line_height = window.line_height();
        let lines = std::mem::take(&mut prepaint.lines);
        let scroll = prepaint.scroll;
        let selection = std::mem::take(&mut prepaint.selection);
        // Everything is drawn inside the field: a row half scrolled out is cut at the field's edge, not drawn over
        // what is above or below it.
        let mask = gpui::ContentMask { bounds };
        window.with_content_mask(Some(mask), |window| {
            for quad in selection {
                window.paint_quad(quad)
            }
            let first = (scroll / line_height).max(0.) as usize;
            let rows = ((bounds.size.height / line_height).ceil() as usize) + 1;
            for (n, (_, line)) in lines.iter().enumerate().skip(first).take(rows) {
                let origin = point(bounds.origin.x, bounds.origin.y + line_height * n as f32 - scroll);
                let _ = line.paint(origin, line_height, window, cx);
            }
        });
        // A thin bar at the right edge says there is more above or below, and how much.
        if let Some((top, height)) = prepaint.thumb {
            let t = theme(cx);
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(bounds.right() - px(3.), bounds.top() + top),
                        size(px(3.), height),
                    ),
                    t.faint.opacity(0.55),
                )
                .corner_radii(px(1.5)),
            );
        }

        // A code field scrolls with its parent; when the cursor has moved, the view moves to it.
        let (scroller, seen, here) = {
            let input = self.input.read(cx);
            (
                input.scroller.clone(),
                input.viewed_cursor,
                input.cursor_offset(),
            )
        };
        if let (Some(scroller), Some(quad)) = (&scroller, &prepaint.cursor)
            && seen != Some(here)
        {
            let (view, caret) = (scroller.bounds(), quad.bounds);
            let mut offset = scroller.offset();
            let margin = px(24.0);
            if caret.top() < view.top() + margin {
                offset.y += view.top() + margin - caret.top();
            } else if caret.bottom() > view.bottom() - margin {
                offset.y -= caret.bottom() - (view.bottom() - margin);
            }
            if caret.left() < view.left() + margin + px(56.0) {
                offset.x += view.left() + margin + px(56.0) - caret.left();
            } else if caret.right() > view.right() - margin {
                offset.x -= caret.right() - (view.right() - margin);
            }
            offset.x = offset.x.min(px(0.0));
            if offset != scroller.offset() {
                scroller.set_offset(offset);
                window.refresh();
            }
        }

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                window.paint_quad(cursor)
            });
        }

        // The height was asked for at the width the text was last drawn at. If the width has changed, the rows it
        // takes have too, and the field is laid out again at once.
        let rows_now = lines.len().clamp(1, self.input.read(cx).multiline.unwrap_or(1));
        let moved = self.input.read(cx).last_bounds.is_some_and(|b| b.size.width != bounds.size.width);
        let wrong_height = (bounds.size.height - line_height * rows_now as f32).abs() > px(1.) && self.input.read(cx).multiline.is_some();
        self.input.update(cx, |input, _cx| {
            input.lines = lines;
            input.last_bounds = Some(bounds);
            input.viewed_cursor = Some(here);
            input.scroll_y = scroll;
            input.row_height = line_height;
        });
        if moved || wrong_height {
            window.refresh();
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A code field is as wide as its widest line, so its parent can scroll sideways instead of clipping.
        let code_width = self.code.then(|| {
            let size = window.text_style().font_size.to_pixels(window.rem_size());
            size * 0.62 * (self.widest_line() as f32 + 4.0)
        });
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .key_context(if self.code {
                "TextInput CodeInput"
            } else {
                "TextInput"
            })
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
            .on_action(cx.listener(Self::text_start))
            .on_action(cx.listener(Self::text_end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_text_start))
            .on_action(cx.listener(Self::select_text_end))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::fix_spelling))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            // A wheel scrolls the view of a multi-line field, not just moves the caret through it.
            // The box used to follow only the cursor, so a message longer than the rows it shows
            // could not be read at all: rows past the visible window had no way to be reached. The
            // code editor scrolls its parent instead, so it is left alone here.
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                if this.multiline.is_none() || this.code || this.scroller.is_some() {
                    return;
                }
                let most = this.multiline.unwrap_or(1).max(1);
                let visible = this.lines.len().clamp(1, most);
                if this.lines.len() <= visible {
                    return;
                }
                let line_height = window.line_height();
                // As far as the wheel turned: the system's own lines per notch (gpui has already multiplied by the
                // Windows "lines to scroll" setting, three by default, as Notepad and VS Code use), a touchpad's
                // movement in its own pixels. The text follows the hand instead of jumping a whole row at a time.
                let dy = ev.delta.pixel_delta(line_height).y;
                let reach = line_height * (this.lines.len() - visible) as f32;
                let next = (this.scroll_y - dy).clamp(px(0.), reach);
                // Within the box, the box scrolls; at its ends the turn is not taken from the page around it.
                if next != this.scroll_y {
                    this.scroll_y = next;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(match code_width {
                Some(width) => div()
                    .min_w(width)
                    .w_full()
                    .child(TextElement { input: cx.entity() }),
                None => div()
                    .w_full()
                    .overflow_hidden()
                    .child(TextElement { input: cx.entity() }),
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

    #[test]
    fn a_burst_of_characters_never_loses_one() {
        // What a fast keyboard or an automation tool delivers between frames:
        // each character replaces the caret from the previous one.
        let mut text = String::new();
        let mut caret = 0..0;
        for ch in "gotest".chars() {
            let (next, after) = insert(&text, caret, &ch.to_string());
            text = next;
            caret = after;
        }
        assert_eq!(text, "gotest");
        assert_eq!(caret, 6..6);
    }

    #[test]
    fn the_view_moves_only_as_far_as_the_cursor_needs() {
        let (row, view) = (px(20.), px(160.));
        // Already in view: nothing moves.
        assert_eq!(follow(px(200.), px(240.), row, view), px(200.));
        // Above the view: its row becomes the top.
        assert_eq!(follow(px(200.), px(60.), row, view), px(60.));
        // Below the view: its row becomes the bottom.
        assert_eq!(follow(px(200.), px(600.), row, view), px(460.));
    }

    #[test]
    fn the_scrollbar_says_where_the_view_is_and_how_much_there_is() {
        // Everything fits: no bar.
        assert_eq!(thumb(px(100.), px(160.), px(0.)), None);
        // Four times the view: a quarter-height thumb, at the top, then at the bottom.
        assert_eq!(thumb(px(640.), px(160.), px(0.)), Some((px(0.), px(40.))));
        assert_eq!(thumb(px(640.), px(160.), px(480.)), Some((px(120.), px(40.))));
        // A very long message still has a thumb a finger can find.
        assert_eq!(thumb(px(100_000.), px(160.), px(0.)).map(|t| t.1), Some(px(18.)));
    }

    #[test]
    fn typing_a_word_is_one_step_back_and_a_paste_is_its_own() {
        let at = std::time::Instant::now();
        let soon = at + std::time::Duration::from_millis(100);
        let state = |text: &str| Snapshot { content: text.to_string().into(), selected: text.len()..text.len() };
        let mut history = History::default();
        history.before(state(""), kind_of("h", 0), at);
        history.before(state("h"), kind_of("i", 0), soon);
        assert_eq!(history.undo.len(), 1, "two letters typed together are one step");
        history.before(state("hi"), kind_of(" ", 0), soon);
        history.before(state("hi "), kind_of("pasted text", 0), soon);
        assert_eq!(history.undo.len(), 3, "a space and a paste each start a step");
        let back = history.undo(state("hi pasted text")).unwrap();
        assert_eq!(back.content.as_ref(), "hi ");
        let forward = history.redo(back).unwrap();
        assert_eq!(forward.content.as_ref(), "hi pasted text");
        // A new edit after an undo cannot be redone over.
        history.undo(forward);
        history.before(state("hi "), kind_of("x", 0), soon);
        assert!(history.redo.is_empty());
    }

    #[test]
    fn a_letter_waits_its_turn_after_a_pause() {
        let at = std::time::Instant::now();
        let later = at + std::time::Duration::from_secs(2);
        let state = Snapshot { content: "".into(), selected: 0..0 };
        let mut history = History::default();
        history.before(state.clone(), EditKind::Typing, at);
        history.before(state, EditKind::Typing, later);
        assert_eq!(history.undo.len(), 2, "typing after a pause is a new step");
        assert_eq!(kind_of("", 1), EditKind::Deleting);
        assert_eq!(kind_of("", 400), EditKind::Other, "deleting a selection is its own step");
    }

    #[test]
    fn a_paste_replaces_the_selection_and_keeps_the_rest() {
        let (text, caret) = insert("hello world", 6..11, "devd");
        assert_eq!(text, "hello devd");
        assert_eq!(caret, 10..10);
        let (text, caret) = insert("shop", 2..2, "🛒");
        assert_eq!(text, "sh🛒op");
        assert_eq!(
            caret,
            6..6,
            "the caret is after the four bytes of the emoji"
        );
    }

    #[test]
    #[allow(clippy::reversed_empty_ranges)] // the defensive case itself
    fn a_range_from_an_older_frame_cannot_cut_a_character() {
        let (text, _) = insert("shop", 99..120, "!");
        assert_eq!(text, "shop!");
        let (text, _) = insert("shop", 2..1, "-");
        assert_eq!(text, "sh-op");
    }
}

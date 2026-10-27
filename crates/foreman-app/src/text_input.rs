//! The one-line text input (m5/30): the tree's search box and the Add workspace name. Built on gpui's
//! `EntityInputHandler` as `.repos/zed/crates/gpui/examples/input.rs` is, so characters, dead keys and IME
//! compositions come from the OS text input (WM_CHAR / WM_IME_COMPOSITION), not from `key_char`. Keys that
//! edit or move are actions bound in the `TextInput` context ([`bind_keys`]); every other key must be left
//! to propagate, or Windows never translates it into a character ([`crate::view::FleetView`]'s `key_down`).
//! The editing itself is [`LineText`], which knows nothing of gpui.

use std::{
    cell::Cell,
    ops::Range,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ShapedLine, SharedString, Style, Task, TextRun, UTF16Selection, UnderlineStyle, Window, actions,
    div, fill, point, prelude::*, px, relative, size,
};
use theme::ActiveTheme as _;
use unicode_segmentation::UnicodeSegmentation as _;

/// Windows Terminal's blink interval, the terminal tab's too.
pub const BLINK: Duration = Duration::from_millis(530);
const CONTEXT: &str = "TextInput";

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Home,
        End,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Confirm,
        Cancel,
    ]
);

/// Windows' edit-control keys. Bound in the `TextInput` context only, so they never reach a terminal tab.
pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, c),
        KeyBinding::new("ctrl-delete", DeleteWordRight, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("ctrl-left", WordLeft, c),
        KeyBinding::new("ctrl-right", WordRight, c),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, c),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("ctrl-a", SelectAll, c),
        KeyBinding::new("ctrl-c", Copy, c),
        KeyBinding::new("ctrl-insert", Copy, c),
        KeyBinding::new("ctrl-x", Cut, c),
        KeyBinding::new("shift-delete", Cut, c),
        KeyBinding::new("ctrl-v", Paste, c),
        KeyBinding::new("shift-insert", Paste, c),
        KeyBinding::new("ctrl-z", Undo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("ctrl-shift-z", Redo, c),
        KeyBinding::new("enter", Confirm, c),
        KeyBinding::new("escape", Cancel, c),
    ]);
}

/// What one edit was, for undo: typed characters run together into one step until the caret moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Other,
}

#[derive(Debug, Clone)]
struct Snapshot {
    text: String,
    selected: Range<usize>,
    reversed: bool,
}

/// One line of text, its selection (byte offsets on grapheme boundaries; the caret is the end that moved)
/// and the IME's marked range, with undo and redo.
#[derive(Debug, Clone, Default)]
pub struct LineText {
    text: String,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The last edit was typing that ended here: the next character typed at it joins its undo step.
    typing_at: Option<usize>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl LineText {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), selected: text.len()..text.len(), ..Default::default() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Range<usize> {
        self.selected.clone()
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    /// The whole text replaced, the caret at its end, the history gone.
    pub fn set(&mut self, text: &str) {
        *self = Self::new(text);
    }

    pub fn move_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        self.selected = offset..offset;
        self.reversed = false;
        self.typing_at = None;
    }

    /// The caret to `offset`, the other end of the selection staying where it is.
    pub fn select_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        self.typing_at = None;
    }

    /// `range` selected, the caret at its end (or its start when `reversed`).
    pub fn select(&mut self, range: Range<usize>, reversed: bool) {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        self.selected = start.min(end)..start.max(end);
        self.reversed = reversed;
        self.typing_at = None;
    }

    pub fn select_all(&mut self) {
        self.select(0..self.text.len(), false);
    }

    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub fn prev_grapheme(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }

    pub fn next_grapheme(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.text.len())
    }

    /// Ctrl+Left: the start of the word before `offset`, over any separators first.
    pub fn word_left(&self, offset: usize) -> usize {
        let before: Vec<(usize, char)> = self.text[..offset].char_indices().collect();
        let mut i = before.len();
        while i > 0 && !is_word(before[i - 1].1) {
            i -= 1;
        }
        while i > 0 && is_word(before[i - 1].1) {
            i -= 1;
        }
        before.get(i).map_or(offset, |(at, _)| *at)
    }

    /// Ctrl+Right: the end of the word after `offset`, over any separators first.
    pub fn word_right(&self, offset: usize) -> usize {
        let mut chars = self.text[offset..].char_indices().peekable();
        while chars.next_if(|(_, c)| !is_word(*c)).is_some() {}
        while chars.next_if(|(_, c)| is_word(*c)).is_some() {}
        chars.peek().map_or(self.text.len(), |(at, _)| offset + at)
    }

    /// What a double-click at `offset` selects: the word there, or the run of separators.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let offset = self.clamp(offset);
        let at = self.text[offset..].chars().next().or_else(|| self.text[..offset].chars().next_back());
        let Some(at) = at else {
            return offset..offset;
        };
        let kind = |c: char| if c.is_whitespace() { 0 } else if is_word(c) { 1 } else { 2 };
        let same = |c: char| kind(c) == kind(at);
        let start = self.text[..offset].char_indices().rev().take_while(|(_, c)| same(*c)).last().map_or(offset, |(i, _)| i);
        let end = self.text[offset..].char_indices().find(|(_, c)| !same(*c)).map_or(self.text.len(), |(i, _)| offset + i);
        start..end
    }

    /// `range` replaced by `new`, the caret after it; one undo step unless typing continues the last one.
    pub fn replace(&mut self, range: Range<usize>, new: &str, kind: EditKind) {
        let range = self.clamp(range.start)..self.clamp(range.end);
        // a composition being committed already pushed its step when it began
        let joins = self.marked.is_some()
            || (kind == EditKind::Typing && range.is_empty() && self.typing_at == Some(range.start) && !new.contains(char::is_whitespace));
        if !joins {
            self.undo.push(self.snapshot());
        }
        self.redo.clear();
        self.text.replace_range(range.clone(), new);
        let caret = range.start + new.len();
        self.selected = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.typing_at = (kind == EditKind::Typing).then_some(caret);
    }

    /// The IME's composition: `new` replaces `range` and stays marked, `selected` (relative to `new`) selected
    /// in it. Undo takes back the whole composition in one step.
    pub fn replace_marked(&mut self, range: Range<usize>, new: &str, selected: Option<Range<usize>>) {
        let range = self.clamp(range.start)..self.clamp(range.end);
        if self.marked.is_none() {
            self.undo.push(self.snapshot());
            self.redo.clear();
        }
        self.text.replace_range(range.clone(), new);
        self.marked = (!new.is_empty()).then(|| range.start..range.start + new.len());
        let inside = |o: usize| range.start + o.min(new.len());
        self.selected = selected.map_or(range.start + new.len()..range.start + new.len(), |s| inside(s.start)..inside(s.end));
        self.selected = self.clamp(self.selected.start)..self.clamp(self.selected.end);
        self.reversed = false;
        self.typing_at = None;
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), selected: self.selected.clone(), reversed: self.reversed }
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.selected = s.selected;
        self.reversed = s.reversed;
        self.marked = None;
        self.typing_at = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(s);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(s);
        true
    }

    pub fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        for (i, c) in self.text.char_indices() {
            if utf16 >= offset {
                return i;
            }
            utf16 += c.len_utf16();
        }
        self.text.len()
    }

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].encode_utf16().count()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

/// What `accept` takes of `text`: each character mapped, the refused ones dropped, the first refused returned.
pub fn filtered(text: &str, accept: fn(char) -> Option<char>) -> (String, Option<char>) {
    let mut refused = None;
    let taken = text
        .chars()
        .filter_map(|c| {
            let taken = accept(c);
            if taken.is_none() && refused.is_none() {
                refused = Some(c);
            }
            taken
        })
        .collect();
    (taken, refused)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// The text changed.
    Edited,
    /// A character the input's filter does not take; nothing of it was inserted.
    Refused(char),
    /// Enter.
    Confirm,
    /// Esc.
    Cancel,
}

pub struct TextInput {
    focus_handle: FocusHandle,
    pub line: LineText,
    placeholder: SharedString,
    /// Maps each typed or pasted character to what is stored, or refuses it.
    filter: Option<fn(char) -> Option<char>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    /// How far the text is scrolled left so the caret stays in view.
    scroll_x: Pixels,
    is_selecting: bool,
    /// The last edit, move or focus: the caret is lit for one [`BLINK`] after it.
    moved_at: Instant,
    /// Whether the last frame painted the caret lit; the selftest reads it.
    pub caret_painted: Cell<bool>,
    _blink: Task<()>,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, filter: Option<fn(char) -> Option<char>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        cx.on_focus(&focus_handle, window, |this: &mut Self, _, cx| this.lit(cx)).detach();
        cx.on_blur(&focus_handle, window, |_: &mut Self, _, cx| cx.notify()).detach();
        // repaints at each blink edge while focused
        let blink = cx.spawn_in(window, async move |this, cx| {
            loop {
                let wait = this.update_in(cx, |this, window, cx| {
                    if !this.focus_handle.is_focused(window) {
                        return BLINK;
                    }
                    cx.notify();
                    this.next_blink(Instant::now())
                });
                let Ok(wait) = wait else {
                    return;
                };
                cx.background_executor().timer(wait + Duration::from_millis(5)).await;
            }
        });
        Self {
            focus_handle,
            line: LineText::default(),
            placeholder: placeholder.into(),
            filter,
            last_layout: None,
            last_bounds: None,
            scroll_x: px(0.),
            is_selecting: false,
            moved_at: Instant::now(),
            caret_painted: Cell::new(false),
            _blink: blink,
        }
    }

    pub fn text(&self) -> &str {
        self.line.text()
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// The whole text replaced from outside (not an edit: no event).
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.line.set(text);
        self.lit(cx);
    }

    /// Focused, everything selected (Ctrl+Shift+E on the search box).
    pub fn focus_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_all();
        self.focus_handle.focus(window, cx);
        self.lit(cx);
    }

    /// Whether the caret is painted at `now`: lit for one interval after an edit or move, then blinking.
    pub fn caret_lit(&self, now: Instant) -> bool {
        (now.saturating_duration_since(self.moved_at).as_millis() / BLINK.as_millis()) % 2 == 0
    }

    fn next_blink(&self, now: Instant) -> Duration {
        let ms = BLINK.as_millis() as u64;
        let elapsed = now.saturating_duration_since(self.moved_at).as_millis() as u64;
        Duration::from_millis(ms - elapsed % ms)
    }

    fn lit(&mut self, cx: &mut Context<Self>) {
        self.moved_at = Instant::now();
        cx.notify();
    }

    fn moved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.notify_text_selection_changed(&self.focus_handle);
        self.lit(cx);
    }

    fn edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.notify_text_input_changed(&self.focus_handle);
        self.lit(cx);
        cx.emit(InputEvent::Edited);
    }

    fn move_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.line.move_to(offset);
        self.moved(window, cx);
    }

    fn select_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_to(offset);
        self.moved(window, cx);
    }

    /// `text` in place of `range` (the selection when `None`), through the filter.
    fn insert(&mut self, range: Option<Range<usize>>, text: &str, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        let marked = self.line.marked();
        let range = range.or(marked.clone()).unwrap_or(self.line.selection());
        let (taken, refused) = match self.filter {
            Some(accept) => filtered(text, accept),
            None => (text.to_string(), None),
        };
        if let Some(c) = refused {
            cx.emit(InputEvent::Refused(c));
        }
        // a refused key leaves the selection alone, as the old field did
        if taken.is_empty() && marked.is_none() && (range.is_empty() || refused.is_some()) {
            self.lit(cx);
            return;
        }
        self.line.replace(range, &taken, kind);
        self.edited(window, cx);
    }

    fn delete_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.line.selection().is_empty() {
            let caret = self.line.cursor();
            if offset == caret {
                return;
            }
            self.line.select(caret.min(offset)..caret.max(offset), false);
        }
        self.line.replace(self.line.selection(), "", EditKind::Other);
        self.edited(window, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.prev_grapheme(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.next_grapheme(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.word_left(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.word_right(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    fn left(&mut self, _: &Left, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        let to = if s.is_empty() { self.line.prev_grapheme(s.start) } else { s.start };
        self.move_to(to, window, cx);
    }

    fn right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        let to = if s.is_empty() { self.line.next_grapheme(s.end) } else { s.end };
        self.move_to(to, window, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.prev_grapheme(self.line.cursor()), window, cx);
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.next_grapheme(self.line.cursor()), window, cx);
    }

    fn word_left(&mut self, _: &WordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line.word_left(self.line.cursor()), window, cx);
    }

    fn word_right(&mut self, _: &WordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line.word_right(self.line.cursor()), window, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.word_left(self.line.cursor()), window, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.word_right(self.line.cursor()), window, cx);
    }

    fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
    }

    fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line.text().len(), window, cx);
    }

    fn select_home(&mut self, _: &SelectHome, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, window, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.text().len(), window, cx);
    }

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_all();
        self.moved(window, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        if !s.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.line.text()[s].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        if !s.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.line.text()[s.clone()].to_string()));
            self.line.replace(s, "", EditKind::Other);
            self.edited(window, cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let line = text.trim_end_matches(['\r', '\n']).replace("\r\n", " ").replace(['\r', '\n'], " ");
            self.insert(None, &line, EditKind::Other, window, cx);
        }
    }

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.line.undo() {
            self.edited(window, cx);
        }
    }

    fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.line.redo() {
            self.edited(window, cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Confirm);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }

    fn index_for_mouse(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref()) else {
            return 0;
        };
        if self.line.text().is_empty() || line.text.as_ref() != self.line.text() {
            return 0;
        }
        line.closest_index_for_x(position.x - bounds.left() + self.scroll_x)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // the window's own focus does not take this click back
        cx.stop_propagation();
        self.focus_handle.focus(window, cx);
        let at = self.index_for_mouse(event.position);
        match event.click_count {
            2 => {
                let word = self.line.word_at(at);
                self.line.select(word, false);
                self.moved(window, cx);
            }
            n if n >= 3 => {
                self.line.select_all();
                self.moved(window, cx);
            }
            _ => {
                self.is_selecting = true;
                if event.modifiers.shift { self.select_to(at, window, cx) } else { self.move_to(at, window, cx) }
            }
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting && event.dragging() {
            self.select_to(self.index_for_mouse(event.position), window, cx);
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(&mut self, range_utf16: Range<usize>, actual_range: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = self.line.range_from_utf16(&range_utf16);
        actual_range.replace(self.line.range_to_utf16(&range));
        Some(self.line.text()[range].to_string())
    }

    fn selected_text_range(&mut self, _ignore_disabled_input: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.line.range_to_utf16(&self.line.selection()), reversed: self.line.cursor() != self.line.selection().end })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.line.marked().map(|r| self.line.range_to_utf16(&r))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.line.unmark();
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.map(|r| self.line.range_from_utf16(&r));
        self.insert(range, text, EditKind::Typing, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.line.range_from_utf16(&r))
            .or(self.line.marked())
            .unwrap_or(self.line.selection());
        // the selection inside the composition, in UTF-16 units of `new_text`, to bytes of it
        let selected = new_selected_range_utf16.map(|s| {
            let byte = |u: usize| {
                let mut units = 0;
                for (i, c) in new_text.char_indices() {
                    if units >= u {
                        return i;
                    }
                    units += c.len_utf16();
                }
                new_text.len()
            };
            byte(s.start)..byte(s.end)
        });
        self.line.replace_marked(range, new_text, selected);
        window.notify_text_input_changed(&self.focus_handle);
        self.lit(cx);
        cx.emit(InputEvent::Edited);
    }

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.line.range_from_utf16(&range_utf16);
        let x = |i: usize| bounds.left() + layout.x_for_index(i.min(layout.text.len())) - self.scroll_x;
        Some(Bounds::from_corners(point(x(range.start), bounds.top()), point(x(range.end), bounds.bottom())))
    }

    fn character_index_for_point(&mut self, point: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let local = self.last_bounds?.localize(&point)?;
        let layout = self.last_layout.as_ref()?;
        if layout.text.as_ref() != self.line.text() {
            return None;
        }
        Some(self.line.offset_to_utf16(layout.closest_index_for_x(local.x + self.scroll_x)))
    }
}

/// The input's line: text (or the placeholder), selection, the IME's underline and the caret.
struct TextElement {
    input: Entity<TextInput>,
}

struct Prepainted {
    line: ShapedLine,
    scroll_x: Pixels,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let colors = cx.theme().colors().clone();
        let input = self.input.read(cx);
        let text = input.line.text();
        let style = window.text_style();
        let (shown, color) = if text.is_empty() { (input.placeholder.clone(), colors.text_placeholder) } else { (SharedString::from(text.to_string()), style.color) };
        let run = TextRun { len: shown.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs = match input.line.marked().filter(|_| !text.is_empty()) {
            Some(m) => vec![
                TextRun { len: m.start, ..run.clone() },
                TextRun { len: m.end - m.start, underline: Some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }), ..run.clone() },
                TextRun { len: shown.len() - m.end, ..run },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(shown, font_size, &runs, None);

        let (selected, cursor) = (input.line.selection(), input.line.cursor());
        let x = |i: usize| if text.is_empty() { px(0.) } else { line.x_for_index(i) };
        let caret_x = x(cursor);
        // keep the caret inside the box, with a little room on the right
        let width = bounds.size.width - px(2.);
        let mut scroll_x = input.scroll_x.min((line.width - width).max(px(0.)));
        if caret_x - scroll_x > width {
            scroll_x = caret_x - width;
        } else if caret_x < scroll_x {
            scroll_x = caret_x;
        }
        let focused = input.focus_handle.is_focused(window);
        let lit = focused && input.caret_lit(Instant::now());
        input.caret_painted.set(lit);
        let left = bounds.left() - scroll_x;
        let selection = (!selected.is_empty()).then(|| {
            fill(
                Bounds::from_corners(point(left + x(selected.start), bounds.top()), point(left + x(selected.end), bounds.bottom())),
                colors.element_selection_background,
            )
        });
        let caret = lit.then(|| fill(Bounds::new(point(left + caret_x, bounds.top()), size(px(1.5), bounds.size.height)), colors.text));
        self.input.update(cx, |input, _| input.scroll_x = scroll_x);
        Prepainted { line, scroll_x, selection, caret }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            let origin = point(bounds.left() - prepaint.scroll_x, bounds.top());
            prepaint.line.paint(origin, window.line_height(), gpui::TextAlign::Left, None, window, cx).ok();
            if let Some(caret) = prepaint.caret.take() {
                window.paint_quad(caret);
            }
        });
        let line = prepaint.line.clone();
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let focused = self.focus_handle.is_focused(window);
        div()
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .h(px(22.))
            .px_1()
            .rounded_sm()
            .border_1()
            .border_color(if focused { colors.border_focused } else { colors.border })
            .bg(colors.editor_background)
            .text_sm()
            .line_height(px(18.))
            .overflow_hidden()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(TextElement { input: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, cursor: usize) -> LineText {
        let mut l = LineText::new(text);
        l.move_to(cursor);
        l
    }

    fn typed(l: &mut LineText, text: &str) {
        for c in text.chars() {
            l.replace(l.selection(), &c.to_string(), EditKind::Typing);
        }
    }

    #[test]
    fn typing_replaces_the_selection_and_moves_the_caret() {
        let mut l = line("acd", 1);
        typed(&mut l, "b");
        assert_eq!((l.text(), l.cursor()), ("abcd", 2));
        l.select(1..3, false);
        typed(&mut l, "X");
        assert_eq!((l.text(), l.selection()), ("aXd", 2..2));
    }

    #[test]
    fn select_to_extends_from_the_anchor_on_either_side() {
        let mut l = line("abcdef", 3);
        l.select_to(5);
        assert_eq!((l.selection(), l.cursor()), (3..5, 5));
        l.select_to(1);
        assert_eq!((l.selection(), l.cursor()), (1..3, 1));
        l.select_to(4);
        assert_eq!((l.selection(), l.cursor()), (3..4, 4));
        l.select_all();
        assert_eq!(l.selection(), 0..6);
    }

    #[test]
    fn arrows_step_over_whole_graphemes() {
        let l = LineText::new("ae\u{301}z");
        assert_eq!(l.next_grapheme(1), 4, "e + combining acute is one step");
        assert_eq!(l.prev_grapheme(4), 1);
        assert_eq!(l.prev_grapheme(0), 0);
        assert_eq!(l.next_grapheme(5), 5);
    }

    #[test]
    fn ctrl_arrows_jump_words_over_separators() {
        let l = LineText::new("selftest-added  foo_bar.rs");
        assert_eq!(l.word_right(0), 8);
        assert_eq!(l.word_right(8), 14);
        assert_eq!(l.word_right(14), 23);
        assert_eq!(l.word_right(23), 26);
        assert_eq!(l.word_left(26), 24);
        assert_eq!(l.word_left(24), 16);
        assert_eq!(l.word_left(16), 9);
        assert_eq!(l.word_left(9), 0);
        assert_eq!(l.word_left(0), 0);
        let umlaut = LineText::new("grüße welt");
        assert_eq!(umlaut.word_right(0), "grüße".len());
    }

    #[test]
    fn a_double_click_selects_the_word_or_the_separator_run() {
        let l = LineText::new("selftest-added  x");
        assert_eq!(l.word_at(3), 0..8);
        assert_eq!(l.word_at(10), 9..14);
        assert_eq!(l.word_at(8), 8..9);
        assert_eq!(l.word_at(15), 14..16);
        assert_eq!(l.word_at(17), 16..17, "past the end: the last word");
        assert_eq!(LineText::new("").word_at(0), 0..0);
    }

    #[test]
    fn undo_takes_back_a_typed_run_at_once_and_redo_brings_it_back() {
        let mut l = LineText::new("");
        typed(&mut l, "abc");
        typed(&mut l, " de");
        assert_eq!(l.text(), "abc de");
        assert!(l.undo());
        assert_eq!(l.text(), "abc", "a space starts a new step");
        assert!(l.undo());
        assert_eq!((l.text(), l.cursor()), ("", 0));
        assert!(!l.undo());
        assert!(l.redo());
        assert!(l.redo());
        assert_eq!(l.text(), "abc de");
        l.move_to(1);
        typed(&mut l, "X");
        assert!(!l.redo(), "an edit drops the redo history");
        l.undo();
        assert_eq!(l.text(), "abc de", "moving the caret ends the typed run");
    }

    #[test]
    fn a_composition_stays_marked_and_commits_as_one_undo_step() {
        let mut l = LineText::new("A");
        l.replace_marked(1..1, "ka", Some(2..2));
        assert_eq!((l.text(), l.marked(), l.selection()), ("Aka", Some(1..3), 3..3));
        l.replace_marked(1..3, "か", Some(3..3));
        assert_eq!((l.text(), l.marked()), ("Aか", Some(1..4)));
        l.replace(l.marked().unwrap(), "仮", EditKind::Typing);
        assert_eq!((l.text(), l.marked(), l.cursor()), ("A仮", None, 4));
        l.undo();
        assert_eq!(l.text(), "A");
    }

    #[test]
    fn utf16_offsets_map_to_bytes_and_back() {
        let l = LineText::new("e🙂漢");
        assert_eq!(l.range_from_utf16(&(1..3)), 1..5);
        assert_eq!(l.range_to_utf16(&(1..8)), 1..4);
        assert_eq!(l.offset_from_utf16(99), 8);
    }

    #[test]
    fn the_filter_maps_and_reports_the_first_refused_character() {
        let lower = |c: char| c.is_ascii_alphabetic().then(|| c.to_ascii_lowercase());
        assert_eq!(filtered("AbC", lower), ("abc".to_string(), None));
        assert_eq!(filtered("a1ä", lower), ("a".to_string(), Some('1')));
    }
}

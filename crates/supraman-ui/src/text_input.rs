//! The one text input; a key not bound in its context ([`bind_keys`]) must propagate, or Windows never makes it a character.

use std::{
    cell::Cell,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    LineLayout, PaintQuad, Pixels, Point, ScrollEvent, SharedString, Style, Subscription, Task, TextRun, UTF16Selection, UnderlineStyle,
    Window, WrappedLine, actions, div, fill, point, prelude::*, px, relative, size,
};
use supraman_term::{blink_phase, terminal_settings::CURSOR_BLINK_INTERVAL};
use theme::ActiveTheme as _;
use unicode_segmentation::UnicodeSegmentation as _;

const CONTEXT: &str = "TextInput";
/// A multi-line input shows at least this many rows, and grows with its text up to [`MAX_ROWS`].
pub const MIN_ROWS: usize = 3;
pub const MAX_ROWS: usize = 8;
/// Undo steps kept; the oldest go first.
const UNDO_STEPS: usize = 100;

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
        Up,
        Down,
        SelectUp,
        SelectDown,
        Home,
        End,
        SelectHome,
        SelectEnd,
        TextStart,
        TextEnd,
        SelectTextStart,
        SelectTextEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Confirm,
        ConfirmBack,
        Submit,
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
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("ctrl-home", TextStart, c),
        KeyBinding::new("ctrl-end", TextEnd, c),
        KeyBinding::new("ctrl-shift-home", SelectTextStart, c),
        KeyBinding::new("ctrl-shift-end", SelectTextEnd, c),
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
        KeyBinding::new("shift-enter", ConfirmBack, c),
        KeyBinding::new("ctrl-enter", Submit, c),
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
    /// Where Up/Down aim on the next row: the x the caret had when the run of vertical moves began.
    goal_x: Option<f32>,
}

/// The text's visual rows as they were painted: what Up/Down/Home/End move by in a multi-line input.
pub trait Rows {
    /// Each visual row's byte range, in order, without the newline that ends a text line. A row that wraps
    /// ends where the next begins.
    fn rows(&self) -> &[Range<usize>];
    /// `offset`'s x within its row.
    fn x_for(&self, offset: usize) -> f32;
    /// The offset in `row` closest to `x`.
    fn offset_at(&self, row: usize, x: f32) -> usize;
}

/// The row `offset` is painted on: at a wrap, the next row's start.
pub fn row_of(rows: &[Range<usize>], offset: usize) -> usize {
    rows.iter()
        .enumerate()
        .position(|(i, r)| offset >= r.start && (offset < r.end || (offset == r.end && rows.get(i + 1).is_none_or(|n| n.start != offset))))
        .unwrap_or(rows.len().saturating_sub(1))
}

/// Whether `row` wraps into the next one (ends where the next begins) rather than at a newline.
fn wraps(rows: &[Range<usize>], row: usize) -> bool {
    rows.get(row + 1).is_some_and(|n| n.start == rows[row].end) && !rows[row].is_empty()
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
        self.goal_x = None;
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
        self.goal_x = None;
    }

    /// `range` selected, the caret at its end (or its start when `reversed`).
    pub fn select(&mut self, range: Range<usize>, reversed: bool) {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        self.selected = start.min(end)..start.max(end);
        self.reversed = reversed;
        self.typing_at = None;
        self.goal_x = None;
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

    /// Up (`delta` -1) or Down (+1) by visual rows, the caret aiming at the x it had when the run of vertical
    /// moves began; past the first row it goes to the start, past the last to the end. `select` extends.
    pub fn move_vertically(&mut self, rows: &(impl Rows + ?Sized), delta: isize, select: bool) {
        let caret = self.cursor();
        let all = rows.rows();
        let x = self.goal_x.unwrap_or_else(|| rows.x_for(caret));
        let target = row_of(all, caret) as isize + delta;
        let to = if target < 0 {
            0
        } else if target as usize >= all.len() {
            self.text.len()
        } else {
            rows.offset_at(target as usize, x)
        };
        if select { self.select_to(to) } else { self.move_to(to) }
        self.goal_x = Some(x);
    }

    /// Home: the start of the caret's visual row.
    pub fn row_start(&self, rows: &[Range<usize>]) -> usize {
        rows.get(row_of(rows, self.cursor())).map_or(0, |r| r.start)
    }

    /// End: the end of the caret's visual row; before its last character when the row wraps, which would
    /// put the caret on the next row.
    pub fn row_end(&self, rows: &[Range<usize>]) -> usize {
        let row = row_of(rows, self.cursor());
        match rows.get(row) {
            Some(r) if wraps(rows, row) => self.prev_grapheme(r.end).max(r.start),
            Some(r) => r.end,
            None => self.text.len(),
        }
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
            self.push_undo();
        }
        self.redo.clear();
        self.text.replace_range(range.clone(), new);
        let caret = range.start + new.len();
        self.selected = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.typing_at = (kind == EditKind::Typing).then_some(caret);
        self.goal_x = None;
    }

    /// The IME's composition: `new` replaces `range` and stays marked, `selected` (relative to `new`) selected
    /// in it. Undo takes back the whole composition in one step.
    pub fn replace_marked(&mut self, range: Range<usize>, new: &str, selected: Option<Range<usize>>) {
        let range = self.clamp(range.start)..self.clamp(range.end);
        if self.marked.is_none() {
            self.push_undo();
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

    fn push_undo(&mut self) {
        if self.undo.len() == UNDO_STEPS {
            self.undo.remove(0);
        }
        self.undo.push(self.snapshot());
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
        self.push_undo();
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
    Edited,
    /// A character the input's filter does not take; nothing of it was inserted.
    Refused(char),
    /// Enter on one line or with [`TextInput::enter_confirms`], Ctrl+Enter on either unless [`TextInput::submit_apart`].
    Confirm,
    /// Ctrl+Enter with [`TextInput::submit_apart`].
    Submit,
    /// Shift+Enter on one line.
    ConfirmBack,
    /// Esc.
    Cancel,
}

/// [`Rows`] without a painted layout: each text line one row, x counted in characters.
pub struct LineRows {
    rows: Vec<Range<usize>>,
    text: String,
}

impl LineRows {
    pub fn new(text: &str) -> Self {
        let mut start = 0;
        let rows = text
            .split('\n')
            .map(|line| {
                let row = start..start + line.len();
                start = row.end + 1;
                row
            })
            .collect();
        Self { rows, text: text.to_string() }
    }
}

impl Rows for LineRows {
    fn rows(&self) -> &[Range<usize>] {
        &self.rows
    }

    fn x_for(&self, offset: usize) -> f32 {
        let row = &self.rows[row_of(&self.rows, offset)];
        self.text[row.start..offset.clamp(row.start, row.end)].chars().count() as f32
    }

    fn offset_at(&self, row: usize, x: f32) -> usize {
        let r = &self.rows[row];
        self.text[r.clone()].char_indices().nth(x.round().max(0.) as usize).map_or(r.end, |(i, _)| r.start + i)
    }
}

/// What was painted last: each text line's shaped layout and where it starts, and the visual rows.
#[derive(Clone, Default)]
struct Layout {
    /// The text shaped; empty while the placeholder shows.
    text: SharedString,
    lines: Vec<(usize, Arc<LineLayout>)>,
    rows: Vec<Range<usize>>,
    /// Per row: its text line, and the x its start has in that line's unwrapped layout.
    row_line: Vec<(usize, Pixels)>,
}

impl Layout {
    fn new(text: &str, lines: &[WrappedLine]) -> Self {
        let mut layout = Self { text: SharedString::from(text.to_string()), ..Default::default() };
        let mut start = 0;
        for (i, line) in lines.iter().enumerate() {
            let unwrapped = line.unwrapped_layout.clone();
            let ends = line.wrap_boundaries().iter().map(|b| unwrapped.runs[b.run_ix].glyphs[b.glyph_ix].index).chain([line.len()]);
            let mut from = 0;
            for end in ends {
                layout.rows.push(start + from..start + end);
                layout.row_line.push((i, unwrapped.x_for_index(from)));
                from = end;
            }
            layout.lines.push((start, unwrapped));
            start += line.len() + 1;
        }
        if layout.rows.is_empty() {
            layout.rows.push(0..0);
            layout.row_line.push((0, px(0.)));
        }
        layout
    }

    /// `offset`'s x on `row` (a wrapped row's end is its own, not the next row's start).
    fn x_in_row(&self, row: usize, offset: usize) -> Pixels {
        let (line, start_x) = self.row_line[row];
        match self.lines.get(line) {
            Some((start, l)) => l.x_for_index(offset.saturating_sub(*start).min(l.len)) - start_x,
            None => px(0.),
        }
    }

    fn x(&self, offset: usize) -> Pixels {
        self.x_in_row(row_of(&self.rows, offset), offset)
    }
}

impl Rows for Layout {
    fn rows(&self) -> &[Range<usize>] {
        &self.rows
    }

    fn x_for(&self, offset: usize) -> f32 {
        f32::from(self.x(offset))
    }

    fn offset_at(&self, row: usize, x: f32) -> usize {
        let Some(r) = self.rows.get(row) else {
            return self.text.len();
        };
        let (line, start_x) = self.row_line[row];
        let Some((start, l)) = self.lines.get(line) else {
            return r.start;
        };
        let at = (start + l.closest_index_for_x(px(x) + start_x)).clamp(r.start, r.end);
        // a wrapped row's end is painted as the next row's start: stay on this row
        match self.text[r.clone()].chars().next_back() {
            Some(c) if at == r.end && wraps(&self.rows, row) => r.end - c.len_utf8(),
            _ => at,
        }
    }
}

/// Room kept right of the text for the caret.
const CARET_ROOM: Pixels = px(2.);
/// The field's padding beside the text, inside its 1 px border.
pub const PAD_X: Pixels = px(8.);
/// The field's padding above and below the text, inside its 1 px border.
pub const PAD_Y: Pixels = px(3.);
/// Where the text starts, from the field's outer left edge: the border and [`PAD_X`].
pub const TEXT_LEFT: Pixels = px(9.);
const LINE_HEIGHT: Pixels = px(18.);
/// A single-line field: one line, [`PAD_Y`] above and below, and the border.
pub const SINGLE_HEIGHT: Pixels = px(26.);
/// A [`TextInput::quiet`] field: one line inside its 1 px border, inside a graph row's band.
pub const QUIET_HEIGHT: Pixels = px(20.);

/// A [`TextInput::quiet`] field's box: a low-contrast fill, no border but the focus colour's while `focused`; a still copy of the field paints it too.
pub fn quiet_frame(focused: bool, cx: &App) -> gpui::Div {
    let colors = cx.theme().colors();
    let border = if focused { colors.border_focused } else { gpui::transparent_black() };
    div().flex().items_center().min_w_0().h(QUIET_HEIGHT).px(PAD_X).rounded_md().border_1().border_color(border).bg(colors.element_background).text_sm().line_height(LINE_HEIGHT).overflow_hidden()
}

pub struct TextInput {
    focus_handle: FocusHandle,
    pub line: LineText,
    placeholder: SharedString,
    /// Maps each typed or pasted character to what is stored, or refuses it.
    filter: Option<fn(char) -> Option<char>>,
    /// Several lines, wrapped: Enter types a new line and Ctrl+Enter confirms.
    multi_line: bool,
    /// On several lines, Enter confirms and Shift+Enter types the new line.
    enter_confirms: bool,
    /// Ctrl+Enter is [`InputEvent::Submit`], told apart from Enter's [`InputEvent::Confirm`].
    submit_apart: bool,
    masked: bool,
    quiet: bool,
    last_layout: Option<Layout>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    /// How far the text is scrolled so the caret stays in view: left on one line, up on several.
    scroll: Point<Pixels>,
    /// The caret moved since the last paint: scroll it into view.
    autoscroll: bool,
    is_selecting: bool,
    /// The last edit, move or focus: the caret is lit for one [`CURSOR_BLINK_INTERVAL`] after it.
    moved_at: Instant,
    /// Whether the last frame painted the caret lit; the selftest reads it.
    pub caret_painted: Cell<bool>,
    _blink: Task<()>,
}

impl EventEmitter<InputEvent> for TextInput {}

/// A filter field of `V`: the input, and the subscription that hands `on_edit` its text after every edit; keep both.
pub fn filter_box<V: 'static>(
    placeholder: &'static str,
    on_edit: impl Fn(&mut V, &str, &mut Context<V>) + 'static,
    window: &mut Window,
    cx: &mut Context<V>,
) -> (Entity<TextInput>, Subscription) {
    let input = cx.new(|cx| TextInput::new(placeholder, None, window, cx));
    let sub = cx.subscribe(&input, move |this, input, e: &InputEvent, cx| {
        if *e == InputEvent::Edited {
            let text = input.read(cx).text().to_string();
            on_edit(this, &text, cx);
        }
    });
    (input, sub)
}

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
        let blink = cx.spawn_in(window, async move |this, cx| {
            loop {
                let wait = this.update_in(cx, |this, window, cx| {
                    if !this.focus_handle.is_focused(window) {
                        return CURSOR_BLINK_INTERVAL;
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
            multi_line: false,
            enter_confirms: false,
            submit_apart: false,
            masked: false,
            quiet: false,
            last_layout: None,
            last_bounds: None,
            last_line_height: LINE_HEIGHT,
            scroll: Point::default(),
            autoscroll: true,
            is_selecting: false,
            moved_at: Instant::now(),
            caret_painted: Cell::new(false),
            _blink: blink,
        }
    }

    /// The task box's kind: wrapped lines, [`MIN_ROWS`] to [`MAX_ROWS`] high.
    pub fn multi_line(mut self) -> Self {
        self.multi_line = true;
        self
    }

    /// [`Self::multi_line`] where Enter confirms and Shift+Enter types the new line: a prompt box.
    pub fn enter_confirms(mut self) -> Self {
        self.multi_line = true;
        self.enter_confirms = true;
        self
    }

    /// Ctrl+Enter says [`InputEvent::Submit`]: a commit message's summary, where Enter moves on.
    pub fn submit_apart(mut self) -> Self {
        self.submit_apart = true;
        self
    }

    /// A passphrase's field: one `*` per byte shown, and nothing copied or cut.
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    /// A light one-line field in [`quiet_frame`]: the graph's WIP row's summary.
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    pub fn text(&self) -> &str {
        self.line.text()
    }

    /// What is laid out: the text, or as many `*` as it has bytes.
    fn shown(&self) -> SharedString {
        if self.masked { "*".repeat(self.line.text().len()).into() } else { SharedString::from(self.line.text().to_string()) }
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.lit(cx);
    }

    /// The whole text replaced from outside (not an edit: no event).
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.line.set(text);
        self.lit(cx);
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Focused, everything selected (Ctrl+Shift+E on the search box).
    pub fn focus_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_all();
        self.focus_handle.focus(window, cx);
        self.lit(cx);
    }

    /// Whether the caret is painted at `now`: lit for one interval after an edit or move, then blinking.
    pub fn caret_lit(&self, now: Instant) -> bool {
        blink_phase(now.saturating_duration_since(self.moved_at)).0
    }

    fn next_blink(&self, now: Instant) -> Duration {
        blink_phase(now.saturating_duration_since(self.moved_at)).1
    }

    fn lit(&mut self, cx: &mut Context<Self>) {
        self.moved_at = Instant::now();
        self.autoscroll = true;
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

    /// Up/Down by visual rows; on one line the key goes on (it is not the input's).
    fn vertical(&mut self, delta: isize, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.multi_line {
            cx.propagate();
            return;
        }
        {
            let rows = painted_rows(&self.last_layout, self.line.text());
            self.line.move_vertically(rows.as_ref(), delta, select);
        }
        self.moved(window, cx);
    }

    fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, window, cx);
    }

    fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, false, window, cx);
    }

    fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, window, cx);
    }

    fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, window, cx);
    }

    fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_start(painted_rows(&self.last_layout, self.line.text()).rows());
        self.move_to(to, window, cx);
    }

    fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_end(painted_rows(&self.last_layout, self.line.text()).rows());
        self.move_to(to, window, cx);
    }

    fn select_home(&mut self, _: &SelectHome, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_start(painted_rows(&self.last_layout, self.line.text()).rows());
        self.select_to(to, window, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_end(painted_rows(&self.last_layout, self.line.text()).rows());
        self.select_to(to, window, cx);
    }

    fn text_start(&mut self, _: &TextStart, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
    }

    fn text_end(&mut self, _: &TextEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line.text().len(), window, cx);
    }

    fn select_text_start(&mut self, _: &SelectTextStart, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, window, cx);
    }

    fn select_text_end(&mut self, _: &SelectTextEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.text().len(), window, cx);
    }

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_all();
        self.moved(window, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        if !s.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.line.text()[s].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        if !s.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(self.line.text()[s.clone()].to_string()));
            self.line.replace(s, "", EditKind::Other);
            self.edited(window, cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = pasted(&text, self.multi_line);
            self.insert(None, &text, EditKind::Other, window, cx);
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

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.multi_line && !self.enter_confirms {
            self.insert(None, "\n", EditKind::Other, window, cx);
        } else {
            cx.emit(InputEvent::Confirm);
        }
    }

    fn confirm_back(&mut self, _: &ConfirmBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.multi_line {
            self.insert(None, "\n", EditKind::Other, window, cx);
        } else {
            cx.emit(InputEvent::ConfirmBack);
        }
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(if self.submit_apart { InputEvent::Submit } else { InputEvent::Confirm });
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }

    /// The offset at a window position, from the last paint.
    fn index_at(&self, position: Point<Pixels>) -> Option<usize> {
        let (bounds, layout) = (self.last_bounds?, self.last_layout.as_ref()?);
        if layout.text.as_ref() != self.line.text() {
            return None;
        }
        let local = position - bounds.origin + self.scroll;
        let row = ((local.y / self.last_line_height).floor().max(0.) as usize).min(layout.rows.len() - 1);
        Some(layout.offset_at(row, f32::from(local.x)))
    }

    fn index_for_mouse(&self, position: Point<Pixels>) -> usize {
        if self.line.text().is_empty() {
            return 0;
        }
        self.index_at(position).unwrap_or(0)
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

    /// The wheel scrolls a box whose text is taller than it; the panel's scroll gets it otherwise.
    fn scroll_wheel(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.last_layout.as_ref().map_or(1, |l| l.rows.len());
        if rows <= MAX_ROWS {
            return;
        }
        cx.stop_propagation();
        self.scroll.y -= event.delta.pixel_delta(self.last_line_height).y;
        self.autoscroll = false;
        cx.notify();
    }

    /// What is shaped: the text with the IME's composition underlined, or the placeholder.
    fn shaping(&self, window: &Window, cx: &App) -> (SharedString, Vec<TextRun>, Pixels) {
        let text = self.line.text();
        let style = window.text_style();
        let (shown, color) = if text.is_empty() {
            (self.placeholder.clone(), cx.theme().colors().text_placeholder)
        } else {
            (self.shown(), style.color)
        };
        let run = TextRun { len: shown.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs = match self.line.marked().filter(|_| !text.is_empty()) {
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
        (shown, runs, style.font_size.to_pixels(window.rem_size()))
    }
}

/// What a paste inserts: on one line its line breaks as spaces (a trailing one dropped), on several `\n`.
/// Typed text without its tabs: Tab moves between a form's fields (`focus_order`), it never types.
fn without_tabs(text: &str) -> String {
    text.replace('\t', "")
}

pub fn pasted(text: &str, multi_line: bool) -> String {
    if multi_line {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.trim_end_matches(['\r', '\n']).replace("\r\n", " ").replace(['\r', '\n'], " ")
    }
}

/// The visual rows painted last, or the text lines while the last paint is older than the text.
fn painted_rows<'a>(layout: &'a Option<Layout>, text: &str) -> Box<dyn Rows + 'a> {
    match layout.as_ref().filter(|l| l.text.as_ref() == text) {
        Some(layout) => Box::new(layout.clone()),
        None => Box::new(LineRows::new(text)),
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
        self.insert(range, &without_tabs(text), EditKind::Typing, window, cx);
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
        let row = row_of(&layout.rows, range.start);
        let end = if row_of(&layout.rows, range.end) == row { range.end } else { layout.rows[row].end };
        let (top, lh) = (bounds.top() + self.last_line_height * row as f32 - self.scroll.y, self.last_line_height);
        let x = |i: usize| bounds.left() + layout.x_in_row(row, i) - self.scroll.x;
        Some(Bounds::from_corners(point(x(range.start), top), point(x(end), top + lh)))
    }

    fn character_index_for_point(&mut self, point: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.last_bounds?.localize(&point)?;
        let at = self.index_at(point)?;
        Some(self.line.offset_to_utf16(at))
    }
}

/// The input's text (or the placeholder): wrapped rows when multi-line, the selection, the IME's underline
/// and the caret.
struct TextElement {
    input: Entity<TextInput>,
}

struct Prepainted {
    lines: Vec<WrappedLine>,
    /// The visual row each text line starts on.
    first_rows: Vec<usize>,
    layout: Layout,
    scroll: Point<Pixels>,
    selections: Vec<PaintQuad>,
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
        let input = self.input.read(cx);
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if !input.multi_line {
            style.size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        }
        let (shown, runs, font_size) = input.shaping(window, cx);
        let id = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let rows: usize = window
                .text_system()
                .shape_text(shown.clone(), font_size, &runs, width.map(|w| w - CARET_ROOM), None)
                .map_or(1, |lines| lines.iter().map(|l| l.wrap_boundaries().len() + 1).sum());
            size(width.unwrap_or_default(), line_height * rows.clamp(MIN_ROWS, MAX_ROWS) as f32)
        });
        (id, ())
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
        let line_height = window.line_height();
        let (shown, runs, font_size) = input.shaping(window, cx);
        let wrap = input.multi_line.then(|| bounds.size.width - CARET_ROOM);
        let lines: Vec<WrappedLine> = window.text_system().shape_text(shown, font_size, &runs, wrap, None).map(|l| l.into_iter().collect()).unwrap_or_default();
        let layout = if text.is_empty() { Layout::new("", &[]) } else { Layout::new(&input.shown(), &lines) };
        let mut first_rows = Vec::with_capacity(lines.len());
        let mut rows = 0;
        for line in &lines {
            first_rows.push(rows);
            rows += line.wrap_boundaries().len() + 1;
        }

        let (selected, cursor) = (input.line.selection(), input.line.cursor());
        let caret_row = row_of(&layout.rows, cursor);
        let caret = point(layout.x_in_row(caret_row, cursor), line_height * caret_row as f32);
        let mut scroll = input.scroll;
        if input.multi_line {
            scroll.x = px(0.);
            let height = bounds.size.height;
            if input.autoscroll {
                if caret.y < scroll.y {
                    scroll.y = caret.y;
                } else if caret.y + line_height > scroll.y + height {
                    scroll.y = caret.y + line_height - height;
                }
            }
            scroll.y = scroll.y.clamp(px(0.), (line_height * rows.max(1) as f32 - height).max(px(0.)));
        } else {
            scroll.y = px(0.);
            let width = bounds.size.width - CARET_ROOM;
            let line_width = lines.first().map_or(px(0.), |l| l.unwrapped_layout.width);
            scroll.x = scroll.x.min((line_width - width).max(px(0.)));
            if caret.x - scroll.x > width {
                scroll.x = caret.x - width;
            } else if caret.x < scroll.x {
                scroll.x = caret.x;
            }
        }
        let focused = input.focus_handle.is_focused(window);
        let lit = focused && input.caret_lit(Instant::now());
        input.caret_painted.set(lit);
        let origin = bounds.origin - scroll;
        let newline = px(5.);
        let selections = if selected.is_empty() {
            Vec::new()
        } else {
            layout
                .rows
                .iter()
                .enumerate()
                .filter_map(|(i, r)| {
                    let (start, end) = (selected.start.max(r.start), selected.end.min(r.end));
                    // a selected newline shows as a little room after its row
                    let past = r.end < text.len() && !wraps(&layout.rows, i) && selected.start <= r.end && selected.end > r.end;
                    (start < end || past).then(|| {
                        let top = origin.y + line_height * i as f32;
                        let right = layout.x_in_row(i, end) + if past { newline } else { px(0.) };
                        fill(
                            Bounds::from_corners(point(origin.x + layout.x_in_row(i, start), top), point(origin.x + right, top + line_height)),
                            colors.element_selection_background,
                        )
                    })
                })
                .collect()
        };
        let caret = lit.then(|| fill(Bounds::new(origin + caret, size(px(1.5), line_height)), colors.text));
        self.input.update(cx, |input, _| input.scroll = scroll);
        Prepainted { lines, first_rows, layout, scroll, selections, caret }
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
        let line_height = window.line_height();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for selection in prepaint.selections.drain(..) {
                window.paint_quad(selection);
            }
            let origin = bounds.origin - prepaint.scroll;
            for (line, first) in prepaint.lines.iter().zip(&prepaint.first_rows) {
                line.paint(point(origin.x, origin.y + line_height * *first as f32), line_height, gpui::TextAlign::Left, None, window, cx).ok();
            }
            if let Some(caret) = prepaint.caret.take() {
                window.paint_quad(caret);
            }
        });
        let layout = std::mem::take(&mut prepaint.layout);
        self.input.update(cx, |input, _| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
            input.last_line_height = line_height;
            input.autoscroll = false;
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let focused = self.focus_handle.is_focused(window);
        let multi_line = self.multi_line;
        let frame = if self.quiet && !multi_line {
            quiet_frame(focused, cx)
        } else {
            div()
                .flex()
                .min_w_0()
                .when(!multi_line, |d| d.items_center().h(SINGLE_HEIGHT))
                .when(multi_line, |d| d.py(PAD_Y).on_scroll(cx.listener(Self::scroll_wheel)))
                .px(PAD_X)
                .rounded_sm()
                .border_1()
                .border_color(if focused { colors.border_focused } else { colors.border })
                .bg(colors.editor_background)
                .text_sm()
                .line_height(LINE_HEIGHT)
                .overflow_hidden()
        };
        frame
            .w_full()
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
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::text_start))
            .on_action(cx.listener(Self::text_end))
            .on_action(cx.listener(Self::select_text_start))
            .on_action(cx.listener(Self::select_text_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::confirm_back))
            .on_action(cx.listener(Self::submit))
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
    fn the_text_has_room_inside_the_border() {
        assert!(PAD_X >= px(6.) && PAD_Y >= px(2.), "padding {PAD_X:?} × {PAD_Y:?}");
        assert_eq!(TEXT_LEFT, px(1.) + PAD_X);
        assert_eq!(SINGLE_HEIGHT, LINE_HEIGHT + PAD_Y * 2. + px(2.));
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
    fn undo_keeps_the_newest_steps_only() {
        let mut l = LineText::new("");
        for _ in 0..UNDO_STEPS + 20 {
            typed(&mut l, " x");
        }
        let mut steps = 0;
        while l.undo() {
            steps += 1;
        }
        assert_eq!(steps, UNDO_STEPS);
        assert_eq!(l.text().len(), 2 * 20, "the oldest steps went first");
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

    /// Rows as a box `width` characters wide would wrap them: at the last space that fits, else mid-word.
    fn wrapped(text: &str, width: usize) -> LineRows {
        let mut rows = Vec::new();
        let mut start = 0;
        for line in text.split('\n') {
            let mut from = 0;
            while line.len() - from > width {
                let cut = line[from..from + width].rfind(' ').map_or(from + width, |i| from + i + 1);
                rows.push(start + from..start + cut);
                from = cut;
            }
            rows.push(start + from..start + line.len());
            start += line.len() + 1;
        }
        LineRows { rows, text: text.to_string() }
    }

    #[test]
    fn up_and_down_keep_the_column_across_shorter_lines() {
        let text = "abcdef\nxy\nlonger line";
        let rows = LineRows::new(text);
        let mut l = line(text, 4);
        l.move_vertically(&rows, 1, false);
        assert_eq!(l.cursor(), 9, "line 2 is shorter: its end");
        l.move_vertically(&rows, 1, false);
        assert_eq!(l.cursor(), 14, "back at column 4 on line 3");
        l.move_vertically(&rows, -1, false);
        l.move_vertically(&rows, -1, false);
        assert_eq!(l.cursor(), 4);
        l.move_vertically(&rows, -1, false);
        assert_eq!(l.cursor(), 0, "Up on the first row: the start");
        l.move_to(12);
        l.move_vertically(&rows, 1, false);
        assert_eq!(l.cursor(), text.len(), "Down on the last row: the end");
        l.move_to(14);
        l.move_vertically(&rows, -1, false);
        l.move_to(l.cursor());
        l.move_vertically(&rows, 1, false);
        assert_eq!(l.cursor(), 12, "a horizontal move resets the column: 2 on line 2 is 2 on line 3");
    }

    #[test]
    fn home_end_and_vertical_moves_follow_wrapped_rows() {
        let text = "abcd efgh ij\nxyz";
        let rows = wrapped(text, 5);
        assert_eq!(rows.rows(), &[0..5, 5..10, 10..12, 13..16]);
        assert_eq!(row_of(rows.rows(), 5), 1, "a wrap offset is the next row's start");
        assert_eq!(row_of(rows.rows(), 12), 2, "a line's end is its own row's");
        let mut l = line(text, 7);
        assert_eq!((l.row_start(rows.rows()), l.row_end(rows.rows())), (5, 9), "End stops before the wrap's space");
        l.move_vertically(&rows, -1, false);
        assert_eq!(l.cursor(), 2);
        l.move_vertically(&rows, 1, false);
        l.move_vertically(&rows, 1, false);
        assert_eq!(l.cursor(), 12, "row 3 is two characters: its end");
        l.move_vertically(&rows, 1, false);
        assert_eq!(l.cursor(), 15, "column 2 again on the next line");
        assert_eq!((l.row_start(rows.rows()), l.row_end(rows.rows())), (13, 16));
    }

    #[test]
    fn shift_up_and_down_select_across_lines() {
        let text = "first line\nsecond";
        let rows = LineRows::new(text);
        let mut l = line(text, 3);
        l.move_vertically(&rows, 1, true);
        assert_eq!((l.selection(), l.cursor()), (3..14, 14));
        assert_eq!(&l.text()[l.selection()], "st line\nsec");
        l.move_vertically(&rows, -1, true);
        assert_eq!(l.selection(), 3..3, "back to the anchor");
        l.move_vertically(&rows, -1, true);
        assert_eq!((l.selection(), l.cursor()), (0..3, 0), "above the first row: to the start");
        l.select_all();
        l.replace(l.selection(), "X", EditKind::Typing);
        assert_eq!(l.text(), "X", "typing replaces a selection over both lines");
    }

    #[test]
    fn a_paste_keeps_its_lines_on_several_and_joins_them_on_one() {
        assert_eq!(pasted("a\r\nb\rc\n", true), "a\nb\nc\n");
        assert_eq!(pasted("1\r\n2\n", false), "1 2");
        let mut l = line("ab\ncd", 1);
        l.select(1..4, false);
        l.replace(l.selection(), &pasted("X\r\nY", true), EditKind::Other);
        assert_eq!((l.text(), l.cursor()), ("aX\nYd", 4));
        let rows = LineRows::new(l.text());
        assert_eq!((l.row_start(rows.rows()), l.row_end(rows.rows())), (3, 5), "the caret is on the pasted second line");
        l.undo();
        assert_eq!(l.text(), "ab\ncd", "one undo step");
    }

    #[test]
    fn word_moves_cross_line_breaks() {
        let l = LineText::new("fix the\nbuild now");
        assert_eq!(l.word_right(7), 13, "over the newline to the end of `build`");
        assert_eq!(l.word_left(8), 4, "from the start of line 2 to `the`");
        assert_eq!(l.word_at(7), 7..8, "a double-click on the newline selects only it");
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

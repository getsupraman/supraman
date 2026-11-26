//! The one text engine (UI-KIT §4.5): [`TextInput`] edits, and read only it is a [`TextView`]'s; a key not bound in its
//! context ([`bind_keys`]) must propagate, or Windows never makes it a character.

mod edit;
mod element;
mod line;
mod scroll;
mod view;

use std::cell::Cell;

use gpui::{
    App, Bounds, Context, CursorStyle, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton, Pixels, Point, SharedString,
    Subscription, Window, actions, div, prelude::*, px,
};
use theme::ActiveTheme as _;

use crate::blink::Blink;
use crate::tokens;
use element::{Layout, TextElement};
pub use line::{EditKind, LineRows, LineText, Rows, filtered, pasted, row_of};
#[cfg(test)]
use line::UNDO_STEPS;
use scroll::TextScroll;
pub use view::TextView;

const CONTEXT: &str = "TextInput";
const VIEW_CONTEXT: &str = "TextView";
/// A multi-line input shows at least this many rows, and grows with its text up to [`MAX_ROWS`].
pub const MIN_ROWS: usize = 3;
pub const MAX_ROWS: usize = 8;

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
    let v = Some(VIEW_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("shift-left", SelectLeft, v),
        KeyBinding::new("shift-right", SelectRight, v),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, v),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, v),
        KeyBinding::new("shift-up", SelectUp, v),
        KeyBinding::new("shift-down", SelectDown, v),
        KeyBinding::new("shift-home", SelectHome, v),
        KeyBinding::new("shift-end", SelectEnd, v),
        KeyBinding::new("ctrl-shift-home", SelectTextStart, v),
        KeyBinding::new("ctrl-shift-end", SelectTextEnd, v),
        KeyBinding::new("ctrl-a", SelectAll, v),
        KeyBinding::new("ctrl-c", Copy, v),
        KeyBinding::new("ctrl-insert", Copy, v),
    ]);
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
    /// A press on read-only text, after it took the keys and placed the selection: its owner may make it editable.
    Pressed,
}

/// Room kept right of the text for the caret.
const CARET_ROOM: Pixels = px(2.);
/// The field's padding beside the text, inside its 1 px border.
pub const PAD_X: Pixels = tokens::space::L;
/// The field's padding above and below the text, inside its 1 px border.
pub const PAD_Y: Pixels = px(3.);
/// Where the text starts, from the field's outer left edge: the border and [`PAD_X`].
pub const TEXT_LEFT: Pixels = px(9.);
const LINE_HEIGHT: Pixels = px(18.);
/// A single-line field: one line, [`PAD_Y`] above and below, and the border.
pub const SINGLE_HEIGHT: Pixels = tokens::control::MEDIUM.height;
/// A [`TextInput::quiet`] field: one line inside its 1 px border, inside a graph row's band.
pub const QUIET_HEIGHT: Pixels = tokens::control::SMALL.height;

/// A [`TextInput::quiet`] field's box: a low-contrast fill, no border but the focus colour's while `focused`; a still copy of the field paints it too.
pub fn quiet_frame(focused: bool, cx: &App) -> gpui::Div {
    let colors = cx.theme().colors();
    let border = if focused { colors.border_focused } else { gpui::transparent_black() };
    div().flex().items_center().min_w_0().h(QUIET_HEIGHT).px(PAD_X).rounded(tokens::radius::M).border(tokens::stroke::HAIRLINE).border_color(border).bg(colors.element_background).text_size(tokens::text::BODY).line_height(LINE_HEIGHT).overflow_hidden()
}

/// An [`inline_frame`]'s padding beside the text, inside its 1 px border.
const INLINE_PAD_X: Pixels = tokens::space::S;
/// Where an [`inline_frame`]'s text starts, from its outer left edge: the border and [`INLINE_PAD_X`].
pub const INLINE_TEXT_LEFT: Pixels = px(5.);

/// A [`TextInput::inline`] field's box: the text style it sits in, the full height, the focus colour's border, the error colour's while `invalid`.
pub fn inline_frame(focused: bool, invalid: bool, cx: &App) -> gpui::Div {
    let colors = cx.theme().colors();
    let border = if invalid { cx.theme().status().error } else if focused { colors.border_focused } else { colors.border };
    div().flex().items_center().min_w_0().h_full().px(INLINE_PAD_X).rounded(tokens::radius::S).border(tokens::stroke::HAIRLINE).border_color(border).bg(colors.editor_background).overflow_hidden()
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
    /// Laid out over wrapped rows: [`Self::multi_line`], or one line of text that wraps ([`Self::wrapped`]).
    wrap: bool,
    /// No frame: the text in the style it sits in, every row shown.
    plain: bool,
    /// Selectable and copied, never edited: no caret, no IME, the [`VIEW_CONTEXT`] keys.
    read_only: bool,
    masked: bool,
    quiet: bool,
    inline: bool,
    /// What is typed is refused: the border in the error colour.
    invalid: bool,
    last_layout: Option<Layout>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Pixels,
    /// How far the text is scrolled so the caret stays in view: left on one line, up on several.
    scroll: Point<Pixels>,
    /// The rows' scroll as the scrollbar reads and moves it.
    bar: TextScroll,
    /// The caret moved since the last paint: scroll it into view.
    autoscroll: bool,
    is_selecting: bool,
    blink: Entity<Blink>,
    /// Whether the last frame painted the caret lit; the selftest reads it.
    pub caret_painted: Cell<bool>,
    _blink: [Subscription; 4],
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
        let (blink, following) = Blink::follow_while(&focus_handle, |this: &Self| !this.read_only, window, cx);
        cx.on_focus(&focus_handle, window, |this: &mut Self, _, cx| this.lit(cx)).detach();
        Self {
            focus_handle,
            line: LineText::default(),
            placeholder: placeholder.into(),
            filter,
            multi_line: false,
            wrap: false,
            plain: false,
            read_only: false,
            enter_confirms: false,
            submit_apart: false,
            masked: false,
            quiet: false,
            inline: false,
            invalid: false,
            last_layout: None,
            last_bounds: None,
            last_line_height: LINE_HEIGHT,
            scroll: Point::default(),
            bar: TextScroll::default(),
            autoscroll: true,
            is_selecting: false,
            blink,
            caret_painted: Cell::new(false),
            _blink: following,
        }
    }

    /// The task box's kind: wrapped lines, [`MIN_ROWS`] to [`MAX_ROWS`] high.
    pub fn multi_line(mut self) -> Self {
        self.multi_line = true;
        self.wrap = true;
        self
    }

    /// One line of text laid out over wrapped rows: Enter confirms, a paste's line breaks become spaces.
    pub fn wrapped(mut self) -> Self {
        self.wrap = true;
        self
    }

    /// No frame: the text in the font, size, weight and colour it sits in, as tall as all its rows.
    pub fn plain(mut self) -> Self {
        self.plain = true;
        self
    }

    /// Selectable and copied, never edited ([`TextView`]'s): no caret, no placeholder, no IME.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Edited or read only from now on, the text, its layout and the selection kept: only the caret comes or goes.
    pub fn set_read_only(&mut self, read_only: bool, window: &Window, cx: &mut Context<Self>) {
        if self.read_only == read_only {
            return;
        }
        self.read_only = read_only;
        let on = !read_only && self.focus_handle.is_focused(window) && window.is_window_active();
        self.blink.update(cx, |b, cx| b.set_running(on, cx));
        self.lit(cx);
    }

    /// [`Self::multi_line`] where Enter confirms and Shift+Enter types the new line: a prompt box.
    pub fn enter_confirms(mut self) -> Self {
        self.multi_line = true;
        self.wrap = true;
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

    /// A one-line field in its row's own text style, as tall as the room it is given ([`inline_frame`]): a tree row's name.
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self
    }

    /// The border in the error colour while what is typed is refused.
    pub fn set_invalid(&mut self, invalid: bool, cx: &mut Context<Self>) {
        if self.invalid != invalid {
            self.invalid = invalid;
            cx.notify();
        }
    }

    pub fn invalid(&self) -> bool {
        self.invalid
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

    /// Everything selected, the focus left where it is (a field [`crate::focus::request`]ed as it opens).
    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.line.select_all();
        self.lit(cx);
    }

    /// Focused, everything selected (Ctrl+Shift+E on the search box).
    pub fn focus_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_all();
        self.focus_handle.focus(window, cx);
        self.lit(cx);
    }

    /// Where `offset` was last painted, in the window: its x on its row, the row's middle; a test's or selftest's press.
    pub fn painted_at(&self, offset: usize) -> Option<Point<Pixels>> {
        let (bounds, layout) = (self.last_bounds?, self.last_layout.as_ref()?);
        let row = row_of(&layout.rows, offset);
        let x = layout.x_in_row(row, offset) - self.scroll.x;
        Some(gpui::point(bounds.left() + x, bounds.top() + self.last_line_height * (row as f32 + 0.5) - self.scroll.y))
    }

    /// The box and each row's byte range and where it ends, as last painted: what a mode switch must leave alone.
    pub fn painted_rows(&self) -> Option<(Bounds<Pixels>, Vec<(std::ops::Range<usize>, Pixels)>)> {
        let (bounds, layout) = (self.last_bounds?, self.last_layout.as_ref()?);
        Some((bounds, layout.rows.iter().enumerate().map(|(n, r)| (r.clone(), layout.x_in_row(n, r.end))).collect()))
    }

    /// Whether the caret shows: it holds the keys in an active window and its [`Blink`] is on.
    pub fn caret_shown(&self, window: &Window, cx: &App) -> bool {
        !self.read_only && self.focus_handle.is_focused(window) && window.is_window_active() && self.blink.read(cx).visible()
    }

    fn lit(&mut self, cx: &mut Context<Self>) {
        self.blink.update(cx, |b, cx| b.pause(cx));
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

}

impl TextInput {
    /// A bordered multi-line field's scrollbar over its rows.
    fn scrollbar(&self, cx: &Context<Self>) -> Option<crate::scroll::Scrollbar> {
        let bordered = self.wrap && !self.plain;
        bordered.then(|| crate::scroll::Scrollbar::new(gpui::ElementId::NamedInteger("text-input-bar".into(), cx.entity_id().as_u64()), &self.bar))
    }

    /// The box: plain, inline, quiet, or the bordered field (multi-line with its scrollbar).
    fn frame(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let colors = cx.theme().colors();
        let focused = self.focus_handle.is_focused(window);
        let multi_line = self.wrap;
        if self.plain {
            return div().flex().min_w_0();
        }
        if self.inline && !multi_line {
            return inline_frame(focused, self.invalid, cx);
        }
        if self.quiet && !multi_line {
            return quiet_frame(focused, cx);
        }
        div()
            .relative()
            .flex()
            .min_w_0()
            .when(!multi_line, |d| d.items_center().h(SINGLE_HEIGHT))
            .when(multi_line, |d| d.py(PAD_Y).on_scroll(cx.listener(Self::scroll_wheel)))
            .px(PAD_X)
            .rounded(tokens::radius::S)
            .border(tokens::stroke::HAIRLINE)
            .border_color(if focused { colors.border_focused } else { colors.border })
            .bg(colors.editor_background)
            .text_size(tokens::text::BODY)
            .line_height(LINE_HEIGHT)
            .overflow_hidden()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame(window, cx)
            .w_full()
            .key_context(if self.read_only { VIEW_CONTEXT } else { CONTEXT })
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
            .children(self.scrollbar(cx))
    }
}

#[cfg(test)]
mod tests;

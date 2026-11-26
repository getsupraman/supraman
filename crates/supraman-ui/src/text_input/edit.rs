//! What a key, the mouse and the IME do to a [`TextInput`]; read only, only what selects and copies.

use std::ops::Range;

use gpui::{
    Bounds, Context, EntityInputHandler, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollEvent, UTF16Selection, Window, point,
};

use super::element::painted_rows;
use super::line::{EditKind, Rows, filtered, pasted, row_of, without_tabs};
use super::*;

impl TextInput {
    pub(super) fn move_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.line.move_to(offset);
        self.moved(window, cx);
    }

    pub(super) fn select_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_to(offset);
        self.moved(window, cx);
    }

    /// `text` in place of `range` (the selection when `None`), through the filter.
    pub(super) fn insert(&mut self, range: Option<Range<usize>>, text: &str, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
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

    pub(super) fn delete_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
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

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.prev_grapheme(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.next_grapheme(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    pub(super) fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.word_left(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    pub(super) fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.word_right(self.line.cursor());
        self.delete_to(to, window, cx);
    }

    pub(super) fn left(&mut self, _: &Left, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        let to = if s.is_empty() { self.line.prev_grapheme(s.start) } else { s.start };
        self.move_to(to, window, cx);
    }

    pub(super) fn right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        let to = if s.is_empty() { self.line.next_grapheme(s.end) } else { s.end };
        self.move_to(to, window, cx);
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.prev_grapheme(self.line.cursor()), window, cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.next_grapheme(self.line.cursor()), window, cx);
    }

    pub(super) fn word_left(&mut self, _: &WordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line.word_left(self.line.cursor()), window, cx);
    }

    pub(super) fn word_right(&mut self, _: &WordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line.word_right(self.line.cursor()), window, cx);
    }

    pub(super) fn select_word_left(&mut self, _: &SelectWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.word_left(self.line.cursor()), window, cx);
    }

    pub(super) fn select_word_right(&mut self, _: &SelectWordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.word_right(self.line.cursor()), window, cx);
    }

    /// Up/Down by visual rows; on one line the key goes on (it is not the input's).
    pub(super) fn vertical(&mut self, delta: isize, select: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.wrap {
            cx.propagate();
            return;
        }
        {
            let rows = painted_rows(&self.last_layout, self.line.text());
            self.line.move_vertically(rows.as_ref(), delta, select);
        }
        self.moved(window, cx);
    }

    pub(super) fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, window, cx);
    }

    pub(super) fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, false, window, cx);
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, true, window, cx);
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1, true, window, cx);
    }

    pub(super) fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_start(painted_rows(&self.last_layout, self.line.text()).rows());
        self.move_to(to, window, cx);
    }

    pub(super) fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_end(painted_rows(&self.last_layout, self.line.text()).rows());
        self.move_to(to, window, cx);
    }

    pub(super) fn select_home(&mut self, _: &SelectHome, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_start(painted_rows(&self.last_layout, self.line.text()).rows());
        self.select_to(to, window, cx);
    }

    pub(super) fn select_end(&mut self, _: &SelectEnd, window: &mut Window, cx: &mut Context<Self>) {
        let to = self.line.row_end(painted_rows(&self.last_layout, self.line.text()).rows());
        self.select_to(to, window, cx);
    }

    pub(super) fn text_start(&mut self, _: &TextStart, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
    }

    pub(super) fn text_end(&mut self, _: &TextEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line.text().len(), window, cx);
    }

    pub(super) fn select_text_start(&mut self, _: &SelectTextStart, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, window, cx);
    }

    pub(super) fn select_text_end(&mut self, _: &SelectTextEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line.text().len(), window, cx);
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.line.select_all();
        self.moved(window, cx);
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        if !s.is_empty() && !self.masked {
            crate::clipboard::write_text(cx, self.line.text()[s].to_string());
        }
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.line.selection();
        if !s.is_empty() && !self.masked && !self.read_only {
            crate::clipboard::write_text(cx, self.line.text()[s.clone()].to_string());
            self.line.replace(s, "", EditKind::Other);
            self.edited(window, cx);
        }
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = crate::clipboard::text(cx) {
            let text = pasted(&text, self.multi_line);
            self.insert(None, &text, EditKind::Other, window, cx);
        }
    }

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if !self.read_only && self.line.undo() {
            self.edited(window, cx);
        }
    }

    pub(super) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if !self.read_only && self.line.redo() {
            self.edited(window, cx);
        }
    }

    pub(super) fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.multi_line && !self.enter_confirms {
            self.insert(None, "\n", EditKind::Other, window, cx);
        } else {
            cx.emit(InputEvent::Confirm);
        }
    }

    pub(super) fn confirm_back(&mut self, _: &ConfirmBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.multi_line {
            self.insert(None, "\n", EditKind::Other, window, cx);
        } else {
            cx.emit(InputEvent::ConfirmBack);
        }
    }

    pub(super) fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(if self.submit_apart { InputEvent::Submit } else { InputEvent::Confirm });
    }

    pub(super) fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }

    /// The offset at a window position, from the last paint.
    pub(super) fn index_at(&self, position: Point<Pixels>) -> Option<usize> {
        let (bounds, layout) = (self.last_bounds?, self.last_layout.as_ref()?);
        if layout.text.as_ref() != self.line.text() {
            return None;
        }
        let local = position - bounds.origin + self.scroll;
        let row = ((local.y / self.last_line_height).floor().max(0.) as usize).min(layout.rows.len() - 1);
        Some(layout.offset_at(row, f32::from(local.x)))
    }

    pub(super) fn index_for_mouse(&self, position: Point<Pixels>) -> usize {
        if self.line.text().is_empty() {
            return 0;
        }
        self.index_at(position).unwrap_or(0)
    }

    pub(super) fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
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
                let line = self.line.line_at(at);
                self.line.select(line, false);
                self.moved(window, cx);
            }
            _ => {
                self.is_selecting = true;
                if event.modifiers.shift { self.select_to(at, window, cx) } else { self.move_to(at, window, cx) }
            }
        }
        if self.read_only {
            cx.emit(InputEvent::Pressed);
        }
    }

    pub(super) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    pub(super) fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting && event.dragging() {
            self.select_to(self.index_for_mouse(event.position), window, cx);
        }
    }

    /// The wheel scrolls a box whose text is taller than it; the panel's scroll gets it otherwise.
    pub(super) fn scroll_wheel(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.last_layout.as_ref().map_or(1, |l| l.rows.len());
        if rows <= MAX_ROWS {
            return;
        }
        cx.stop_propagation();
        self.scroll.y -= event.delta.pixel_delta(self.last_line_height).y;
        self.autoscroll = false;
        cx.notify();
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

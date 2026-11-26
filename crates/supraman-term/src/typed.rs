//! The terminal as gpui's text consumer (UI-KIT §8): composed characters, AltGr characters and IME text
//! arrive through its input handler and go to the PTY as UTF-8; the keys the escape map knows go from `key_down`.

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, KeyDownEvent, Pixels, Point, UTF16Selection, Window, point, size};

use crate::Terminal;

fn is_text(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(char::is_control)
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl Terminal {
    /// Whether the key was taken here. With `handler_shown` (the terminal's input handler registered, its
    /// focus holding the keys) a key that types a character is left untaken: Windows' WM_CHAR, or gpui's
    /// fallback for a dispatched keystroke, brings its text to [`EntityInputHandler::replace_text_in_range`].
    pub fn key_down(&mut self, event: &KeyDownEvent, handler_shown: bool) -> bool {
        let text = event.keystroke.key_char.as_deref().filter(|c| is_text(c));
        if handler_shown && text.is_some() && event.prefer_character_input {
            return false;
        }
        if self.try_keystroke(&event.keystroke, false) {
            return true;
        }
        match text {
            Some(_) if handler_shown => false,
            Some(text) => {
                self.input(text.to_owned().into_bytes());
                true
            }
            None => true,
        }
    }

    /// The composition an IME shows at the cursor, not yet sent.
    pub fn marked_text(&self) -> Option<&str> {
        self.marked.as_deref()
    }

    fn cursor_cell(&self, element: Bounds<Pixels>) -> Bounds<Pixels> {
        let content = &self.last_content;
        let dims = content.terminal_bounds;
        let line = content.cursor.point.line + content.display_offset as i32;
        let origin = point(dims.cell_width * content.cursor.point.column as f32, dims.line_height * line as f32);
        Bounds::new(element.origin + origin, size(dims.cell_width, dims.line_height))
    }
}

impl EntityInputHandler for Terminal {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let marked = self.marked.as_deref()?;
        let units: Vec<u16> = marked.encode_utf16().collect();
        let range = range.start.min(units.len())..range.end.min(units.len());
        *adjusted = Some(range.clone());
        String::from_utf16(&units[range]).ok()
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let end = self.marked.as_deref().map_or(0, utf16_len);
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|m| 0..utf16_len(m))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    fn paste(&mut self, item: gpui::ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            Terminal::paste(self, &text);
            cx.notify();
        }
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if !text.is_empty() {
            self.input(text.into_bytes());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(&mut self, range: Range<usize>, element: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let mut cell = self.cursor_cell(element);
        cell.origin.x += self.last_content.terminal_bounds.cell_width * range.start as f32;
        Some(cell)
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests;

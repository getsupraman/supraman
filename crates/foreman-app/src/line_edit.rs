//! A one-line text field (m5/26): the Add workspace name. Takes the window's `KeyDown`s while it has the
//! focus, as the task box does ([`crate::compose`]): characters by their `key_char` at the caret, Left /
//! Right / Home / End move it (with Shift they select), Backspace / Delete, Ctrl+A selects all; a click
//! places the caret ([`LineEdit::place`]). The caret blinks as the terminal's does and is lit again by
//! every key. A character the field does not take is reported, never dropped silently.

use std::time::{Duration, Instant};

use gpui::Keystroke;

/// Windows Terminal's blink interval, the terminal tab's too.
pub const BLINK: Duration = Duration::from_millis(530);

pub struct LineEdit {
    pub text: String,
    /// Byte offsets into `text`, on char boundaries; the selection runs from `anchor` to `cursor`.
    cursor: usize,
    anchor: usize,
    /// The last key or click: the caret is lit for one interval after it.
    moved_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKey {
    Enter,
    Escape,
    Edited,
    Moved,
    /// A character the field does not take.
    Refused(char),
    Ignored,
}

impl LineEdit {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.len(), anchor: text.len(), moved_at: Instant::now() }
    }

    /// The whole text replaced, the caret at its end.
    pub fn set(&mut self, text: &str) {
        *self = Self::new(text);
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected byte range, empty when nothing is selected.
    pub fn selection(&self) -> std::ops::Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    /// Whether the caret is painted at `now`: lit for one interval after a key or click, then blinking.
    pub fn caret_lit(&self, now: Instant) -> bool {
        (now.saturating_duration_since(self.moved_at).as_millis() / BLINK.as_millis()) % 2 == 0
    }

    /// How long until the caret next changes, from `now`.
    pub fn next_blink(&self, now: Instant) -> Duration {
        let ms = BLINK.as_millis() as u64;
        let elapsed = now.saturating_duration_since(self.moved_at).as_millis() as u64;
        Duration::from_millis(ms - elapsed % ms)
    }

    /// The caret at byte `at` (clamped to a char boundary); with `extend` the selection grows to it.
    pub fn place(&mut self, at: usize, extend: bool) {
        let mut at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        self.cursor = at;
        if !extend {
            self.anchor = at;
        }
        self.moved_at = Instant::now();
    }

    fn delete_selection(&mut self) -> bool {
        let range = self.selection();
        if range.is_empty() {
            return false;
        }
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.anchor = range.start;
        true
    }

    fn prev(&self) -> usize {
        self.text[..self.cursor].chars().next_back().map_or(0, |c| self.cursor - c.len_utf8())
    }

    fn next(&self) -> usize {
        self.text[self.cursor..].chars().next().map_or(self.text.len(), |c| self.cursor + c.len_utf8())
    }

    /// One keystroke. `accept` maps a typed character to what the field stores, or refuses it.
    pub fn key(&mut self, k: &Keystroke, accept: impl Fn(char) -> Option<char>) -> LineKey {
        let m = &k.modifiers;
        self.moved_at = Instant::now();
        match k.key.as_str() {
            "enter" => return LineKey::Enter,
            "escape" => return LineKey::Escape,
            "a" if m.control && !m.alt => {
                self.anchor = 0;
                self.cursor = self.text.len();
                LineKey::Moved
            }
            "left" | "right" | "home" | "end" => {
                let range = self.selection();
                let at = match k.key.as_str() {
                    "left" if !m.shift && !range.is_empty() => range.start,
                    "right" if !m.shift && !range.is_empty() => range.end,
                    "left" => self.prev(),
                    "right" => self.next(),
                    "home" => 0,
                    _ => self.text.len(),
                };
                self.cursor = at;
                if !m.shift {
                    self.anchor = at;
                }
                LineKey::Moved
            }
            "backspace" | "delete" => {
                if !self.delete_selection() {
                    let (from, to) = if k.key == "backspace" { (self.prev(), self.cursor) } else { (self.cursor, self.next()) };
                    if from == to {
                        return LineKey::Ignored;
                    }
                    self.text.replace_range(from..to, "");
                    self.cursor = from;
                    self.anchor = from;
                }
                LineKey::Edited
            }
            _ if m.control || m.alt || m.platform => LineKey::Ignored,
            _ => {
                let Some(typed) = k.key_char.as_deref().filter(|s| !s.is_empty()) else {
                    return LineKey::Ignored;
                };
                let mut taken = String::new();
                for c in typed.chars() {
                    match accept(c) {
                        Some(c) => taken.push(c),
                        None => return LineKey::Refused(c),
                    }
                }
                self.delete_selection();
                self.text.insert_str(self.cursor, &taken);
                self.cursor += taken.len();
                self.anchor = self.cursor;
                LineKey::Edited
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    fn key(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke { modifiers, key: key.into(), key_char: key_char.map(Into::into) }
    }

    fn plain(key_: &str) -> Keystroke {
        key(key_, None, Modifiers::default())
    }

    fn shift(key_: &str) -> Keystroke {
        key(key_, None, Modifiers { shift: true, ..Default::default() })
    }

    fn lower(c: char) -> Option<char> {
        c.is_ascii_lowercase().then_some(c)
    }

    fn typed(e: &mut LineEdit, text: &str) -> Vec<LineKey> {
        text.chars().map(|c| e.key(&key(&c.to_string(), Some(&c.to_string()), Modifiers::default()), lower)).collect()
    }

    #[test]
    fn edits_at_the_caret_and_moves_it() {
        let mut e = LineEdit::new("acd");
        assert_eq!(e.cursor(), 3);
        e.key(&plain("left"), lower);
        e.key(&plain("left"), lower);
        typed(&mut e, "b");
        assert_eq!((e.text.as_str(), e.cursor()), ("abcd", 2));
        e.key(&plain("delete"), lower);
        assert_eq!(e.text, "abd");
        e.key(&plain("home"), lower);
        e.key(&plain("backspace"), lower);
        assert_eq!((e.text.as_str(), e.cursor()), ("abd", 0));
        e.key(&plain("end"), lower);
        e.key(&plain("backspace"), lower);
        assert_eq!((e.text.as_str(), e.cursor()), ("ab", 2));
    }

    #[test]
    fn shift_selects_and_typing_replaces_the_selection() {
        let mut e = LineEdit::new("abcd");
        e.key(&shift("left"), lower);
        e.key(&shift("left"), lower);
        assert_eq!(e.selection(), 2..4);
        typed(&mut e, "x");
        assert_eq!((e.text.as_str(), e.selection()), ("abx", 3..3));
        e.key(&key("a", Some("a"), Modifiers { control: true, ..Default::default() }), lower);
        assert_eq!(e.selection(), 0..3);
        e.key(&plain("backspace"), lower);
        assert_eq!(e.text, "");
    }

    #[test]
    fn a_click_places_the_caret_on_a_char_boundary() {
        let mut e = LineEdit::new("aé");
        e.place(2, false);
        assert_eq!(e.cursor(), 1);
        e.place(99, true);
        assert_eq!((e.cursor(), e.selection()), (3, 1..3));
    }

    #[test]
    fn a_refused_character_is_reported_not_dropped() {
        let mut e = LineEdit::new("ab");
        assert_eq!(typed(&mut e, "Ä"), [LineKey::Refused('Ä')]);
        assert_eq!(e.text, "ab");
    }

    #[test]
    fn the_caret_is_lit_after_a_key_then_blinks() {
        let mut e = LineEdit::new("ab");
        let t = Instant::now();
        e.moved_at = t;
        assert!(e.caret_lit(t));
        assert!(!e.caret_lit(t + BLINK + Duration::from_millis(1)));
        assert!(e.caret_lit(t + BLINK * 2 + Duration::from_millis(1)));
        assert_eq!(e.next_blink(t + Duration::from_millis(30)), BLINK - Duration::from_millis(30));
    }
}

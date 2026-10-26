//! The task box: a minimal multi-line text input for what the human sends a workspace session
//! (a new task, or the answer to an `APPROVE?`/`DECIDE:`). gpui has no stock text area in our deps, so
//! the box takes the window's `KeyDown`s while it has the focus ([`Compose::key`]): characters by their
//! `key_char`, Enter a new line, Ctrl+Enter sends, Escape cancels. No IME composition, no selection.

use gpui::Keystroke;

pub struct Compose {
    pub workspace: String,
    /// The outbox file this answers; `None`: a new task.
    pub reply_to: Option<String>,
    pub text: String,
    /// Byte offset into `text`, always on a char boundary.
    cursor: usize,
    /// Keys go to the box, not to the active terminal tab.
    pub focused: bool,
    /// Why the last send was refused or failed.
    pub error: Option<String>,
    /// The first line of the last task sent from this box.
    pub sent: Option<String>,
    /// How it went: `sent direct` or `sent via relay (<why>)`.
    pub route: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    Send,
    Cancel,
    Edited,
    Ignored,
}

impl Compose {
    pub fn new(workspace: &str, reply_to: Option<String>) -> Self {
        Self {
            workspace: workspace.to_string(),
            reply_to,
            text: String::new(),
            cursor: 0,
            focused: true,
            error: None,
            sent: None,
            route: None,
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// One keystroke; `paste` is asked for the clipboard's text on Ctrl+V only.
    pub fn key(&mut self, k: &Keystroke, paste: impl FnOnce() -> Option<String>) -> KeyOutcome {
        let m = &k.modifiers;
        // AltGr arrives as Ctrl+Alt with the character it types
        let ctrl = m.control && !m.alt;
        match k.key.as_str() {
            "escape" => return KeyOutcome::Cancel,
            "enter" if ctrl => return KeyOutcome::Send,
            "enter" => self.insert("\n"),
            "v" if ctrl => match paste() {
                Some(text) => self.insert(&text),
                None => return KeyOutcome::Ignored,
            },
            "backspace" => match self.text[..self.cursor].chars().next_back() {
                Some(c) => {
                    self.cursor -= c.len_utf8();
                    self.text.remove(self.cursor);
                }
                None => return KeyOutcome::Ignored,
            },
            "delete" if self.cursor < self.text.len() => {
                self.text.remove(self.cursor);
            }
            "left" => match self.text[..self.cursor].chars().next_back() {
                Some(c) => self.cursor -= c.len_utf8(),
                None => return KeyOutcome::Ignored,
            },
            "right" => match self.text[self.cursor..].chars().next() {
                Some(c) => self.cursor += c.len_utf8(),
                None => return KeyOutcome::Ignored,
            },
            "home" => self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1),
            "end" => self.cursor += self.text[self.cursor..].find('\n').unwrap_or(self.text.len() - self.cursor),
            _ => match &k.key_char {
                Some(c) if !ctrl && !m.platform => self.insert(c),
                _ => return KeyOutcome::Ignored,
            },
        }
        KeyOutcome::Edited
    }
}

/// What a row shows of an answer, and what `answered.json` keeps: its first non-blank line, at most
/// 80 characters.
pub fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    match line.char_indices().nth(80) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    fn key(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke { modifiers, key: key.into(), key_char: key_char.map(String::from) }
    }

    fn typed(c: &mut Compose, text: &str) {
        for ch in text.chars() {
            let s = ch.to_string();
            let name = if ch == ' ' { "space".to_string() } else { s.clone() };
            assert_eq!(c.key(&key(&name, Some(&s), Modifiers::default()), || None), KeyOutcome::Edited);
        }
    }

    #[test]
    fn types_multiple_lines_and_sends_on_ctrl_enter_only() {
        let mut c = Compose::new("demo", None);
        typed(&mut c, "fix the build");
        assert_eq!(c.key(&key("enter", None, Modifiers::default()), || None), KeyOutcome::Edited);
        typed(&mut c, "then ü€");
        assert_eq!(c.text, "fix the build\nthen ü€");
        assert_eq!(c.key(&key("enter", None, Modifiers::control()), || None), KeyOutcome::Send);
        assert_eq!(c.key(&key("escape", None, Modifiers::default()), || None), KeyOutcome::Cancel);
        assert_eq!(c.text, "fix the build\nthen ü€", "neither changes the text");
        // Ctrl+letter is not text; AltGr (Ctrl+Alt) is
        assert_eq!(c.key(&key("a", Some("a"), Modifiers::control()), || None), KeyOutcome::Ignored);
        let altgr = Modifiers { control: true, alt: true, ..Default::default() };
        assert_eq!(c.key(&key("q", Some("@"), altgr), || None), KeyOutcome::Edited);
        assert!(c.text.ends_with("€@"));
    }

    #[test]
    fn edits_at_the_cursor() {
        let mut c = Compose::new("demo", None);
        typed(&mut c, "ab\ncd");
        let none = Modifiers::default();
        c.key(&key("home", None, none), || None);
        assert_eq!(c.cursor(), 3);
        typed(&mut c, "X");
        assert_eq!(c.text, "ab\nXcd");
        c.key(&key("left", None, none), || None);
        c.key(&key("left", None, none), || None);
        assert_eq!(c.key(&key("backspace", None, none), || None), KeyOutcome::Edited);
        assert_eq!(c.text, "a\nXcd");
        c.key(&key("delete", None, none), || None);
        assert_eq!(c.text, "aXcd");
        c.key(&key("end", None, none), || None);
        assert_eq!(c.cursor(), c.text.len());
        assert_eq!(c.key(&key("right", None, none), || None), KeyOutcome::Ignored);
        assert_eq!(c.key(&key("v", None, Modifiers::control()), || Some("1\r\n2".into())), KeyOutcome::Edited);
        assert_eq!(c.text, "aXcd1\n2");
        c.clear();
        assert_eq!((c.text.as_str(), c.cursor()), ("", 0));
        assert_eq!(c.key(&key("backspace", None, none), || None), KeyOutcome::Ignored);
    }

    #[test]
    fn first_line_is_the_first_non_blank_one_cut_at_80() {
        assert_eq!(first_line("\n  use postgres  \nbecause"), "use postgres");
        assert_eq!(first_line(""), "");
        let long = "x".repeat(100);
        assert_eq!(first_line(&long), format!("{}…", "x".repeat(80)));
    }
}

// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

#[cfg(test)]
use crate::Event;
use crate::events::InternalEvent;
use crate::mappings::keys::to_esc_str;
use crate::process::{PtyResources, TerminalType};
use crate::scroll::Scroll;
use crate::{Modes, Terminal};
use alacritty_terminal::grid::Dimensions as _;
#[cfg(test)]
use gpui::Context;
use gpui::{App, Keystroke};
use itertools::Itertools as _;
use std::borrow::Cow;
use std::time::Instant;
#[cfg(test)]
use vte::ansi::{Processor, StdSyncHandler};

#[cfg(test)]
pub(crate) fn convert_lf_to_crlf(bytes: &[u8], previous_byte_was_cr: &mut bool) -> Vec<u8> {
    let mut converted = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte == b'\n' && !*previous_byte_was_cr {
            converted.push(b'\r');
        }
        converted.push(byte);
        *previous_byte_was_cr = byte == b'\r';
    }
    converted
}

/// Copied text as Windows Terminal puts it on the clipboard: each line without its trailing blanks.
pub(crate) fn trim_line_ends(text: &str) -> String {
    text.split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .join("\n")
}

impl Terminal {
    #[cfg(test)]
    pub(crate) fn write_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let mut previous_byte_was_cr = false;
        let converted = convert_lf_to_crlf(bytes, &mut previous_byte_was_cr);
        self.write_raw_output(&converted, cx);
    }

    #[cfg(test)]
    pub(crate) fn write_raw_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let mut term = self.term.lock();
        self.output_processor
            .get_or_insert_with(Processor::<StdSyncHandler>::new)
            .advance(&mut *term, bytes);
        drop(term);
        cx.emit(Event::Wakeup);
    }

    pub fn copy(&mut self, keep_selection: Option<bool>) {
        self.events.push_back(InternalEvent::Copy(keep_selection));
    }

    pub(crate) fn write_to_pty(&self, input: impl Into<Cow<'static, [u8]>>) {
        let input = input.into();
        #[cfg(test)]
        self.pty_write_log.borrow_mut().push(input.to_vec());
        if let TerminalType::Pty {
            resources: PtyResources::Active(pty_tx),
            ..
        } = &self.terminal_type
        {
            if log::log_enabled!(log::Level::Debug) {
                if let Ok(str) = str::from_utf8(&input) {
                    log::debug!("Writing to PTY: {:?}", str);
                } else {
                    log::debug!("Writing to PTY: {:?}", input);
                }
            }
            pty_tx.notify(input);
        }
    }

    pub fn input(&mut self, input: impl Into<Cow<'static, [u8]>>) {
        self.keyboard_input_sent = true;
        self.cursor_moved_at = Instant::now();
        self.write_input(input);
    }

    pub(crate) fn write_input(&mut self, input: impl Into<Cow<'static, [u8]>>) {
        let input = input.into();
        if !self.is_remote_terminal && input.contains(&b'\r') {
            let term = self.term.lock_unfair();
            self.pending_cwd_boundary = Some(Self::scrollback_position(
                term.grid().cursor.point.line.0,
                term.history_size(),
            ));
        }
        if input.contains(&b'\r')
            && let TerminalType::Pty { info, .. } = &self.terminal_type
        {
            info.refresh_soon();
        }

        self.events.push_back(InternalEvent::Scroll(Scroll::Bottom));
        self.events.push_back(InternalEvent::SetSelection(None));
        #[cfg(test)]
        self.input_log.push(input.to_vec());

        self.write_to_pty(input);
    }

    #[cfg(test)]
    pub(crate) fn take_input_log(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.input_log)
    }

    #[cfg(test)]
    pub(crate) fn take_pty_write_log(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(self.pty_write_log.get_mut())
    }

    pub fn try_keystroke(&mut self, keystroke: &Keystroke, option_as_meta: bool) -> bool {
        let esc = to_esc_str(keystroke, self.last_content.mode, option_as_meta);
        if let Some(esc) = esc {
            match esc {
                Cow::Borrowed(string) => self.input(string.as_bytes()),
                Cow::Owned(string) => self.input(string.into_bytes()),
            };
            true
        } else {
            false
        }
    }

    /// Bracketed when the program enabled bracketed paste; otherwise line breaks go as returns.
    pub fn paste(&mut self, text: &str) {
        let paste_text = if self.last_content.mode.contains(Modes::BRACKETED_PASTE) {
            format!("{}{}{}", "\x1b[200~", text.replace('\x1b', ""), "\x1b[201~")
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };

        self.input(paste_text.into_bytes());
    }

    /// Pastes the clipboard's text; false when it holds none (an image), and the caller sends ^V instead.
    pub fn paste_clipboard(&mut self, cx: &App) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return false;
        };
        self.paste(&text);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alacritty::make_content;
    use crate::mouse::Selection;
    use crate::terminal_settings::{AlternateScroll, CursorShape as SettingsCursorShape};
    use crate::test_support::*;
    use crate::{Content, Point, Range, TerminalBuilder, alacritty};
    use gpui::{ClipboardItem, TestAppContext};

    use gpui::AppContext as _;
    use gpui::VisualContext as _;
    use util::paths::PathStyle;
    use vte::ansi::{Color, NamedColor};

    #[test]
    fn test_convert_lf_to_crlf_preserves_split_crlf() {
        let mut previous_byte_was_cr = false;
        assert_eq!(
            convert_lf_to_crlf(b"one\n", &mut previous_byte_was_cr),
            b"one\r\n"
        );
        assert!(!previous_byte_was_cr);

        let mut previous_byte_was_cr = false;
        assert_eq!(
            convert_lf_to_crlf(b"two\r", &mut previous_byte_was_cr),
            b"two\r"
        );
        assert!(previous_byte_was_cr);
        assert_eq!(
            convert_lf_to_crlf(b"\nthree", &mut previous_byte_was_cr),
            b"\nthree"
        );
        assert!(!previous_byte_was_cr);
    }

    #[gpui::test]
    async fn test_write_output_converts_lf_to_crlf(cx: &mut TestAppContext) {
        let terminal = cx.new(|cx| {
            TerminalBuilder::new_display_only(
                SettingsCursorShape::default(),
                AlternateScroll::On,
                None,
                cx.background_executor(),
                PathStyle::local(),
            )
            .subscribe(cx)
        });

        terminal.update(cx, |terminal, cx| {
            terminal.write_output(b"line1\nline2\n", cx);
        });

        let content = terminal.update(cx, |terminal, _cx| {
            let term = terminal.term.lock_unfair();
            make_content(&term, &terminal.last_content)
        });

        // LF converted to CRLF starts every line at column 0, not in a staircase
        let cells = &content.cells;
        let mut line1_col0 = false;
        let mut line2_col0 = false;

        for cell in cells {
            if cell.character() == 'l' && cell.point.column == 0 {
                if cell.point.line == 0 && !line1_col0 {
                    line1_col0 = true;
                } else if cell.point.line == 1 && !line2_col0 {
                    line2_col0 = true;
                }
            }
        }

        assert!(line1_col0, "First line should start at column 0");
        assert!(line2_col0, "Second line should start at column 0");
    }

    #[gpui::test]
    async fn test_write_output_preserves_existing_crlf(cx: &mut TestAppContext) {
        let terminal = cx.new(|cx| {
            TerminalBuilder::new_display_only(
                SettingsCursorShape::default(),
                AlternateScroll::On,
                None,
                cx.background_executor(),
                PathStyle::local(),
            )
            .subscribe(cx)
        });

        terminal.update(cx, |terminal, cx| {
            terminal.write_output(b"line1\r\nline2\r\n", cx);
        });

        let content = terminal.update(cx, |terminal, _cx| {
            let term = terminal.term.lock_unfair();
            make_content(&term, &terminal.last_content)
        });

        let cells = &content.cells;

        let mut found_lines_at_column_0 = 0;
        for cell in cells {
            if cell.character() == 'l' && cell.point.column == 0 {
                found_lines_at_column_0 += 1;
            }
        }

        assert!(
            found_lines_at_column_0 >= 2,
            "Both lines should start at column 0"
        );
    }

    #[gpui::test]
    async fn test_write_output_preserves_bare_cr(cx: &mut TestAppContext) {
        let terminal = cx.new(|cx| {
            TerminalBuilder::new_display_only(
                SettingsCursorShape::default(),
                AlternateScroll::On,
                None,
                cx.background_executor(),
                PathStyle::local(),
            )
            .subscribe(cx)
        });

        terminal.update(cx, |terminal, cx| {
            terminal.write_output(b"hello\rworld", cx);
        });

        let content = terminal.update(cx, |terminal, _cx| {
            let term = terminal.term.lock_unfair();
            make_content(&term, &terminal.last_content)
        });

        let cells = &content.cells;

        let mut text = String::new();
        for cell in cells.iter().take(5) {
            if cell.point.line == 0 {
                text.push(cell.character());
            }
        }

        assert!(
            text.starts_with("world"),
            "Bare CR should allow overwriting: got '{}'",
            text
        );
    }

    #[gpui::test]
    async fn test_write_raw_output_preserves_line_control_semantics(cx: &mut TestAppContext) {
        let bytes = b"a\nb\rc\r\nd";
        let raw = raw_content(&[bytes], cx);

        let normalized_terminal = display_only_terminal(cx);
        normalized_terminal.update(cx, |terminal, cx| terminal.write_output(bytes, cx));
        let normalized = normalized_terminal.read_with(cx, |terminal, _| {
            let term = terminal.term.lock_unfair();
            make_content(&term, &terminal.last_content)
        });

        let raw_cells = raw
            .cells
            .iter()
            .map(|cell| (cell.point.line, cell.point.column, cell.character()))
            .collect::<Vec<_>>();
        let normalized_cells = normalized
            .cells
            .iter()
            .map(|cell| (cell.point.line, cell.point.column, cell.character()))
            .collect::<Vec<_>>();

        assert!(raw_cells.contains(&(0, 0, 'a')));
        assert!(raw_cells.contains(&(1, 0, 'c')));
        assert!(raw_cells.contains(&(1, 1, 'b')));
        assert!(raw_cells.contains(&(2, 0, 'd')));
        assert!(!normalized_cells.contains(&(1, 1, 'b')));
        assert!(normalized_cells.contains(&(1, 0, 'c')));
        assert!(normalized_cells.contains(&(2, 0, 'd')));
        assert_eq!(raw.cursor.point, Point::new(2, 1));
        assert_eq!(normalized.cursor.point, Point::new(2, 1));
    }

    #[gpui::test]
    async fn test_write_raw_output_is_independent_of_chunk_boundaries(cx: &mut TestAppContext) {
        let bytes = b"A\nB\rC\r\n\x1b[31mR\xc3\xa9\x1b[0m\x1b[2;6HX";
        let whole = raw_content(&[bytes], cx);
        let cells = |content: &Content| {
            content
                .cells
                .iter()
                .map(|cell| (cell.point, cell.cell.clone()))
                .collect::<Vec<_>>()
        };
        let whole_cells = cells(&whole);
        let whole_cursor = whole.cursor.point;

        let red_r = whole
            .cells
            .iter()
            .find(|cell| cell.character() == 'R')
            .expect("raw output should contain the styled R");
        assert_eq!(red_r.point, Point::new(2, 0));
        assert_eq!(red_r.foreground(), Color::Named(NamedColor::Red));
        let accented = whole
            .cells
            .iter()
            .find(|cell| cell.character() == 'é')
            .expect("raw output should contain the complete UTF-8 character");
        assert_eq!(accented.point, Point::new(2, 1));
        assert_eq!(accented.foreground(), Color::Named(NamedColor::Red));
        let positioned = whole
            .cells
            .iter()
            .find(|cell| cell.point == Point::new(1, 5))
            .expect("cursor movement should position X");
        assert_eq!(positioned.character(), 'X');
        assert_eq!(
            positioned.foreground(),
            Color::Named(NamedColor::Foreground)
        );
        assert_eq!(
            positioned.background(),
            Color::Named(NamedColor::Background)
        );
        assert_eq!(whole_cursor, Point::new(1, 6));

        for split in 0..=bytes.len() {
            let (first, second) = bytes.split_at(split);
            let split_content = raw_content(&[b"", first, b"", second, b""], cx);
            assert_eq!(
                cells(&split_content),
                whole_cells,
                "two-part split at byte {split}"
            );
            assert_eq!(
                split_content.cursor.point, whole_cursor,
                "two-part cursor at byte {split}"
            );
        }

        let byte_chunks = bytes
            .iter()
            .map(std::slice::from_ref)
            .chain(std::iter::once(&[][..]))
            .collect::<Vec<_>>();
        let bytewise = raw_content(&byte_chunks, cx);
        assert_eq!(cells(&bytewise), whole_cells);
        assert_eq!(bytewise.cursor.point, whole_cursor);
    }

    #[gpui::test]
    async fn test_paste_clipboard_brackets_or_turns_newlines_into_returns(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"");
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("a\r\nb\n".to_string())));
        terminal.update(cx, |terminal, cx| {
            assert!(terminal.paste_clipboard(cx));
            assert_eq!(terminal.take_input_log(), [b"a\rb\r".to_vec()]);
        });

        let terminal = init_terminal_test(cx, b"\x1b[?2004h");
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("x\x1b[201~y".to_string())));
        terminal.update(cx, |terminal, cx| {
            assert!(terminal.paste_clipboard(cx));
            assert_eq!(
                terminal.take_input_log(),
                [b"\x1b[200~x[201~y\x1b[201~".to_vec()]
            );
        });
    }

    #[gpui::test]
    async fn test_a_pasted_name_goes_without_a_return_and_leaves_the_clipboard(cx: &mut TestAppContext) {
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("kept".to_string())));
        let terminal = init_terminal_test(cx, b"");
        terminal.update(cx, |terminal, _| {
            terminal.paste("twin [d923d7]");
            assert_eq!(terminal.take_input_log(), [b"twin [d923d7]".to_vec()]);
        });
        let terminal = init_terminal_test(cx, b"\x1b[?2004h");
        terminal.update(cx, |terminal, _| {
            terminal.paste("twin [d923d7]");
            assert_eq!(terminal.take_input_log(), [b"\x1b[200~twin [d923d7]\x1b[201~".to_vec()]);
        });
        let kept = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
        assert_eq!(kept.as_deref(), Some("kept"));
    }

    #[gpui::test]
    async fn test_paste_clipboard_leaves_an_image_to_the_caller(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"");
        let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, vec![0x89, b'P', b'N', b'G']);
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_image(&image)));
        terminal.update(cx, |terminal, cx| {
            assert!(!terminal.paste_clipboard(cx));
            assert!(terminal.take_input_log().is_empty());
        });
    }

    #[gpui::test]
    async fn test_copy_trims_trailing_blanks(cx: &mut TestAppContext) {
        let (terminal, cx) = init_terminal_test_with_window(cx, b"ab   \r\ncd\t ");
        cx.run_until_parked();
        cx.update_window_entity(&terminal, |terminal, window, cx| {
            alacritty::set_selection(
                &mut terminal.term.lock(),
                Some(&Selection::simple_range(Range::new(
                    Point::new(0, 0),
                    Point::new(1, 5),
                ))),
            );
            terminal.copy(None);
            terminal.sync(window, cx);
        });
        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(copied.as_deref(), Some("ab\ncd"));
    }

    #[test]
    fn copied_lines_lose_trailing_blanks_and_keep_line_breaks() {
        assert_eq!(
            trim_line_ends("PS C:\\>   \nwheel line 1\t \n\n  x  "),
            "PS C:\\>\nwheel line 1\n\n  x"
        );
    }
}

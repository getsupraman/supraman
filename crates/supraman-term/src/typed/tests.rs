use gpui::{EntityInputHandler as _, KeyDownEvent, Keystroke, Modifiers, TestAppContext};

use crate::test_support::init_terminal_test_with_window;

fn key(key: &str, key_char: Option<&str>, modifiers: Modifiers, prefer_character_input: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke { modifiers, key: key.into(), key_char: key_char.map(Into::into) },
        is_held: false,
        prefer_character_input,
    }
}

fn ctrl() -> Modifiers {
    Modifiers { control: true, ..Default::default() }
}

#[gpui::test]
fn committed_text_goes_to_the_pty_as_utf8(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, window, cx| {
        t.take_pty_write_log();
        t.replace_text_in_range(None, "é", window, cx);
        assert_eq!(t.take_pty_write_log(), [vec![0xC3, 0xA9]]);
    });
}

#[gpui::test]
fn marked_text_is_shown_unsent_and_its_commit_sends_once(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, window, cx| {
        t.take_pty_write_log();
        t.replace_and_mark_text_in_range(None, "に", None, window, cx);
        t.replace_and_mark_text_in_range(None, "日本", None, window, cx);
        assert_eq!(t.marked_text(), Some("日本"));
        assert_eq!(t.marked_text_range(window, cx), Some(0..2));
        assert!(t.take_pty_write_log().is_empty(), "a composition is not sent");
        t.replace_text_in_range(None, "日本", window, cx);
        assert_eq!(t.marked_text(), None);
        assert_eq!(t.take_pty_write_log(), ["日本".as_bytes().to_vec()]);
        t.unmark_text(window, cx);
        assert!(t.take_pty_write_log().is_empty(), "unmarking sends nothing");
    });
}

#[gpui::test]
fn a_cancelled_composition_sends_nothing(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, window, cx| {
        t.take_pty_write_log();
        t.replace_and_mark_text_in_range(None, "ka", None, window, cx);
        t.unmark_text(window, cx);
        assert_eq!(t.marked_text(), None);
        assert!(t.take_pty_write_log().is_empty());
    });
}

#[gpui::test]
fn a_dead_key_writes_nothing_and_its_composed_character_once(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, window, cx| {
        t.take_pty_write_log();
        assert!(!t.key_down(&key("`", Some("`"), Modifiers::default(), true), true), "left for the character");
        assert!(t.take_pty_write_log().is_empty(), "the bare accent is not sent");
        assert!(!t.key_down(&key("e", Some("é"), Modifiers::default(), false), true));
        t.replace_text_in_range(None, "é", window, cx);
        assert_eq!(t.take_pty_write_log(), ["é".as_bytes().to_vec()], "the character arrives once, as text");
    });
}

#[gpui::test]
fn an_altgr_character_waits_for_its_text(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, _, _| {
        t.take_pty_write_log();
        let altgr = Modifiers { control: true, alt: true, ..Default::default() };
        assert!(!t.key_down(&key("q", Some("@"), altgr, true), true));
        assert!(t.take_pty_write_log().is_empty());
    });
}

#[gpui::test]
fn without_the_handler_a_typed_character_is_written_from_the_key(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, _, _| {
        t.take_pty_write_log();
        assert!(t.key_down(&key("a", Some("a"), Modifiers::default(), false), false));
        assert_eq!(t.take_pty_write_log(), [b"a".to_vec()]);
        assert!(!t.key_down(&key("a", Some("a"), Modifiers::default(), false), true), "with it, left for WM_CHAR");
        assert!(t.take_pty_write_log().is_empty());
    });
}

#[gpui::test]
fn mapped_keys_are_still_sent_from_the_key(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, _, _| {
        t.take_pty_write_log();
        for (event, sent) in [
            (key("c", None, ctrl(), false), b"\x03".to_vec()),
            (key("enter", None, Modifiers::default(), false), b"\r".to_vec()),
            (key("left", None, Modifiers { shift: true, ..Default::default() }, false), b"\x1b[1;2D".to_vec()),
            (key("b", Some("b"), Modifiers { alt: true, ..Default::default() }, false), b"\x1bb".to_vec()),
        ] {
            assert!(t.key_down(&event, true), "{:?}", event.keystroke);
            assert_eq!(t.take_pty_write_log(), [sent], "{:?}", event.keystroke);
        }
    });
}

#[gpui::test]
fn a_key_with_no_text_and_no_mapping_is_taken_and_sends_nothing(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"");
    terminal.update_in(cx, |t, _, _| {
        t.take_pty_write_log();
        assert!(t.key_down(&key("1", None, ctrl(), false), true));
        assert!(t.take_pty_write_log().is_empty());
    });
}

#[gpui::test]
fn the_ime_window_opens_at_the_cursor_cell(cx: &mut TestAppContext) {
    let (terminal, cx) = init_terminal_test_with_window(cx, b"ab");
    terminal.update_in(cx, |t, window, cx| {
        t.last_content.cursor.point = crate::Point::new(1, 2);
        t.last_content.terminal_bounds = crate::TerminalBounds::new(gpui::px(20.), gpui::px(10.), gpui::Bounds::default());
        let element = gpui::Bounds::new(gpui::point(gpui::px(100.), gpui::px(50.)), gpui::size(gpui::px(400.), gpui::px(400.)));
        let cell = t.bounds_for_range(0..0, element, window, cx).expect("bounds");
        assert_eq!(cell, gpui::Bounds::new(gpui::point(gpui::px(120.), gpui::px(70.)), gpui::size(gpui::px(10.), gpui::px(20.))));
    });
}

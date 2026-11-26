//! Every key × modifier set pinned to what Windows Terminal sends (xterm's PC-style function keys with
//! modifyCursorKeys and modifyFunctionKeys: `CSI 1;m X`, `CSI n;m ~`), keystrokes shaped as gpui_windows makes them.

use gpui::{Keystroke, Modifiers};

use super::to_esc_str;
use crate::Modes;

const NONE: u8 = 0;
const SHIFT: u8 = 1;
const ALT: u8 = 2;
const CTRL: u8 = 4;

fn keystroke(key: &str, mods: u8) -> Keystroke {
    let modifiers = Modifiers { shift: mods & SHIFT != 0, alt: mods & ALT != 0, control: mods & CTRL != 0, ..Default::default() };
    Keystroke { modifiers, key: key.into(), key_char: None }
}

fn sent(key: &str, mods: u8, mode: Modes) -> Option<String> {
    to_esc_str(&keystroke(key, mods), mode, false).map(|s| s.into_owned())
}

/// The modifier sets that put a parameter on a navigation or function key, with xterm's code for each.
const MODIFIED: [(u8, u8); 7] = [(SHIFT, 2), (ALT, 3), (SHIFT | ALT, 4), (CTRL, 5), (SHIFT | CTRL, 6), (ALT | CTRL, 7), (SHIFT | ALT | CTRL, 8)];

#[test]
fn cursor_keys_are_csi_or_ss3_bare_and_csi_1_m_modified() {
    for (key, end) in [("up", 'A'), ("down", 'B'), ("right", 'C'), ("left", 'D'), ("home", 'H'), ("end", 'F')] {
        assert_eq!(sent(key, NONE, Modes::NONE), Some(format!("\x1b[{end}")), "{key}");
        assert_eq!(sent(key, NONE, Modes::APP_CURSOR), Some(format!("\x1bO{end}")), "{key} in DECCKM");
        for (mods, m) in MODIFIED {
            for mode in [Modes::NONE, Modes::APP_CURSOR] {
                assert_eq!(sent(key, mods, mode), Some(format!("\x1b[1;{m}{end}")), "{key} with modifiers {mods} in {mode:?}");
            }
        }
    }
}

#[test]
fn editing_keys_are_csi_n_tilde_with_the_modifier_as_second_parameter() {
    for (key, n) in [("insert", 2), ("delete", 3), ("pageup", 5), ("pagedown", 6)] {
        for mode in [Modes::NONE, Modes::APP_CURSOR] {
            assert_eq!(sent(key, NONE, mode), Some(format!("\x1b[{n}~")), "{key} in {mode:?}");
            for (mods, m) in MODIFIED {
                assert_eq!(sent(key, mods, mode), Some(format!("\x1b[{n};{m}~")), "{key} with modifiers {mods}");
            }
        }
    }
}

#[test]
fn function_keys_are_ss3_for_f1_to_f4_and_csi_n_tilde_after() {
    for (i, end) in ['P', 'Q', 'R', 'S'].into_iter().enumerate() {
        let key = format!("f{}", i + 1);
        assert_eq!(sent(&key, NONE, Modes::NONE), Some(format!("\x1bO{end}")), "{key}");
        for (mods, m) in MODIFIED {
            assert_eq!(sent(&key, mods, Modes::NONE), Some(format!("\x1b[1;{m}{end}")), "{key} with modifiers {mods}");
        }
    }
    for (key, n) in [("f5", 15), ("f6", 17), ("f7", 18), ("f8", 19), ("f9", 20), ("f10", 21), ("f11", 23), ("f12", 24)] {
        assert_eq!(sent(key, NONE, Modes::NONE), Some(format!("\x1b[{n}~")), "{key}");
        for (mods, m) in MODIFIED {
            assert_eq!(sent(key, mods, Modes::NONE), Some(format!("\x1b[{n};{m}~")), "{key} with modifiers {mods}");
        }
    }
}

#[test]
fn tab_enter_backspace_escape_and_space() {
    let cases: [(&str, u8, &str); 13] = [
        ("tab", NONE, "\t"),
        ("tab", SHIFT, "\x1b[Z"),
        ("enter", NONE, "\r"),
        ("enter", ALT, "\x1b\r"),
        ("backspace", NONE, "\x7f"),
        ("backspace", SHIFT, "\x7f"),
        ("backspace", CTRL, "\x08"),
        ("backspace", ALT, "\x1b\x7f"),
        ("escape", NONE, "\x1b"),
        ("space", CTRL, "\x00"),
        ("[", CTRL, "\x1b"),
        ("\\", CTRL, "\x1c"),
        ("]", CTRL, "\x1d"),
    ];
    for (key, mods, bytes) in cases {
        for mode in [Modes::NONE, Modes::APP_CURSOR | Modes::APP_KEYPAD] {
            assert_eq!(sent(key, mods, mode).as_deref(), Some(bytes), "{key} with modifiers {mods} in {mode:?}");
        }
    }
}

#[test]
fn ctrl_letters_are_c0_with_or_without_shift_and_alt_prefixes_esc() {
    for (i, letter) in ('a'..='z').enumerate() {
        let key = letter.to_string();
        let c0 = char::from(i as u8 + 1).to_string();
        assert_eq!(sent(&key, CTRL, Modes::NONE), Some(c0.clone()), "Ctrl+{letter}");
        assert_eq!(sent(&key, CTRL | SHIFT, Modes::NONE), Some(c0.clone()), "Ctrl+Shift+{letter}");
        assert_eq!(sent(&key, ALT, Modes::NONE), Some(format!("\x1b{letter}")), "Alt+{letter}");
        assert_eq!(sent(&key, ALT | SHIFT, Modes::NONE), Some(format!("\x1b{}", letter.to_ascii_uppercase())), "Alt+Shift+{letter}");
        assert_eq!(sent(&key, ALT | CTRL, Modes::NONE), Some(format!("\x1b{c0}")), "Ctrl+Alt+{letter}");
    }
}

#[test]
fn a_plain_or_shifted_character_is_text_not_a_sequence() {
    for (key, mods) in [("a", NONE), ("a", SHIFT), ("1", NONE), ("space", NONE), ("space", SHIFT), ("/", NONE)] {
        assert_eq!(sent(key, mods, Modes::NONE), None, "{key} with modifiers {mods}");
    }
}

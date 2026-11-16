// Vendored from Zed crates/terminal/src/terminal_settings.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later); `AlternateScroll` from crates/settings_content/src/terminal.rs.

use std::time::Duration;

use gpui::{App, ClipboardItem, Global, Hsla, black, rgb};
use serde::{Deserialize, Serialize};

/// Windows Terminal's default scheme, Campbell (its defaults.json), as ANSI 0-15: black, red, green,
/// yellow, blue, purple, cyan, white, then the bright variants.
pub const CAMPBELL: [u32; 16] = [
    0x0C0C0C, 0xC50F1F, 0x13A10E, 0xC19C00, 0x0037DA, 0x881798, 0x3A96DD, 0xCCCCCC, //
    0x767676, 0xE74856, 0x16C60C, 0xF9F1A5, 0x3B78FF, 0xB4009E, 0x61D6D6, 0xF2F2F2,
];

pub fn campbell() -> [Hsla; 16] {
    CAMPBELL.map(|hex| rgb(hex).into())
}

/// Windows Terminal's default `wordDelimiters`: a double-click selects up to these.
pub(crate) const WORD_DELIMITERS: &str = " /\\()\"'-.,:;<>~!@#$%^&*|+=[]{}~?\u{2502}";

/// Zed's default `terminal.path_hyperlink_regexes` (assets/settings/default.json), tried in order: Python's
/// `File "…", line N`, then a path with an optional `:line:column` or `(line,column)` suffix.
pub const PATH_HYPERLINK_REGEXES: [&str; 2] = [
    r#"File "(?<path>[^"]+)", line (?<line>[0-9]+)"#,
    r#"(?x)
(?<path>
    (
        # multi-char path: first char (not opening delimiter, space, or box drawing char)
        [^({\[<"'`\ \u2500-\u257F]
        # middle chars: non-space, and colon/paren only if not followed by digit/paren/space
        ([^\ :(]|[:(][^0-9()\ ])*
        # last char: not closing delimiter or colon
        [^()}\]>"'`.,;:\ ]
    |
        # single-char path: not delimiter, punctuation, space, or box drawing char
        [^(){}\[\]<>"'`.,;:\ \u2500-\u257F]
    )
    # optional line/column suffix (included in path for PathWithPosition::parse_str)
    (:+[0-9]+(:[0-9]+)?|:?\([0-9]+([,:]?[0-9]+)?\))?
)"#,
];

/// Zed's default `terminal.path_hyperlink_timeout_ms`.
pub const PATH_HYPERLINK_TIMEOUT: Duration = Duration::from_millis(1);

/// Windows Terminal's caret blink: Windows' default `GetCaretBlinkTime`.
pub const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlternateScroll {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
    Hollow,
}

/// Colours the emulator answers OSC colour queries with (`get_color_at_index`).
#[derive(Clone, Debug)]
pub struct TerminalPalette {
    /// ANSI 0-15: black, red, green, yellow, blue, magenta, cyan, white, then the bright variants.
    pub ansi: [Hsla; 16],
    /// Dim black..white.
    pub dim: [Hsla; 8],
    pub foreground: Hsla,
    pub background: Hsla,
    pub bright_foreground: Hsla,
    pub cursor: Hsla,
}

impl Default for TerminalPalette {
    fn default() -> Self {
        Self {
            ansi: [black(); 16],
            dim: [black(); 8],
            foreground: black(),
            background: black(),
            bright_foreground: black(),
            cursor: black(),
        }
    }
}

/// What the terminal reads as a gpui global; absent, the defaults below apply.
#[derive(Clone, Debug)]
pub struct TerminalConfig {
    pub copy_on_select: bool,
    pub keep_selection_on_copy: bool,
    pub open_links_in_mouse_mode: bool,
    /// Exported to the child as `TERM_PROGRAM_VERSION`.
    pub program_version: String,
    pub palette: TerminalPalette,
    pub clipboard: ClipboardRoute,
}

/// The clipboard a terminal copies to and pastes from, its owner's (`supraman_ui::clipboard::terminal`); without one it has none.
#[derive(Clone, Copy)]
pub struct ClipboardRoute {
    pub read: fn(&App) -> Option<ClipboardItem>,
    pub write: fn(&App, ClipboardItem),
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub read_primary: fn(&App) -> Option<ClipboardItem>,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub write_primary: fn(&App, ClipboardItem),
}

#[cfg(test)]
thread_local! {
    static TEST_CLIPBOARD: std::cell::RefCell<Option<ClipboardItem>> = const { std::cell::RefCell::new(None) };
}

impl Default for ClipboardRoute {
    #[cfg(not(test))]
    fn default() -> Self {
        Self {
            read: |_| None,
            write: |_, _| {},
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            read_primary: |_| None,
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            write_primary: |_, _| {},
        }
    }

    /// This crate's tests have no owner: one clipboard per test thread.
    #[cfg(test)]
    fn default() -> Self {
        Self {
            read: |_| TEST_CLIPBOARD.with(|c| c.borrow().clone()),
            write: |_, item| TEST_CLIPBOARD.with(|c| *c.borrow_mut() = Some(item)),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            read_primary: |_| None,
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            write_primary: |_, _| {},
        }
    }
}

impl std::fmt::Debug for ClipboardRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClipboardRoute")
    }
}

pub(crate) fn read_clipboard(cx: &App) -> Option<ClipboardItem> {
    (TerminalConfig::get(cx).clipboard.read)(cx)
}

pub(crate) fn write_clipboard(cx: &App, item: ClipboardItem) {
    (TerminalConfig::get(cx).clipboard.write)(cx, item)
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            copy_on_select: false,
            keep_selection_on_copy: true,
            open_links_in_mouse_mode: true,
            program_version: env!("CARGO_PKG_VERSION").to_string(),
            palette: TerminalPalette::default(),
            clipboard: ClipboardRoute::default(),
        }
    }
}

impl Global for TerminalConfig {}

impl TerminalConfig {
    pub fn get(cx: &App) -> TerminalConfig {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }
}

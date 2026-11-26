//! What every Supraman gpui binary shares: the widgets, themes, terminal painter, init and the selftests' probes.

pub mod banner;
pub mod blink;
mod builtin_glyphs;
pub mod button;
pub mod checkbox;
pub mod clipboard;
pub mod clipped;
pub mod confirm;
pub mod details;
pub mod dialog;
pub mod dnd;
pub mod focus;
pub mod focus_order;
pub mod form;
#[cfg(any(test, feature = "gallery"))]
pub mod gallery;
pub mod folder_picker;
pub mod fonts;
pub mod hit;
pub mod hover_card;
pub mod icon;
pub mod inbox_card;
pub mod ink;
pub mod keyed_list;
pub mod label;
pub mod list;
pub mod marks;
pub mod matcher;
pub mod menu;
pub mod notices;
pub mod palette;
pub mod placeholder;
pub mod popup;
#[cfg(test)]
mod ratchet;
pub mod scroll;
pub mod segmented;
pub mod session;
pub mod settings_page;
pub mod split;
pub mod state;
pub mod table;
pub mod tabs;
pub mod term_element;
pub mod text_input;
pub mod themes;
pub mod tokens;
pub mod toast;
pub mod tooltip;
pub mod tree;

use supraman_term::terminal_settings::TerminalConfig;
use gpui::App;

pub use themes::palette;

/// The theme globals with [`themes::DEFAULT_THEME`] and the terminal config; `main` then applies the user's ([`themes::apply`]).
pub fn init_globals(cx: &mut App, program_version: &str) {
    theme::init(theme::LoadThemes::JustBase, cx);
    cx.set_global(TerminalConfig {
        program_version: program_version.to_string(),
        clipboard: clipboard::terminal(),
        ..Default::default()
    });
    themes::apply(cx, None, themes::TerminalColors::Theme, None);
    table::bind_keys(cx);
    focus_order::bind_keys(cx);
    focus::bind_keys(cx);
    dnd::init(cx);
}

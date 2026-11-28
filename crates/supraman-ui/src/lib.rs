//! What every Supraman gpui binary shares: the kit (UI-KIT §9), its themes, the terminal painter and their init.

mod builtin_glyphs;
pub mod clipboard;
pub mod clipped;
pub mod controls;
pub mod dnd;
pub mod dock;
pub mod focus;
pub mod focus_order;
#[cfg(any(test, feature = "gallery"))]
pub mod gallery;
pub mod folder_picker;
pub mod fonts;
pub mod hit;
pub mod icon;
pub mod ink;
pub mod label;
pub mod list;
pub mod marks;
pub mod matcher;
pub mod measure;
pub mod patterns;
pub mod popup;
#[cfg(test)]
mod ratchet;
pub mod scroll;
pub mod split;
pub mod state;
pub mod surfaces;
pub mod tabs;
pub mod text;
pub mod term_element;
pub mod themes;
pub mod tokens;

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
    list::table::bind_keys(cx);
    focus_order::bind_keys(cx);
    focus::bind_keys(cx);
    dnd::init(cx);
}

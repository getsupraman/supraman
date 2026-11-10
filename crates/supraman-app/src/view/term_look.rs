use std::sync::Arc;

use gpui::{Context, Pixels, SharedString, px};
use supraman_core::TerminalSettings;

use super::{FleetView, shells, term};

/// How every terminal tab looks and starts: its font, ligatures, zoom, cursor blink and default shell.
pub struct TermLook {
    pub term_font_size: Pixels,
    pub term_font_family: SharedString,
    pub term_ligatures: bool,
    pub installed_fonts: Arc<Vec<String>>,
    pub(super) font_save: Option<gpui::Task<()>>,
    pub(super) zoom_carried: f32,
    pub(super) blink: term::CursorBlink,
    pub(crate) shell_default: shells::ShellDefault,
}

impl TermLook {
    pub(super) fn new(terminal: &TerminalSettings, cx: &mut Context<FleetView>) -> Self {
        let installed_fonts: Arc<Vec<String>> = Arc::new(cx.text_system().all_font_names());
        Self {
            term_font_size: px(terminal.font_size() as f32),
            term_font_family: SharedString::from(terminal.font_family(&installed_fonts)),
            term_ligatures: terminal.ligatures(),
            installed_fonts,
            font_save: None,
            zoom_carried: 0.,
            blink: term::CursorBlink::start(cx),
            shell_default: shells::ShellDefault::new(terminal.clone()),
        }
    }
}

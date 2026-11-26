use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, Pixels, SharedString, Subscription, px};
use supraman_ui::text::blink::Blink;
use supraman_core::TerminalSettings;

use super::{FleetView, shells};

/// How every terminal tab looks and starts: its font, ligatures, zoom, cursor blink and default shell.
pub struct TermLook {
    pub term_font_size: Pixels,
    pub term_font_family: SharedString,
    pub term_ligatures: bool,
    pub installed_fonts: Arc<Vec<String>>,
    pub(super) font_save: Option<gpui::Task<()>>,
    pub(super) zoom_carried: f32,
    /// The terminal tab's line cursor's blink, run while a shown terminal holds the keys in an active window.
    pub(super) blink: Entity<Blink>,
    _blink_repaint: Subscription,
    pub(crate) shell_default: shells::ShellDefault,
}

impl TermLook {
    pub(super) fn new(terminal: &TerminalSettings, cx: &mut Context<FleetView>) -> Self {
        let installed_fonts: Arc<Vec<String>> = Arc::new(cx.text_system().all_font_names());
        let blink = cx.new(Blink::new);
        Self {
            term_font_size: px(terminal.font_size() as f32),
            term_font_family: SharedString::from(terminal.font_family(&installed_fonts)),
            term_ligatures: terminal.ligatures(),
            installed_fonts,
            font_save: None,
            zoom_carried: 0.,
            blink: blink.clone(),
            _blink_repaint: cx.observe(&blink, |_, _, cx| cx.notify()),
            shell_default: shells::ShellDefault::new(terminal.clone()),
        }
    }
}

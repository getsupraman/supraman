use std::path::PathBuf;

use super::menu::{Confirm, ContextMenu};

/// What floats over the window: the right-click menu, a question (in its dialog) and the folder picker's pick.
#[derive(Default)]
pub struct Dialogs {
    pub menu: Option<ContextMenu>,
    pub confirm: Option<Confirm>,
    pub dialog: supraman_ui::dialog::Dialog,
    pub pick_folder: Option<PathBuf>,
}

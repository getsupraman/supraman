use std::path::PathBuf;

use super::folder_edits::RemoveQuestion;
use super::menu::{Confirm, ContextMenu};

/// What floats over the window: the right-click menu, a question (in its dialog) and the folder picker's pick.
#[derive(Default)]
pub struct Dialogs {
    pub menu: Option<ContextMenu>,
    pub confirm: Option<Confirm>,
    /// Remove Folder's or Remove Group's question while it would leave schedules without a row.
    pub remove: Option<RemoveQuestion>,
    pub dialog: supraman_ui::dialog::Dialog,
    pub pick_folder: Option<PathBuf>,
}

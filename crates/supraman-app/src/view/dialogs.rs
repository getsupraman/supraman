use std::path::PathBuf;

use gpui::Entity;

use super::menu::{Confirm, ContextMenu};
use super::workspace_dialog::WorkspaceDialog;

/// What floats over the window: the right-click menu, a question, the workspace dialog and the folder picker's pick.
#[derive(Default)]
pub struct Dialogs {
    pub menu: Option<ContextMenu>,
    pub confirm: Option<Confirm>,
    pub adding: Option<(Entity<WorkspaceDialog>, gpui::Subscription)>,
    pub pick_folder: Option<PathBuf>,
}

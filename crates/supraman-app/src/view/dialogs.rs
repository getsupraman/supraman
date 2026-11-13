use std::path::PathBuf;

use gpui::{Entity, Subscription};

use super::folder_edits::GroupName;
use super::menu::{Confirm, ContextMenu};

/// What floats over the window: the right-click menu, a question, Add Group's name, and the folder picker's pick.
#[derive(Default)]
pub struct Dialogs {
    pub menu: Option<ContextMenu>,
    pub confirm: Option<Confirm>,
    pub group_name: Option<(Entity<GroupName>, Subscription)>,
    pub pick_folder: Option<PathBuf>,
}

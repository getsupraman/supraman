use futures::channel::mpsc::UnboundedSender;

use crate::saved_tabs::SavedTabs;

/// The saved tabs' restore at start and their writes after each change.
pub struct TabSaving {
    pub(super) tabs_resolving: bool,
    pub(super) tabs_again: bool,
    pub tabs_restored: bool,
    pub(super) tabs_reopening: bool,
    pub(super) tabs_frozen: bool,
    pub(super) tabs_saved: Option<SavedTabs>,
    pub(super) tabs_writes: UnboundedSender<SavedTabs>,
    pub tab_notices: Vec<String>,
}

impl TabSaving {
    pub fn new(tabs_writes: UnboundedSender<SavedTabs>) -> Self {
        Self {
            tabs_resolving: false,
            tabs_again: false,
            tabs_restored: false,
            tabs_reopening: false,
            tabs_frozen: false,
            tabs_saved: None,
            tabs_writes,
            tab_notices: Vec::new(),
        }
    }
}

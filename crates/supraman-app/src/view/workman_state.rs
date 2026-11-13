use std::collections::HashSet;

use supraman_ui::nudge::Nudge;

/// Workman's sign-in around its status: the notice dismissed, the recheck, a sign-in under way and the servers synced or left.
pub struct WorkmanState {
    pub(super) workman_dismissed: Option<String>,
    pub(super) workman_recheck: Nudge,
    pub signing_in: Option<String>,
    pub sign_in_error: Option<String>,
    pub(super) synced_server: Option<String>,
    pub(super) left_servers: HashSet<String>,
}

impl WorkmanState {
    pub fn new(workman_recheck: Nudge) -> Self {
        Self {
            workman_dismissed: None,
            workman_recheck,
            signing_in: None,
            sign_in_error: None,
            synced_server: None,
            left_servers: HashSet::new(),
        }
    }
}

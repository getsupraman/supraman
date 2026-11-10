use std::sync::{Arc, Mutex};

use crate::hand::Hands;

/// The window's [+] hands and the quit that releases their tokens.
pub struct Lifecycle {
    pub(super) hands: Option<Arc<Mutex<Hands>>>,
    /// The last run's hand tokens are released, so a reopened tab's mint is not revoked with them.
    pub(super) hands_ready: bool,
    pub(super) hands_released: bool,
    pub(super) quit_release: Option<gpui::Task<()>>,
    pub(super) quitting: bool,
    pub(super) quit_asked: bool,
}

impl Lifecycle {
    pub fn new(hands: Option<Arc<Mutex<Hands>>>) -> Self {
        Self {
            hands_ready: hands.is_none(),
            hands,
            hands_released: false,
            quit_release: None,
            quitting: false,
            quit_asked: false,
        }
    }
}

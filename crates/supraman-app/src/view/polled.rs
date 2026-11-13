use std::collections::HashMap;

use supraman_core::DEFAULT_MAX_WORKING;

use crate::fleet::HookSignal;

/// The poll's side of the window: the upkeep's last report and the hooks' signals.
pub struct Polled {
    pub max_working: usize,
    pub upkeep_errors: Vec<String>,
    pub(super) upkeep_dismissed: Vec<String>,
    pub daemon_hold: Option<String>,
    pub hooks: HashMap<String, HookSignal>,
    pub(super) last_alive: HashMap<String, i64>,
}

impl Polled {
    pub(super) fn new() -> Self {
        Self {
            max_working: DEFAULT_MAX_WORKING,
            upkeep_errors: Vec::new(),
            upkeep_dismissed: Vec::new(),
            daemon_hold: None,
            hooks: HashMap::new(),
            last_alive: HashMap::new(),
        }
    }
}

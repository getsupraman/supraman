use std::collections::{BTreeMap, HashMap};

use futures::channel::mpsc::UnboundedSender;
use supraman_core::DEFAULT_MAX_WORKING;
use supraman_supervisor::NeedsYou;

use super::DesiredWrite;
use crate::fleet::HookSignal;

/// The supervisor's side of the window: the wanted sessions and their writes, its last report and the hooks' signals.
pub struct Supervising {
    pub needs: Vec<NeedsYou>,
    pub desired: BTreeMap<String, (bool, Vec<String>)>,
    pub max_working: usize,
    pub supervise_errors: Vec<String>,
    pub(super) supervise_dismissed: Vec<String>,
    pub daemon_hold: Option<String>,
    pub last_attempt: HashMap<String, i64>,
    pub hooks: HashMap<String, HookSignal>,
    pub(super) last_alive: HashMap<String, i64>,
    pub(super) desired_writes: UnboundedSender<DesiredWrite>,
}

impl Supervising {
    pub(super) fn new(desired_writes: UnboundedSender<DesiredWrite>) -> Self {
        Self {
            needs: Vec::new(),
            desired: BTreeMap::new(),
            max_working: DEFAULT_MAX_WORKING,
            supervise_errors: Vec::new(),
            supervise_dismissed: Vec::new(),
            daemon_hold: None,
            last_attempt: HashMap::new(),
            hooks: HashMap::new(),
            last_alive: HashMap::new(),
            desired_writes,
        }
    }
}

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use supraman_core::secret::{MemoryStore, SecretStore, default_store};

use super::schedules::{ScheduleEdit, SharedRegistrar};
use crate::dismissed_messages::DismissedMessages;
use crate::schedules::ScheduleRow;

/// The Schedules group's state: each workspace's schedules, their errors, the Task Scheduler scan and the Edit dialog.
pub struct ScheduleState {
    pub schedules: BTreeMap<String, Vec<ScheduleRow>>,
    pub schedule_errors: Vec<String>,
    pub(super) schedule_errors_dismissed: Vec<String>,
    /// A failed `sync` by `<ws>/<id>` of the schedule acted on: on its row and in the Inbox until a sync succeeds.
    pub sync_errors: BTreeMap<String, String>,
    pub registrar: SharedRegistrar,
    /// Task Scheduler's tasks that run claude outside Supraman, as the last scan found them; read only.
    pub other_schedules: Option<supraman_core::schedule::Foreign>,
    pub other_scan_error: Option<String>,
    pub(crate) other_scans: u64,
    pub schedule_store: Arc<dyn SecretStore>,
    pub schedule_edit: Option<ScheduleEdit>,
    pub runs_dismissed: DismissedMessages,
    pub waiting_dismissed: DismissedMessages,
}

impl ScheduleState {
    pub fn new(home: Option<&Path>) -> Self {
        Self {
            schedules: BTreeMap::new(),
            schedule_errors: Vec::new(),
            schedule_errors_dismissed: Vec::new(),
            sync_errors: BTreeMap::new(),
            registrar: Arc::new(supraman_core::platform::tasks::WindowsRegistrar),
            other_schedules: None,
            other_scan_error: None,
            other_scans: 0,
            schedule_store: match home {
                Some(h) => default_store(h),
                None => Arc::new(MemoryStore::default()),
            },
            schedule_edit: None,
            runs_dismissed: DismissedMessages::default(),
            waiting_dismissed: DismissedMessages::default(),
        }
    }
}

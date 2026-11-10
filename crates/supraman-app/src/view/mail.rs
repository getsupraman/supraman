use std::collections::BTreeMap;

use supraman_core::OutboxMessage;

use crate::compose::Compose;
use crate::dismissed_messages::DismissedMessages;
use crate::supervise::RelayStatus;

/// The relays and their outbox messages: what each said, what was answered and dismissed, and the reply being written.
#[derive(Default)]
pub struct Mail {
    pub relays: BTreeMap<String, RelayStatus>,
    pub messages: BTreeMap<String, Vec<OutboxMessage>>,
    pub dismissed_messages: DismissedMessages,
    pub answered: BTreeMap<String, BTreeMap<String, String>>,
    pub answered_route: BTreeMap<String, String>,
    pub compose: Option<Compose>,
}

mod dotenv;
mod error;
mod events;
mod migrate;
mod model;
mod plugin;
mod registry;
mod relay;
mod scan;
mod settings;
mod slug;

pub use dotenv::read_env_value;
pub use error::{CoreError, Result};
pub use events::{
    Event, EventReader, ROTATE_AT, append as append_event, events_dir, is_safe_id, log_path, role_injection, role_injection_for, role_tag,
};
pub use migrate::migrate_home;
pub use model::{Repo, Workspace};
pub use plugin::{HOOK_EXE, install_hook_exe};
pub use registry::{Registry, home_from};
pub use relay::{
    ANSWERED_FILE, Ask, AskState, InboxWatcher, OutboxMessage, READY_FILE, REJECTED_DIR, RelayDrop, SENT_DIR, ask_kind, ask_states,
    clear_ready, ensure_relay_dirs, inbox_dir, mark_answered, read_answered,
    outbox_dir, read_outbox, relay_dir, relay_ready, send,
};
pub use scan::{DEFAULT_DEPTH, scan};
pub use settings::{DEFAULT_MAX_WORKING, DEFAULT_WORKSTATE_URL, Settings};
pub use slug::{slugify, validate_name};

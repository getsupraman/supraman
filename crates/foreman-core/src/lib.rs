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

pub use error::{CoreError, Result};
pub use events::{
    Event, EventReader, ROTATE_AT, append as append_event, events_dir, is_safe_id, log_path, role_injection, role_tag,
};
pub use migrate::migrate_home;
pub use model::{Repo, Workspace};
pub use plugin::{HOOK_EXE, install_hook_exe};
pub use registry::{Registry, home_from};
pub use relay::{
    InboxWatcher, OutboxMessage, READY_FILE, REJECTED_DIR, RelayDrop, SENT_DIR, clear_ready, ensure_relay_dirs, inbox_dir,
    outbox_dir, read_outbox, relay_dir, relay_ready, send,
};
pub use scan::{DEFAULT_DEPTH, scan};
pub use settings::{DEFAULT_MAX_WORKING, Settings};
pub use slug::{slugify, validate_name};

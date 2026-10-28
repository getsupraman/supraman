mod discover;
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

pub use discover::{discover, rescan, same_root};
pub use error::{CoreError, Result};
pub use events::{
    Event, EventReader, ROTATE_AT, append as append_event, events_dir, is_safe_id, log_path, role_injection, role_injection_for, role_tag,
};
pub use migrate::migrate_home;
pub use model::{Repo, Workspace};
pub use plugin::{HOOK_EXE, find_plugin_dir, install_hook_exe};
pub use registry::{Registry, home_from};
pub use relay::{
    ANSWERED_FILE, Ask, AskState, DEFAULT_RELAY_IDLE_MS, InboxWatcher, LAST_DROP_FILE, OutboxMessage, READY_FILE, REJECTED_DIR, RelayDrop,
    SENT_DIR, ask_kind, ask_states, clear_ready, ensure_relay_dirs, inbox_dir, last_drop, mark_answered, pending_drops, read_answered,
    outbox_dir, read_outbox, relay_demand, relay_dir, relay_ready, send, tell,
};
pub use scan::{DEFAULT_DEPTH, branch_at, scan};
pub use settings::{
    DEFAULT_MAX_WORKING, DEFAULT_TERMINAL_FONT_SIZE, DEFAULT_WORKSTATE_URL, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE, Settings, url_origin, validate_workstate_url,
    clamp_terminal_font_size,
};
pub use slug::{slugify, validate_name};

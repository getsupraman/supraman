mod discover;
mod edit;
mod error;
mod events;
pub mod desktop;
pub mod fs;
pub mod install;
pub mod home;
mod model;
pub mod paths;
mod plugin;
pub mod prune;
pub mod process;
mod registry;
mod relay;
mod scan;
pub mod schedule;
pub mod secret;
mod settings;
mod slug;
pub mod time;
pub mod update;
pub mod url;

pub use discover::{discover, rescan, same_path};
pub use edit::{Edit, Edited};
pub use error::{CoreError, Result};
pub use events::{
    Event, EventReader, ROTATE_AT, append as append_event, events_dir, is_safe_id, log_path, role_injection, role_injection_for, role_tag,
};
pub use model::{Folder, FolderKind, Scan, Workspace, holder_of, owners};
pub use plugin::{HOOK_EXE, find_plugin_dir, sm_allowed_tools, install_hook_exe};
pub use home::{HOME_VAR, LEGACY_HOME_VAR, env_home, home_from, home_from_env};
pub use registry::Registry;
pub use relay::{
    ANSWERED_FILE, Ask, AskState, DEFAULT_RELAY_IDLE_MS, InboxWatcher, LAST_DROP_FILE, OutboxMessage, READY_FILE, REJECTED_DIR, RelayDrop,
    SENT_DIR, ask_kind, ask_states, clear_ready, ensure_relay_dirs, inbox_dir, last_drop, mark_answered, pending_drops, read_answered,
    outbox_dir, read_outbox, relay_demand, relay_dir, relay_ready, send, tell,
};
pub use scan::{DEFAULT_DEPTH, branch_at, scan};
pub use settings::{
    AppearanceSettings, DEFAULT_MAX_WORKING, DEFAULT_TERMINAL_FONT_SIZE, DEFAULT_WORKMAN_URL, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE, Section,
    SessionSettings, Settings, TERMINAL_FONT_PREFERENCE, TerminalSettings, UpdateSettings, WindowSettings, WorkmanSettings, clamp_terminal_font_size,
    default_terminal_font_family, validate_workman_url,
};
pub use slug::{FM_PREFIX, slugify, validate_name};
pub use url::origin as url_origin;

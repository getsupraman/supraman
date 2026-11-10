pub mod askpass;
mod discover;
mod edit;
mod error;
mod events;
pub mod desktop;
pub mod fs;
pub mod hex;
pub mod install;
pub mod home;
mod model;
pub mod paths;
mod plugin;
pub mod prune;
pub mod pool;
pub mod process;
mod registry;
mod relay;
mod scan;
pub mod schedule;
pub mod secret;
pub mod shells;
pub mod store;
pub mod text;
mod settings;
mod slug;
pub mod time;
pub mod update;
pub mod url;
pub mod watch;
pub mod workman_server;

pub use discover::{discover, rescan, same_path};
pub use edit::{Edit, Edited};
pub use error::{CoreError, Result};
pub use events::{
    Event, EventReader, ROTATE_AT, append as append_event, events_dir, log_path, role_injection, role_injection_for, role_tag,
};
pub use model::{Folder, FolderKind, Scan, Workspace, holder_of, owners};
pub use plugin::{AGENTS_SERVER, AGENT_TOOLS, HOOK_EXE, find_plugin_dir, sm_allowed_tools, install_hook_exe};
pub use home::{HOME_VAR, env_home, home_from, home_from_env};
pub use registry::Registry;
pub use relay::{
    ANSWERED_FILE, Ask, AskState, DEFAULT_RELAY_IDLE_MS, InboxWatcher, LAST_DROP_FILE, OutboxMessage, READY_FILE, REJECTED_DIR, RelayDrop,
    SENT_DIR, ask_kind, ask_states, clear_ready, ensure_relay_dirs, inbox_dir, last_drop, mark_answered, pending_drops, read_answered,
    outbox_dir, read_outbox, relay_demand, relay_dir, relay_ready, send, tell,
};
pub use scan::{DEFAULT_DEPTH, branch_at, scan};
pub use settings::{
    AppearanceSettings, DEFAULT_MAX_WORKING, DEFAULT_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE, Section,
    SessionSettings, Settings, TERMINAL_FONT_PREFERENCE, TabKeys, TerminalSettings, UpdateSettings, WindowSettings, clamp_terminal_font_size,
    default_terminal_font_family,
};
pub use slug::{FM_PREFIX, is_safe_id, is_session_name, name_char, slugify, validate_name};
pub use workman_server::validate_workman_url;

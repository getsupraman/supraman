// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later). Cut for Foreman: no `settings`/`theme`/`theme_settings`/`task`/
// `release_channel`; `TerminalSettings` -> `TerminalConfig`, task mode removed, colours from
// `TerminalPalette`; tests set a `TerminalConfig` instead of a `SettingsStore`. Code Foreman does
// not reach is deleted, not kept for re-syncing with Zed.

mod mappings;

mod alacritty;
mod bounds;
mod builder;
mod content;
mod cwd;
mod events;
mod input;
mod links;
mod mouse;
mod process;
mod pty_info;
mod scroll;
mod search;
pub mod terminal_settings;
#[cfg(test)]
mod test_support;

pub use bounds::TerminalBounds;
pub use builder::{TerminalBuilder, TerminalMode};
pub use content::{
    Cell, Content, Cursor, CursorShape, GridLinesChange, HoveredWord, Hyperlink, IndexedCell,
    Modes, Point, Range, SelectionRange, blink_phase, is_default_background_color,
};
pub use events::Event;
pub use links::{MaybeNavigationTarget, PathLikeTarget};
pub use mappings::colors::{get_color_at_index, rgba_color};
pub use mouse::MouseInputMode;
pub use search::Search;

use crate::alacritty::{AlacrittyTermConfig, AlacrittyTermLock, HyperlinkMatch, RegexSearches};
use crate::cwd::CwdHistoryEntry;
use crate::events::InternalEvent;
use crate::mouse::{SelectionPhase, SelectionSide};
use crate::process::TerminalType;
use anyhow::Result;
use collections::VecDeque;
use gpui::{BackgroundExecutor, EventEmitter, Hsla, Pixels, Point as GpuiPoint, Task};
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Instant;
use util::paths::PathStyle;
pub use util::shell::Shell;
pub use vte::ansi::{Color, NamedColor, Rgb};
#[cfg(test)]
use vte::ansi::{Processor, StdSyncHandler};

pub struct Terminal {
    terminal_type: TerminalType,
    term: Arc<AlacrittyTermLock>,
    term_config: AlacrittyTermConfig,
    #[cfg(test)]
    output_processor: Option<Processor<StdSyncHandler>>,
    events: VecDeque<InternalEvent>,
    /// This is only used for mouse mode cell change detection
    last_mouse: Option<(Point, SelectionSide)>,
    /// Window-relative position of the most recent left mouse-down. Used to
    /// apply a drag threshold before starting a selection (see #58970).
    mouse_down_position: Option<GpuiPoint<Pixels>>,
    pub matches: Vec<Range>,
    pub last_content: Content,
    pub selection_head: Option<Point>,

    pub breadcrumb_text: String,
    scroll_px: Pixels,
    next_link_id: usize,
    selection_phase: SelectionPhase,
    hyperlink_regex_searches: RegexSearches,
    is_remote_terminal: bool,
    last_mouse_move_time: Instant,
    last_hyperlink_search_position: Option<GpuiPoint<Pixels>>,
    mouse_down_hyperlink: Option<HyperlinkMatch>,
    #[cfg(windows)]
    shell_program: Option<String>,
    child_exited: Option<ExitStatus>,
    keyboard_input_sent: bool,
    /// Input or a cursor move restarts the blink with the cursor lit (Windows Terminal).
    cursor_moved_at: Instant,
    /// Whether the painter last drew the cursor lit; the selftest reads it.
    pub cursor_painted_lit: bool,
    /// The selection's background and its text's colour as last painted, while a selection with text
    /// is shown; the selftest reads it.
    pub selection_painted: Option<(Hsla, Hsla)>,
    event_loop_task: Task<Result<(), anyhow::Error>>,
    background_executor: BackgroundExecutor,
    path_style: PathStyle,
    cwd_history: Vec<CwdHistoryEntry>,
    pending_cwd_boundary: Option<i32>,
    #[cfg(test)]
    input_log: Vec<Vec<u8>>,
    #[cfg(test)]
    suppress_hyperlink_throttle_once: bool,
    #[cfg(test)]
    pty_write_log: std::cell::RefCell<Vec<Vec<u8>>>,
}

impl EventEmitter<Event> for Terminal {}

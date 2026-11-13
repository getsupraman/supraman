//! A session's one page: the Details tab's and the Git tab's agent panel, which leaves out the transcript.

use std::path::Path;

use gpui::{App, Hsla};
use supraman_ui::details::Section;
use theme::ActiveTheme as _;

use super::FleetView;
use super::menu::{MenuAction, MenuItem};
use crate::fleet::{Open, SessionRow, State};
use crate::row_details::{self, OnDisk};

pub struct SessionView {
    pub title: String,
    pub pill: (String, Hsla),
    pub sections: Vec<Section>,
    /// First among its actions.
    pub open: Open,
    /// The session menu's items but its own Attach and Resume, which Open is.
    pub actions: Vec<MenuItem>,
}

/// Where the session is shown: the tab running it, the tab shown now, whether it can be resumed.
#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub tab: Option<u64>,
    pub active: Option<u64>,
    pub resumable: bool,
}

pub(super) fn build(row: &SessionRow, listed: bool, placed: Placed, mut sections: Vec<Section>, items: Vec<MenuItem>, transcript: bool, pill: Hsla) -> SessionView {
    if !transcript {
        sections.retain(|s| s.key != "transcript");
    }
    let open = if listed { Open::of(row, placed.tab, placed.active, placed.resumable) } else { Open::Unlisted };
    let actions = items.into_iter().filter(|i| !matches!(i.action, MenuAction::Attach { .. } | MenuAction::Resume { .. })).collect();
    SessionView { title: row_details::session_title(row), pill: (row_details::session_pill(row, listed), pill), sections, open, actions }
}

impl FleetView {
    /// `listed`: in the last poll's tree, else as last seen, with no actions; `transcript`: whether its section shows.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn session_view(&self, entry: &str, place: &str, row: &SessionRow, folder: Option<(&str, &Path)>, disk: &OnDisk, listed: bool, transcript: bool, cx: &App) -> SessionView {
        let sections = self.row_sections(entry, row, folder, disk, listed, cx);
        let items = if listed { self.session_menu(entry, place, row, None, cx) } else { Vec::new() };
        let placed = Placed { tab: self.session_tab(&row.session), active: self.active, resumable: self.resumable(row) };
        let pill = if listed { State::of_row(row).mark(cx).color() } else { cx.theme().colors().text_muted };
        build(row, listed, placed, sections, items, transcript, pill)
    }
}

#[cfg(test)]
mod tests;

//! A session's one Open: the sidebar row's slot and double-click, every page's Open, the Work tab's and the outbox's.

use gpui::Context;
use supraman_claude::now_ms;
use supraman_core::paths::folder_name;
use supraman_ui::session::Opening;

use super::FleetView;
use crate::fleet::{self, Open, SessionRow};
use crate::tab_card;

impl FleetView {
    pub(super) fn open_of(&self, row: &SessionRow) -> Open {
        Open::of(row, self.session_tab(&row.session), self.active, self.resumable(row))
    }

    /// The one open: its tab, an attach or a resume; a refusal shows why.
    /// `place`: where the row is, its tab's title before the session's name.
    pub(super) fn open_row(&mut self, place: &str, row: &SessionRow, cx: &mut Context<Self>) {
        let s = &row.session;
        let open = self.open_of(row);
        let title = || format!("{place} · {}", fleet::display_name(s));
        match open {
            Open::Tab(id) => self.activate_tab(id, cx),
            Open::Attach => {
                self.attach_titled(title(), s, cx);
            }
            Open::Resume => {
                self.resume_titled(title(), s, cx);
            }
            Open::Elsewhere => {
                let text = format!("{}: {}", fleet::display_name(s), open.hint());
                self.toast.show_why(text, |this: &mut Self| &mut this.toast, cx);
            }
            Open::Shown | Open::Foreign | Open::Unlisted | Open::Nothing => {}
        }
    }

    /// The session of claude's `session_id` in the tree: its entry, the folder key or schedule id it is listed under, the row.
    pub(super) fn listed_place(&self, session_id: &str) -> Option<(String, String, SessionRow)> {
        let tree = self.tree();
        tree.entries.iter().find_map(|node| {
            let row = node.rows().find(|r| r.session.session_id == session_id)?;
            Some((node.name.clone(), super::right::tab_menu::row_folder(node, session_id).to_string(), row.clone()))
        })
    }

    /// [`Self::listed_place`], else the session as `claude agents` lists it, placed by its cwd.
    pub(super) fn session_place(&self, session_id: &str) -> Option<(String, String, SessionRow)> {
        self.listed_place(session_id).or_else(|| {
            let s = self.sessions.iter().find(|s| s.session_id == session_id)?;
            let (group, folder) = tab_card::place(&s.cwd, &self.listed.entries, &self.listed.disk.shown);
            let folder = folder.unwrap_or_else(|| folder_name(&s.cwd));
            let place = group.map_or_else(|| folder.clone(), |g| format!("{g}/{folder}"));
            Some((self.entry_of(&s.cwd).unwrap_or_default(), place, SessionRow::new(s.clone(), s.health(now_ms()))))
        })
    }

    pub fn session_opening(&self, session_id: &str) -> Opening {
        self.session_place(session_id).map_or(Open::Unlisted, |(_, _, row)| self.open_of(&row)).opening()
    }

    /// [`Self::open_row`] of claude's `session_id`.
    pub fn open_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if let Some((_, place, row)) = self.session_place(session_id) {
            self.open_row(&place.replace('/', " · "), &row, cx);
        }
    }
}

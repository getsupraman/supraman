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
    pub(super) fn open_row(&mut self, workspace: &str, repo: &str, row: &SessionRow, cx: &mut Context<Self>) {
        let s = &row.session;
        let open = self.open_of(row);
        let title = || format!("{workspace} · {repo} · {}", fleet::display_name(s));
        match open {
            Open::Tab(id) => self.activate_tab(id, cx),
            Open::Attach => {
                self.attach_titled(title(), s, cx);
            }
            Open::Resume => {
                self.resume_titled(title(), s, cx);
            }
            Open::Elsewhere | Open::Supervised => {
                let text = format!("{}: {}", fleet::display_name(s), open.hint());
                self.toast.show_why(text, |this: &mut Self| &mut this.toast, cx);
            }
            Open::Shown | Open::Foreign | Open::Unlisted | Open::Nothing => {}
        }
    }

    /// The session of claude's `session_id` in the tree: its workspace, the folder it is listed under, the row.
    pub(super) fn listed_place(&self, session_id: &str) -> Option<(String, String, SessionRow)> {
        let tree = self.tree();
        tree.workspaces.iter().find_map(|node| {
            let row = node.rows().find(|r| r.session.session_id == session_id)?;
            Some((node.name.clone(), super::right::tab_menu::row_folder(node, session_id).to_string(), row.clone()))
        })
    }

    /// [`Self::listed_place`], else the session as `claude agents` lists it, placed by its cwd.
    pub(super) fn session_place(&self, session_id: &str) -> Option<(String, String, SessionRow)> {
        self.listed_place(session_id).or_else(|| {
            let s = self.sessions.iter().find(|s| s.session_id == session_id)?;
            let (workspace, repo) = tab_card::place(&s.cwd, &self.workspaces);
            let here = folder_name(&s.cwd);
            Some((workspace.unwrap_or_else(|| here.clone()), repo.unwrap_or(here), SessionRow::new(s.clone(), s.health(now_ms()))))
        })
    }

    pub fn session_opening(&self, session_id: &str) -> Opening {
        self.session_place(session_id).map_or(Open::Unlisted, |(_, _, row)| self.open_of(&row)).opening()
    }

    /// [`Self::open_row`] of claude's `session_id`.
    pub fn open_session(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if let Some((workspace, repo, row)) = self.session_place(session_id) {
            self.open_row(&workspace, &repo, &row, cx);
        }
    }

    /// [`Self::open_session`] of the session `claude attach` knows as `id`.
    pub(super) fn open_by_id(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(sid) = self.sessions.iter().find(|s| s.id == id).map(|s| s.session_id.clone()) {
            self.open_session(&sid, cx);
        }
    }
}

//! A repository's Git tab: one per folder, focused when open.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Context, Entity, Pixels, Point, Window};
use supraman_core::paths::path_key;
use supraman_core::text::capitalised;
use supraman_git_view::occupancy::LiveSession;

use super::menu::MenuTarget;
use super::right::tab_menu::row_folder;
use super::{FleetView, Tab, TabKind};
use crate::fleet::{self, SessionRow};
use crate::git_view::branch::OpenGit;
use crate::git_view::{AgentMenu, AttachAgent, GitView, PageAction, SessionPage, SessionPages};
use crate::row_details::{self, OnDisk};
use crate::tab_card;

/// The open Git tab of `folder`, else a new one from `make`, pushed last; true when it is new.
pub(crate) fn open_or_focus(tabs: &mut Vec<Tab>, next_id: &mut u64, folder: &Path, title: String, cx: &mut App, make: impl FnOnce(&mut App) -> Entity<GitView>) -> (u64, bool) {
    let key = path_key(folder);
    if let Some(tab) = tabs.iter().find(|t| matches!(&t.kind, TabKind::Git(v) if path_key(&v.read(cx).folder) == key)) {
        return (tab.id, false);
    }
    let id = *next_id;
    *next_id += 1;
    let view = make(cx);
    tabs.push(Tab::new(id, title, TabKind::Git(view), Some(folder.to_path_buf())));
    (id, true)
}

impl FleetView {
    pub(super) fn show_git_tabs(&self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            if let TabKind::Git(view) = &tab.kind {
                let shown = self.active == Some(tab.id);
                view.update(cx, |v, cx| v.set_shown(shown, cx));
            }
        }
    }

    /// `Git · <repo>` for `folder`, at its repo's layout.
    pub fn open_git(&mut self, folder: PathBuf, title: String, cx: &mut Context<Self>) -> u64 {
        let (home, hits, live) = (self.home.clone(), self.hits.clone(), self.occupancy.clone());
        let make = |cx: &mut App| GitView::open_following(folder.clone(), home, hits, live, cx);
        let (id, new) = open_or_focus(&mut self.tabs, &mut self.next_tab_id, &folder, title, cx, make);
        if new && let Some(TabKind::Git(view)) = self.tabs.iter().find(|t| t.id == id).map(|t| &t.kind) {
            cx.subscribe(view, |this, _, e: &OpenGit, cx| {
                this.open_git(e.0.clone(), self::title(&e.0), cx);
            })
            .detach();
            cx.subscribe(view, |this, _, e: &AttachAgent, cx| this.attach_agent(e.0.attach.as_deref(), cx)).detach();
            cx.subscribe(view, |this, _, e: &AgentMenu, cx| this.agent_menu(&e.session, e.position, cx)).detach();
            let pages = self.session_pages(cx);
            view.update(cx, |v, cx| v.set_session_pages(pages, cx));
        }
        self.active = Some(id);
        cx.notify();
        id
    }
}

impl FleetView {
    /// A Git tab's agent row's Attach: the session's tab when one runs it, else a new attach tab, as the sidebar's Attach.
    fn attach_agent(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        let Some(session) = id.and_then(|id| self.sessions.iter().find(|s| s.id == id)).cloned() else { return };
        if let Some(tab) = self.session_tab(&session) {
            return self.activate_tab(tab, cx);
        }
        let (workspace, repo) = tab_card::place(&session.cwd, &self.workspaces);
        let title = [workspace, repo, Some(fleet::display_name(&session).to_string())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        self.attach_titled(title, &session, cx);
    }
}

impl FleetView {
    /// A live session's row in the tree: its workspace, the folder it is listed under, the row.
    fn live_row(&self, session: &LiveSession) -> Option<(String, String, SessionRow)> {
        let tree = self.tree();
        tree.workspaces.iter().find_map(|node| {
            let row = node.rows().find(|r| r.session.session_id == session.claude_session_id)?;
            Some((node.name.clone(), row_folder(node, &row.session.session_id).to_string(), row.clone()))
        })
    }

    /// A Git tab's agent or worktree row's right-click: the sidebar's menu of the session.
    fn agent_menu(&mut self, session: &LiveSession, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((workspace, repo, row)) = self.live_row(session) else { return };
        self.open_menu(MenuTarget::Session { workspace, repo, session_id: row.session.session_id }, position, cx);
    }

    /// The agent row's panel: the Details tab's sections, but its transcript, which a disk read off the UI thread finds, and its menu's items.
    fn agent_page(&self, session: &LiveSession, cx: &App) -> Option<SessionPage> {
        let (workspace, repo, row) = self.live_row(session)?;
        let mut sections = self.row_sections(&workspace, &repo, &row, None, &OnDisk::default(), true, cx);
        sections.retain(|s| s.key != "transcript");
        let state = fleet::State::of_row(&row);
        let actions = self
            .session_menu(&workspace, &repo, &row, None, cx)
            .into_iter()
            .map(|i| PageAction { key: i.key.to_string(), label: i.label, dimmed: i.dimmed })
            .collect();
        Some(SessionPage { title: row_details::session_title(&row), pill: (capitalised(state.word()), state.mark(cx).color()), sections, actions })
    }

    fn agent_action(&mut self, session: &LiveSession, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((workspace, repo, row)) = self.live_row(session) else { return };
        if let Some(item) = self.session_menu(&workspace, &repo, &row, None, cx).into_iter().find(|i| i.key == key && i.dimmed.is_none()) {
            self.run_action(item.action, window, cx);
        }
    }

    fn session_pages(&self, cx: &Context<Self>) -> SessionPages {
        let (read, act) = (cx.weak_entity(), cx.weak_entity());
        SessionPages {
            page: Rc::new(move |session, cx| read.upgrade()?.read(cx).agent_page(session, cx)),
            act: Rc::new(move |session, key, window, cx| {
                act.update(cx, |this, cx| this.agent_action(session, key, window, cx)).ok();
            }),
        }
    }
}

#[cfg(test)]
#[path = "git_tab_tests.rs"]
mod tests;

pub(crate) fn title(folder: &std::path::Path) -> String {
    let name = folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into_owned());
    format!("Git · {name}")
}

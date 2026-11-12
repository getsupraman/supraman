//! A repository's Git tab: one per folder, focused when open.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Context, Entity, Pixels, Point, Window};
use supraman_core::paths::path_key;
use supraman_git_view::occupancy::LiveSession;

use super::menu::MenuTarget;
use super::{FleetView, Tab, TabKind};
use crate::fleet::{Open, SessionRow};
use crate::git_view::branch::OpenGit;
use crate::git_view::{AgentMenu, GitView, OpenAgent, PageAction, SessionPage, SessionPages};
use crate::row_details::OnDisk;

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
            cx.subscribe(view, |this, _, e: &AgentMenu, cx| this.agent_menu(&e.session, e.position, cx)).detach();
            cx.subscribe(view, |this, _, e: &OpenAgent, cx| this.open_session(&e.0.claude_session_id, cx)).detach();
            let pages = self.session_pages(cx);
            view.update(cx, |v, cx| v.set_session_pages(pages, cx));
        }
        self.active = Some(id);
        cx.notify();
        id
    }
}

impl FleetView {
    fn live_row(&self, session: &LiveSession) -> Option<(String, String, SessionRow)> {
        self.listed_place(&session.claude_session_id)
    }

    /// A Git tab's agent or worktree row's right-click: the sidebar's menu of the session.
    fn agent_menu(&mut self, session: &LiveSession, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((workspace, repo, row)) = self.live_row(session) else { return };
        self.open_menu(MenuTarget::Session { workspace, repo, session_id: row.session.session_id }, position, cx);
    }

    /// The agent row's panel: the session's one page but its transcript, which a disk read off the UI thread finds.
    fn agent_page(&self, session: &LiveSession, cx: &App) -> Option<SessionPage> {
        let (workspace, repo, row) = self.live_row(session)?;
        let view = self.session_view(&workspace, &repo, &row, None, &OnDisk::default(), true, false, cx);
        let actions = view.actions.into_iter().map(|i| PageAction { key: i.key.to_string(), label: i.label, dimmed: i.dimmed }).collect();
        Some(SessionPage { title: view.title, pill: view.pill, sections: view.sections, open: view.open.opening(), actions })
    }

    fn agent_action(&mut self, session: &LiveSession, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((workspace, repo, row)) = self.live_row(session) else { return };
        if let Some(item) = self.session_menu(&workspace, &repo, &row, None, cx).into_iter().find(|i| i.key == key && i.dimmed.is_none()) {
            self.run_action(item.action, window, cx);
        }
    }

    fn session_pages(&self, cx: &Context<Self>) -> SessionPages {
        let (read, act, opening) = (cx.weak_entity(), cx.weak_entity(), cx.weak_entity());
        SessionPages {
            page: Rc::new(move |session, cx| read.upgrade()?.read(cx).agent_page(session, cx)),
            act: Rc::new(move |session, key, window, cx| {
                act.update(cx, |this, cx| this.agent_action(session, key, window, cx)).ok();
            }),
            opening: Rc::new(move |session, cx| opening.upgrade().map(|f| f.read(cx).session_opening(&session.claude_session_id)).unwrap_or_else(|| Open::Unlisted.opening())),
        }
    }
}

#[cfg(test)]
#[path = "git_tab_tests.rs"]
mod tests;

pub(crate) fn title(folder: &std::path::Path) -> String {
    format!("Git · {}", supraman_core::paths::folder_name(folder))
}

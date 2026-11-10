//! A repository's Git tab: one per folder, focused when open.

use std::path::{Path, PathBuf};

use gpui::{App, Context, Entity};
use supraman_core::paths::path_key;

use super::{FleetView, Tab, TabKind};
use crate::fleet;
use crate::git_view::branch::OpenGit;
use crate::git_view::{AttachAgent, GitSizes, GitView};
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

    /// `Git · <repo>` for `folder`, at its repo's layout; `sizes` a tab saved before the layout file kept them.
    pub fn open_git(&mut self, folder: PathBuf, title: String, sizes: &GitSizes, cx: &mut Context<Self>) -> u64 {
        let (home, hits, live) = (self.home.clone(), self.hits.clone(), self.occupancy.clone());
        let make = |cx: &mut App| GitView::open_following(folder.clone(), home, hits, sizes, live, cx);
        let (id, new) = open_or_focus(&mut self.tabs, &mut self.next_tab_id, &folder, title, cx, make);
        if new && let Some(TabKind::Git(view)) = self.tabs.iter().find(|t| t.id == id).map(|t| &t.kind) {
            cx.subscribe(view, |this, _, e: &OpenGit, cx| {
                this.open_git(e.0.clone(), self::title(&e.0), &GitSizes::default(), cx);
            })
            .detach();
            cx.subscribe(view, |this, _, e: &AttachAgent, cx| this.attach_agent(e.0.attach.as_deref(), cx)).detach();
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

#[cfg(test)]
#[path = "git_tab_tests.rs"]
mod tests;

pub(crate) fn title(folder: &std::path::Path) -> String {
    let name = folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into_owned());
    format!("Git · {name}")
}

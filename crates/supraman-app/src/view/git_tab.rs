//! A repository's Git tab: one per folder, focused when open.

use std::path::{Path, PathBuf};

use gpui::{App, Context, Entity};
use supraman_core::paths::path_key;

use super::{FleetView, Tab, TabKind};
use crate::git_view::{GitSizes, GitView};

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
    /// `Git · <repo>` for `folder`, at its repo's layout; `sizes` a tab saved before the layout file kept them.
    pub fn open_git(&mut self, folder: PathBuf, title: String, sizes: &GitSizes, cx: &mut Context<Self>) -> u64 {
        let (home, hits) = (self.home.clone(), self.hits.clone());
        let make = |cx: &mut App| GitView::open(folder.clone(), home, hits, sizes, cx);
        let (id, _) = open_or_focus(&mut self.tabs, &mut self.next_tab_id, &folder, title, cx, make);
        self.active = Some(id);
        cx.notify();
        id
    }
}

//! A tab's Add items: its folder's Git, Work and Details panels placed in its dock, moved there from wherever they show.

use std::path::Path;

use gpui::{App, Context};
use supraman_core::paths::{folder_name, path_key};
use supraman_ui::controls::menu::item;

use super::{NO_FOLDER, Repo, Work, open_git_item, row_folder, workman_item};
use crate::view::menu::{Added, MenuAction, MenuItem, MenuTarget};
use crate::view::{FleetView, PanelKey, Tab, TabKind};
use crate::work_view::Target;

pub const HERE: &str = "Already in this tab";
pub const FULL: &str = "No room: close a panel first";
const NOT_LISTED: &str = "Not in a listed folder";

/// A tab's dock as its Add items weigh it: the tab, the keys its panels show, whether it takes one more.
pub struct DockFacts {
    pub tab: u64,
    pub holds: Vec<PanelKey>,
    pub room: bool,
}

impl Added {
    pub fn key(&self) -> PanelKey {
        match self {
            Added::Git(top) => PanelKey::Git(path_key(top)),
            Added::Work { target, .. } => PanelKey::Work(target.clone()),
            Added::Details(target) => PanelKey::details(target),
        }
    }
}

fn add(key: &'static str, label: String, dock: &DockFacts, added: Added) -> MenuItem {
    item(key, label, MenuAction::AddPanel { tab: dock.tab, added })
}

/// `item` dimmed with `why`, else with this dock's reason it cannot take `item`'s panel.
fn placed(item: MenuItem, why: Option<String>, dock: &DockFacts) -> MenuItem {
    let here = |a: &MenuAction| matches!(a, MenuAction::AddPanel { added, .. } if dock.holds.contains(&added.key()));
    match why {
        Some(why) => item.dimmed(why),
        None if here(&item.action) => item.dimmed(HERE),
        None if !dock.room => item.dimmed(FULL),
        None => item,
    }
}

pub fn add_git_item(folder: Option<&Path>, repo: &Repo, dock: &DockFacts) -> MenuItem {
    let why = open_git_item(folder, repo).dimmed;
    let (label, top) = match repo {
        Repo::Top(top) => (format!("Add Git {} panel", folder_name(top)), top.clone()),
        Repo::Reading | Repo::Outside => ("Add Git panel".to_string(), folder.map(Path::to_path_buf).unwrap_or_default()),
    };
    placed(add("tab-add-git", label, dock, Added::Git(top)), why, dock)
}

pub fn add_work_item(folder: Option<&Path>, work: &Work, dock: &DockFacts) -> MenuItem {
    let why = workman_item(folder, work).dimmed;
    let added = match work {
        Work::Bound { target, project } => Added::Work { target: target.clone(), project: project.clone() },
        _ => Added::Work { target: Target::Group(String::new()), project: String::new() },
    };
    placed(add("tab-add-work", "Add Work panel".into(), dock, added), why, dock)
}

/// `details`: the tab's row, a session's or its folder's.
pub fn add_details_item(folder: Option<&Path>, details: Option<&MenuTarget>, dock: &DockFacts) -> MenuItem {
    let why = match (folder, details) {
        (_, Some(_)) => None,
        (None, None) => Some(NO_FOLDER.to_string()),
        (Some(_), None) => Some(NOT_LISTED.to_string()),
    };
    let target = details.cloned().unwrap_or(MenuTarget::Group(String::new()));
    placed(add("tab-add-details", "Add Details panel".into(), dock, Added::Details(target)), why, dock)
}

/// The Add items right after Open Workman, else after Open Git.
pub fn with_adds(mut items: Vec<MenuItem>, adds: [MenuItem; 3]) -> Vec<MenuItem> {
    let after = items.iter().position(|i| i.key == "tab-open-workman").or_else(|| items.iter().position(|i| i.key == "tab-open-git")).map_or(0, |i| i + 1);
    items.splice(after..after, adds);
    items
}

impl FleetView {
    pub(in crate::view) fn dock_facts(&self, id: u64, cx: &App) -> DockFacts {
        let tab = self.docks.tab_of(id);
        let holds = self.docks.panels(tab).into_iter().filter_map(|p| self.tab(p)).map(|t| t.key(cx)).collect();
        DockFacts { tab, holds, room: self.docks.has_room(tab) }
    }

    /// The Details a tab adds: the session row it runs, else the listed folder row holding its folder.
    pub(in crate::view) fn tab_details(&self, tab: &Tab, folder: Option<&Path>) -> Option<MenuTarget> {
        let tree = self.tree();
        let session = tree.nodes().find_map(|node| {
            let row = node.rows().find(|r| Self::tab_runs(tab, &r.session))?;
            Some(MenuTarget::Session { entry: node.name.clone(), place: row_folder(node, &row.session.session_id), session_id: row.session.session_id.clone() })
        });
        session.or_else(|| {
            let (_, row) = self.entry_and_folder(folder?)?;
            tree.folders().find(|(_, f)| f.path == row).map(|(_, f)| MenuTarget::Folder(f.key.clone()))
        })
    }

    /// `added`'s panel in `tab`'s dock: moved out of the tab or dock it shows in, else opened, then focused there.
    pub(in crate::view) fn add_to_tab(&mut self, tab: u64, added: Added, cx: &mut Context<Self>) {
        let dock = self.dock_facts(tab, cx);
        let key = added.key();
        if dock.holds.contains(&key) || !dock.room {
            return;
        }
        let panel = match self.find_panel(&key, cx) {
            Some(panel) => {
                self.remove_panel(panel, cx);
                Some(panel)
            }
            None => self.open_added(added, cx),
        };
        if let Some(panel) = panel
            && self.add_panel(dock.tab, panel, cx)
        {
            self.show_panel(panel, cx);
        }
    }

    fn open_added(&mut self, added: Added, cx: &mut Context<Self>) -> Option<u64> {
        match added {
            Added::Git(top) => {
                let title = crate::view::git_tab::title(&top);
                Some(self.open_git(top, title, cx))
            }
            Added::Work { target, project } => {
                let id = self.open_work(target, cx);
                if let Some(Tab { kind: TabKind::Work(view), .. }) = self.tab(id) {
                    view.clone().update(cx, |view, cx| view.show_project(project, cx));
                }
                Some(id)
            }
            Added::Details(target) => self.open_details(target, cx),
        }
    }
}

#[cfg(test)]
mod tests;

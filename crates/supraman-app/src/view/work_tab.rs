//! The Work tab of a folder or a group: one per [`Target`], its folders and the fleet's links handed to it.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::Context;

use super::{FleetView, Tab, TabKind, occupancy};
use crate::fleet::{EntryNode, Open};
use crate::work_view::{self, FleetLink, Target, WorkView};

/// The Work target of an entry's row: a group by its name, a top-level folder by its path.
pub(crate) fn entry_target(entry: &EntryNode) -> Option<Target> {
    match entry.group {
        true => Some(Target::Group(entry.name.clone())),
        false => entry.folders.first().map(|f| Target::Folder(f.path.clone())),
    }
}

impl FleetView {
    /// The folders a target's Work tab binds: a group's listed folders, or the folder.
    fn target_folders(&self, target: &Target) -> Vec<PathBuf> {
        match target {
            Target::Folder(path) => vec![path.clone()],
            Target::Group(name) => self.tree().entry(name).map(|e| e.folders.iter().filter(|f| f.listed()).map(|f| f.path.clone()).collect()).unwrap_or_default(),
        }
    }

    /// Every target a Work tab may be on now: each group, and each folder row.
    pub(super) fn work_targets(&self) -> Vec<Target> {
        let tree = self.tree();
        let groups = tree.entries.iter().filter(|e| e.group).map(|e| Target::Group(e.name.clone()));
        groups.chain(tree.folders().map(|(_, f)| Target::Folder(f.path.clone()))).collect()
    }

    /// The Inbox's targets, each entry's with its listed folders, and every folder row, whose bindings it reads.
    pub(super) fn inbox_targets(&self) -> (Vec<(Target, Vec<PathBuf>)>, Vec<PathBuf>) {
        let tree = self.tree();
        let targets = tree.entries.iter().filter_map(|e| Some((entry_target(e)?, e.folders.iter().filter(|f| f.listed()).map(|f| f.path.clone()).collect())));
        (targets.collect(), tree.folders().map(|(_, f)| f.path.clone()).collect())
    }

    /// Why a folder row's Open Work cannot open its tab now, from the Inbox's binding; `None` when it binds a project.
    pub(super) fn work_why_not(&self, folder: &Path, cx: &gpui::App) -> Option<String> {
        match self.work_inbox.read(cx).folder_binding(folder, cx) {
            None => Some("Reading Workman…".into()),
            Some(Err(why)) => Some(why),
            Some(Ok(projects)) if projects.is_empty() => Some("No Workman project is bound to this folder's repo".into()),
            Some(Ok(_)) => None,
        }
    }

    pub fn open_work(&mut self, target: Target, cx: &mut Context<Self>) -> u64 {
        let open = self.tabs.iter().find(|t| matches!(&t.kind, TabKind::Work(v) if v.read(cx).target == target));
        if let Some(id) = open.map(|t| t.id) {
            self.active = Some(id);
            cx.notify();
            return id;
        }
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let folders = self.target_folders(&target);
        let cwd = folders.first().cloned();
        let bound: Vec<String> = match target.folder().and_then(|f| self.work_inbox.read(cx).folder_binding(f, cx)) {
            Some(Ok(projects)) => projects.into_iter().map(|p| p.title).collect(),
            _ => Vec::new(),
        };
        let title = work_view::title(&target, &bound.iter().map(String::as_str).collect::<Vec<_>>());
        let opening = work_view::Opening { target, folders, home: self.home.clone(), hits: self.hits.clone(), link: self.fleet_link(cx), open_link_with: self.open_link_with, repo_of: self.work_inbox.read(cx).repo_of() };
        let view = WorkView::open(opening, cx);
        cx.subscribe(&view, move |this, view, event: &work_view::WorkEvent, cx| match event {
            work_view::WorkEvent::ShowInbox => this.show_inbox(cx),
            work_view::WorkEvent::Bound => {
                let title = view.read(cx).title();
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id) {
                    tab.title = title;
                    cx.notify();
                }
            }
            work_view::WorkEvent::ItemSelected(_) => {}
        })
        .detach();
        self.tabs.push(Tab::new(id, title, TabKind::Work(view), cwd));
        self.active = Some(id);
        cx.notify();
        id
    }

    fn fleet_link(&self, cx: &mut Context<Self>) -> FleetLink {
        let (fleet, reader) = (cx.weak_entity(), cx.weak_entity());
        FleetLink {
            live: {
                let live = self.occupancy.clone();
                Rc::new(move || live.sessions())
            },
            opening: Rc::new(move |sid, cx| reader.upgrade().map_or_else(|| Open::Unlisted.opening(), |f| f.read(cx).session_opening(sid))),
            open: Rc::new(move |sid, _, cx| {
                fleet.update(cx, |fleet, cx| fleet.open_session(sid, cx)).ok();
            }),
        }
    }

    /// A Work tab's Needs you: every Inbox row shown now, the list at its top and highlighted; a toast only when nothing needs you.
    pub fn show_inbox(&mut self, cx: &mut Context<Self>) {
        self.release_left(cx);
        if self.work_inbox.read(cx).rows().is_empty() {
            self.toast.show("Nothing needs you.".into(), |this: &mut Self| &mut this.toast, cx);
            return;
        }
        self.work_inbox.update(cx, |inbox, cx| inbox.show(cx));
    }

    pub fn open_work_item(&mut self, target: Target, item_id: &str, cx: &mut Context<Self>) -> u64 {
        let id = self.open_work(target, cx);
        if let Some(Tab { kind: TabKind::Work(view), .. }) = self.tab(id) {
            let (view, item) = (view.clone(), item_id.to_string());
            view.update(cx, |view, cx| view.open_from_inbox(item, cx));
        }
        id
    }
}

pub(crate) fn attachable(live: &occupancy::Live) -> Vec<String> {
    live.sessions().into_iter().filter(|s| s.attach.is_some()).map(|s| s.name).collect()
}

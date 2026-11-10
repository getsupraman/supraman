//! The Work tab of a workspace: opened once per workspace, its folders and the fleet's links handed to it.

use std::path::Path;
use std::rc::Rc;

use gpui::Context;

use super::{FleetView, Tab, TabKind, occupancy};
use crate::work_view::{self, FleetLink, LiveSession, WorkView};

impl FleetView {
    pub fn open_work(&mut self, workspace: &str, cx: &mut Context<Self>) -> u64 {
        let open = self.tabs.iter().find(|t| matches!(&t.kind, TabKind::Work(v) if v.read(cx).workspace == workspace));
        if let Some(id) = open.map(|t| t.id) {
            self.active = Some(id);
            cx.notify();
            return id;
        }
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let fleet = cx.weak_entity();
        let name = workspace.to_string();
        let link = FleetLink {
            live: {
                let live = self.occupancy.clone();
                Rc::new(move || attachable(&live))
            },
            attach: Rc::new(move |id, window, cx| {
                fleet.update(cx, |fleet, cx| fleet.attach_by_id(&name, id, window, cx)).ok();
            }),
        };
        let found = self.workspaces.iter().find(|w| w.name == workspace);
        let folders = found.map(|w| w.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
        let cwd = found.and_then(|w| w.home()).map(Path::to_path_buf);
        let opening = work_view::Opening { workspace: workspace.to_string(), folders, home: self.home.clone(), hits: self.hits.clone(), link, open_link_with: self.open_link_with, repo_of: self.work_inbox.read(cx).repo_of() };
        let view = WorkView::open(opening, cx);
        cx.subscribe(&view, |this, _, event: &work_view::WorkEvent, cx| {
            if let work_view::WorkEvent::ShowInbox = event {
                this.show_inbox(cx);
            }
        })
        .detach();
        self.tabs.push(Tab::new(id, format!("Work · {workspace}"), TabKind::Work(view), cwd));
        self.active = Some(id);
        cx.notify();
        id
    }

    /// A Work tab's Needs you: every Inbox row shown now, the list at its top and highlighted; a toast only when nothing needs you.
    pub fn show_inbox(&mut self, cx: &mut Context<Self>) {
        self.release_left(cx);
        if self.work_inbox.read(cx).rows().is_empty() && self.needs.is_empty() {
            self.toast.show("Nothing needs you.".into(), |this: &mut Self| &mut this.toast, cx);
            return;
        }
        self.work_inbox.update(cx, |inbox, cx| inbox.show(cx));
    }

    pub fn open_work_item(&mut self, workspace: &str, item_id: &str, cx: &mut Context<Self>) -> u64 {
        let id = self.open_work(workspace, cx);
        if let Some(Tab { kind: TabKind::Work(view), .. }) = self.tab(id) {
            let (view, item) = (view.clone(), item_id.to_string());
            view.update(cx, |view, cx| view.open_from_inbox(item, cx));
        }
        id
    }
}

pub(crate) fn attachable(live: &occupancy::Live) -> Vec<LiveSession> {
    live.sessions()
        .into_iter()
        .filter_map(|s| Some(LiveSession { claude_session_id: s.claude_session_id, id: s.attach?, name: s.name }))
        .collect()
}

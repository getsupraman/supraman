//! A session's and a workspace's Details tab; its actions are the row's menu items, run through `run_action`.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{AnyElement, ClickEvent, Context, ScrollHandle, SharedString, div, prelude::*};
use supraman_core::paths::{path_key, within};
use supraman_ui::button;
use supraman_ui::details::{self, Section};
use theme::ActiveTheme as _;

use super::menu::{MenuAction, MenuItem, MenuTarget, workspace_items};
use super::schedules::At;
use super::{FleetView, Tab, TabKind};
use crate::fleet::{SessionRow, WorkspaceNode};
use crate::row_details::{self, OnDisk, SessionFacts, StartedBy, WorkspaceFacts};
use supraman_claude::now_ms;

/// The tab renders the last poll's tree; `seen` keeps a session readable once the tree drops it.
pub struct DetailsPage {
    /// [`MenuTarget::Session`] or [`MenuTarget::Workspace`].
    pub target: MenuTarget,
    pub scroll: ScrollHandle,
    pub seen: Option<SessionRow>,
    pub folder: Option<(String, PathBuf)>,
    pub listed: bool,
    pub disk: OnDisk,
    reading: bool,
}

struct DiskRead {
    tab: u64,
    cwd: PathBuf,
    session_id: String,
    known: Option<PathBuf>,
}

impl DetailsPage {
    /// Its hit keys start with `details-<key>-`.
    pub fn key(&self) -> String {
        match &self.target {
            MenuTarget::Session { session_id, .. } => format!("session-{session_id}"),
            MenuTarget::Workspace(w) => format!("ws-{w}"),
            MenuTarget::Repo { workspace, repo } => format!("repo-{workspace}/{repo}"),
            MenuTarget::Schedule { workspace, id } => format!("schedule-{workspace}/{id}"),
            MenuTarget::Schedules(workspace) => format!("schedules-{workspace}"),
            MenuTarget::OtherSchedule(path) => format!("other-{path}"),
        }
    }
}

fn same(a: &MenuTarget, b: &MenuTarget) -> bool {
    match (a, b) {
        (MenuTarget::Session { session_id: x, .. }, MenuTarget::Session { session_id: y, .. }) => x == y,
        _ => a == b,
    }
}

impl FleetView {
    pub fn details_page(&self, target: &MenuTarget) -> Option<(u64, &DetailsPage)> {
        self.tabs.iter().find_map(|t| match &t.kind {
            TabKind::Details(p) if same(&p.target, target) => Some((t.id, p)),
            _ => None,
        })
    }

    fn session_row(&self, workspace: &str, session_id: &str) -> Option<(SessionRow, Option<(String, PathBuf)>)> {
        let tree = self.tree();
        let node = tree.workspaces.iter().find(|n| n.name == workspace)?;
        Some((node.rows().find(|r| r.session.session_id == session_id)?.clone(), folder_of(node, session_id)))
    }

    pub fn open_details(&mut self, target: MenuTarget, cx: &mut Context<Self>) -> Option<u64> {
        self.dialogs.menu = None;
        if let Some((tab, _)) = self.details_page(&target) {
            self.activate_tab(tab, cx);
            return Some(tab);
        }
        let (title, cwd, seen, folder) = match &target {
            MenuTarget::Session { workspace, session_id, .. } => {
                let (row, folder) = self.session_row(workspace, session_id)?;
                (format!("Details · {}", row_details::session_title(&row)), Some(row.session.cwd.clone()), Some(row), folder)
            }
            MenuTarget::Workspace(w) => {
                let home = self.workspaces.iter().find(|r| &r.name == w)?.home().map(std::path::Path::to_path_buf);
                (format!("Details · {w}"), home, None, None)
            }
            MenuTarget::Repo { .. } | MenuTarget::Schedule { .. } | MenuTarget::Schedules(_) | MenuTarget::OtherSchedule(_) => return None,
        };
        let tab = self.next_tab_id;
        self.next_tab_id += 1;
        let read = seen.as_ref().map(|r| DiskRead { tab, cwd: r.session.cwd.clone(), session_id: r.session.session_id.clone(), known: None });
        let page = DetailsPage { target, scroll: ScrollHandle::new(), seen, folder, listed: true, disk: OnDisk::default(), reading: read.is_some() };
        self.tabs.push(Tab::new(tab, title, TabKind::Details(page), cwd));
        self.active = Some(tab);
        self.read_disk(read.into_iter().collect(), cx);
        cx.notify();
        Some(tab)
    }

    fn read_disk(&mut self, reads: Vec<DiskRead>, cx: &mut Context<Self>) {
        if reads.is_empty() {
            return;
        }
        let claude_dir = self.claude_dir.clone();
        let read = cx.background_executor().spawn(async move {
            reads
                .into_iter()
                .map(|r| {
                    let disk = OnDisk::read(&r.cwd, &r.session_id, claude_dir.as_deref(), r.known.as_deref());
                    (r.tab, r.session_id, disk)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                if apply_disk(&mut this.tabs, read) {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn refresh_details(&mut self, cx: &mut Context<Self>) {
        let _stall = supraman_ui::stall::section("poll: refresh_details");
        if !self.tabs.iter().any(|t| matches!(&t.kind, TabKind::Details(p) if matches!(p.target, MenuTarget::Session { .. }))) {
            return;
        }
        let tree = self.tree();
        let mut fresh = Vec::new();
        for (i, t) in self.tabs.iter().enumerate() {
            let TabKind::Details(page) = &t.kind else { continue };
            let MenuTarget::Session { workspace, session_id, .. } = &page.target else { continue };
            let node = tree.workspaces.iter().find(|n| &n.name == workspace);
            let row = node.and_then(|n| n.rows().find(|r| &r.session.session_id == session_id)).cloned();
            let folder = node.and_then(|n| folder_of(n, session_id));
            let read = row.as_ref().or(page.seen.as_ref()).filter(|_| !page.reading).map(|r| DiskRead {
                tab: t.id,
                cwd: r.session.cwd.clone(),
                session_id: r.session.session_id.clone(),
                known: page.disk.transcript.clone(),
            });
            fresh.push((i, row, folder, read));
        }
        let mut reads = Vec::new();
        for (i, row, folder, read) in fresh {
            if let TabKind::Details(page) = &mut self.tabs[i].kind {
                page.listed = row.is_some();
                if row.is_some() {
                    page.seen = row;
                    page.folder = folder;
                }
                if let Some(read) = read {
                    page.reading = true;
                    reads.push(read);
                }
            }
        }
        self.read_disk(reads, cx);
    }

    fn started_by(&self, workspace: &str, row: &SessionRow) -> StartedBy {
        let name = &row.session.name;
        if let Some(id) = supraman_core::schedule::schedule_of_session(workspace, name) {
            return StartedBy::Schedule(id.to_string());
        }
        if self.supervising.desired.get(workspace).is_some_and(|(_, wanted)| wanted.contains(name)) || super::menu::is_foremans(&row.session) {
            return StartedBy::Supervisor;
        }
        if self.hand_tab(row).is_some() {
            return StartedBy::Hand;
        }
        StartedBy::Outside
    }

    fn hand_tab(&self, row: &SessionRow) -> Option<&Tab> {
        let sid = &row.session.session_id;
        self.tabs.iter().find(|t| t.hand && (Self::tab_runs(t, &row.session) || t.launched_session.as_ref() == Some(sid)))
    }

    fn work_open(&self, workspace: &str, cx: &gpui::App) -> bool {
        self.tabs.iter().any(|t| matches!(&t.kind, TabKind::Work(v) if v.read(cx).workspace == workspace))
    }

    pub fn session_sections(&self, page: &DetailsPage, cx: &gpui::App) -> Option<(String, Vec<Section>)> {
        let MenuTarget::Session { workspace, repo, .. } = &page.target else { return None };
        let row = page.seen.as_ref()?;
        let registered = self.workspaces.iter().find(|w| &w.name == workspace);
        let folder = page.folder.as_ref().map(|(n, p)| (n.as_str(), p.as_path())).or_else(|| {
            registered?.folders.iter().filter(|f| within(&row.session.cwd, &f.path)).max_by_key(|f| path_key(&f.path).len()).map(|f| (f.name.as_str(), f.path.as_path()))
        });
        let adopt = match self.adoption(workspace, repo, row) {
            Some(crate::fleet::Adoption::No(why)) => Some(why),
            _ => None,
        };
        let claimed = self.work_open(workspace, cx).then(|| self.claimed_item(workspace, &row.session.name, cx));
        let facts = SessionFacts {
            row,
            workspace,
            folder,
            started_by: self.started_by(workspace, row),
            claimed,
            sign_in: self.hand_tab(row).and_then(|t| t.workman.clone()),
            open: self.open_of(row),
            adopt,
            disk: &page.disk,
            listed: page.listed,
        };
        Some((row_details::session_title(row), row_details::session(&facts, now_ms())))
    }

    pub fn workspace_sections(&self, workspace: &str, cx: &gpui::App) -> Option<(WorkspaceNode, Vec<Section>)> {
        let node = self.tree().workspaces.iter().find(|n| n.name == workspace)?.clone();
        let registered = self.workspaces.iter().find(|r| r.name == workspace);
        let facts = WorkspaceFacts {
            name: workspace.to_string(),
            home: node.home.as_ref().map(|h| h.display().to_string()),
            folders: registered.map(|r| r.folders.iter().map(|f| (f.name.clone(), f.path.clone(), f.enabled)).collect()).unwrap_or_default(),
            scans: registered.map(|r| r.scans.iter().map(|s| (s.path.clone(), s.depth)).collect()).unwrap_or_default(),
            started: node.started,
            sessions: node.session_count(),
            own: node.own.clone(),
            relay: self.mail.relays.get(workspace).and_then(|r| r.label(now_ms())).map(|(text, _)| text),
            project: self.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Work(view) if view.read(cx).workspace == workspace => {
                    let refs: Vec<String> = view.read(cx).projects.iter().map(|p| p.item.qualified_ref.clone()).collect();
                    (!refs.is_empty()).then(|| refs.join(", "))
                }
                _ => None,
            }),
        };
        let sections = row_details::workspace(&facts, now_ms());
        Some((node, sections))
    }

    /// The text of the Details tab open on `target`; for tests.
    #[cfg(feature = "selftest")]
    pub fn details_text(&self, target: &MenuTarget, cx: &gpui::App) -> Option<String> {
        let (_, page) = self.details_page(target)?;
        match &page.target {
            MenuTarget::Session { .. } => self.session_sections(page, cx).map(|(title, s)| details::text(&title, &s)),
            MenuTarget::Workspace(w) => self.workspace_sections(w, cx).map(|(_, s)| details::text(w, &s)),
            MenuTarget::Repo { .. } | MenuTarget::Schedule { .. } | MenuTarget::Schedules(_) | MenuTarget::OtherSchedule(_) => None,
        }
    }

    /// Every menu item but the Details of this page.
    fn action_buttons(&self, page: &DetailsPage, items: Vec<MenuItem>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let muted = cx.theme().colors().text_muted;
        let page_key = page.key();
        let mut out = Vec::new();
        for item in items.into_iter().filter(|i| !matches!(&i.action, MenuAction::Details(t) if same(t, &page.target))) {
            if let Some(caption) = &item.section {
                out.push(div().flex_none().pl_2().text_xs().text_color(muted).child(SharedString::from(caption.clone())).into_any_element());
            }
            let action = item.action.clone();
            out.push(
                button::bordered(&self.hits, format!("details-{page_key}-{}", item.key), item.label.clone(), cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.run_action(action.clone(), window, cx);
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    /// `None` once the workspace is gone.
    pub(super) fn render_details_page(&self, page: &DetailsPage, cx: &mut Context<Self>) -> Option<AnyElement> {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let page_key = page.key();
        let (title, pill, sections, items) = match &page.target {
            MenuTarget::Session { workspace, repo, .. } => {
                let (title, sections) = self.session_sections(page, cx)?;
                let row = page.seen.as_ref()?;
                let words = row_details::session_pill(row, page.listed);
                let color = if page.listed { crate::fleet::State::of_row(row).mark(cx).color() } else { colors.text_muted };
                let items = if page.listed { self.session_menu(workspace, repo, row, None, cx) } else { Vec::new() };
                (title, (words, color), sections, items)
            }
            MenuTarget::Workspace(w) => {
                let (node, sections) = self.workspace_sections(w, cx)?;
                let color = if node.started == Some(true) { status.success } else { colors.text_muted };
                let own = node.own.as_ref().map(|r| self.session_menu(w, "workspace", r, None, cx)).unwrap_or_default();
                let relay = node.relay.as_ref().map(|r| self.session_menu(w, "relay", r, None, cx)).unwrap_or_default();
                let items = workspace_items(&node, self.has_closed_background(w), own, relay);
                (w.clone(), (row_details::workspace_pill(node.started).to_string(), color), sections, items)
            }
            MenuTarget::Repo { .. } | MenuTarget::Schedule { .. } | MenuTarget::Schedules(_) | MenuTarget::OtherSchedule(_) => return None,
        };
        let actions = self.action_buttons(page, items, cx);
        let workspace = match &page.target {
            MenuTarget::Session { workspace, .. } => workspace.clone(),
            _ => String::new(),
        };
        let on_link: details::OnLink<Self> = Rc::new(move |this: &mut Self, schedule: &str, _, cx| {
            this.open_schedule(&workspace, schedule, At::Top, cx);
        });
        let key = format!("details-{page_key}");
        let page = details::Page { key: &key, title, pill: Some(pill), sections: &sections, actions, scroll: &page.scroll, on_link: Some(on_link), top: None, over: None, bar: None };
        Some(details::render(page, &self.hits, cx))
    }
}

/// Each read lands on its tab while that tab still shows the session it was read for; whether any did.
fn apply_disk(tabs: &mut [Tab], read: Vec<(u64, String, OnDisk)>) -> bool {
    let mut applied = false;
    for (tab, session_id, disk) in read {
        let page = tabs.iter_mut().find(|t| t.id == tab).and_then(|t| match &mut t.kind {
            TabKind::Details(page) => Some(page),
            _ => None,
        });
        if let Some(page) = page {
            page.reading = false;
            if matches!(&page.target, MenuTarget::Session { session_id: s, .. } if *s == session_id) {
                applied |= page.disk != disk;
                page.disk = disk;
            }
        }
    }
    applied
}

fn folder_of(node: &WorkspaceNode, session_id: &str) -> Option<(String, PathBuf)> {
    node.repos.iter().find(|r| r.sessions.iter().any(|s| s.session.session_id == session_id)).map(|r| (r.name.clone(), r.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(tab: u64, session_id: &str) -> Tab {
        let target = MenuTarget::Session { workspace: "demo".into(), repo: "api".into(), session_id: session_id.into() };
        let page = DetailsPage { target, scroll: ScrollHandle::new(), seen: None, folder: None, listed: true, disk: OnDisk::default(), reading: true };
        Tab::new(tab, format!("Details · {session_id}"), TabKind::Details(page), None)
    }

    fn disk_of(tabs: &[Tab], tab: u64) -> (OnDisk, bool) {
        match &tabs.iter().find(|t| t.id == tab).unwrap().kind {
            TabKind::Details(p) => (p.disk.clone(), p.reading),
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_disk_read_lands_on_its_tab_while_it_shows_that_session_and_frees_the_next_read() {
        let mut tabs = vec![page(1, "a"), page(2, "b")];
        let read = OnDisk { branch: Some("main".into()), ..OnDisk::default() };
        let landed = apply_disk(&mut tabs, vec![(1, "a".into(), read.clone()), (2, "elsewhere".into(), read.clone()), (9, "gone".into(), read.clone())]);
        assert!(landed);
        assert_eq!(disk_of(&tabs, 1), (read.clone(), false));
        assert_eq!(disk_of(&tabs, 2), (OnDisk::default(), false), "read for another session: dropped, the next poll reads again");
        assert!(!apply_disk(&mut tabs, vec![(1, "a".into(), read)]), "unchanged: no repaint");
    }
}

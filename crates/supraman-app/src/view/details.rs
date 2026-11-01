//! A session's and a workspace's Details tab: `row_details`' sections on the shared page, and the row's menu
//! items as its actions, each run through `run_action` as the menu runs it.

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
    /// The tree's folder row holding the session: name and path.
    pub folder: Option<(String, PathBuf)>,
    pub listed: bool,
    pub disk: OnDisk,
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

    /// Opens the Details tab of a session or a workspace, or focuses the one open.
    pub fn open_details(&mut self, target: MenuTarget, cx: &mut Context<Self>) -> Option<u64> {
        self.menu = None;
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
        let disk = seen.as_ref().map(|r| self.on_disk(r, None)).unwrap_or_default();
        let page = DetailsPage { target, scroll: ScrollHandle::new(), seen, folder, listed: true, disk };
        let tab = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::new(tab, title, TabKind::Details(page), cwd));
        self.active = Some(tab);
        cx.notify();
        Some(tab)
    }

    fn on_disk(&self, row: &SessionRow, known: Option<&std::path::Path>) -> OnDisk {
        OnDisk::read(&row.session.cwd, &row.session.session_id, self.claude_dir.as_deref(), known)
    }

    /// Each session page takes its row from this poll's tree, or keeps the last one seen.
    pub(super) fn refresh_details(&mut self) {
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
            let disk = row.as_ref().or(page.seen.as_ref()).map(|r| self.on_disk(r, page.disk.transcript.as_deref()));
            fresh.push((i, row, folder, disk));
        }
        for (i, row, folder, disk) in fresh {
            if let TabKind::Details(page) = &mut self.tabs[i].kind {
                page.listed = row.is_some();
                if row.is_some() {
                    page.seen = row;
                    page.folder = folder;
                }
                if let Some(disk) = disk {
                    page.disk = disk;
                }
            }
        }
    }

    fn started_by(&self, workspace: &str, row: &SessionRow) -> StartedBy {
        let name = &row.session.name;
        if let Some(id) = supraman_core::schedule::schedule_of_session(workspace, name) {
            return StartedBy::Schedule(id.to_string());
        }
        if self.desired.get(workspace).is_some_and(|(_, wanted)| wanted.contains(name)) || super::menu::is_foremans(&row.session) {
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
            sign_in: self.hand_tab(row).and_then(|t| t.workstate.clone()),
            open: self.open_of(row),
            adopt,
            disk: &page.disk,
            listed: page.listed,
        };
        Some((row_details::session_title(row), row_details::session(&facts, now_ms())))
    }

    pub fn workspace_sections(&self, workspace: &str, cx: &gpui::App) -> Option<(WorkspaceNode, Vec<Section>)> {
        let node = self.tree().workspaces.into_iter().find(|n| n.name == workspace)?;
        let registered = self.workspaces.iter().find(|r| r.name == workspace);
        let facts = WorkspaceFacts {
            name: workspace.to_string(),
            home: node.home.as_ref().map(|h| h.display().to_string()),
            folders: registered.map(|r| r.folders.iter().map(|f| (f.name.clone(), f.path.clone(), f.enabled)).collect()).unwrap_or_default(),
            scans: registered.map(|r| r.scans.iter().map(|s| (s.path.clone(), s.depth)).collect()).unwrap_or_default(),
            started: node.started,
            sessions: node.session_count(),
            own: node.own.clone(),
            relay: self.relays.get(workspace).and_then(|r| r.label(now_ms())).map(|(text, _)| text),
            project: self.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Work(view) if view.read(cx).workspace == workspace => view.read(cx).project().map(|p| p.qualified_ref.clone()),
                _ => None,
            }),
        };
        let sections = row_details::workspace(&facts, now_ms());
        Some((node, sections))
    }

    /// The text of the Details tab open on `target`; for tests.
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
                let (words, asks) = row_details::session_pill(row, page.listed);
                let color = match (asks, &row.health) {
                    (true, _) => status.warning,
                    _ if !page.listed || row.closed => colors.text_muted,
                    (false, h) if crate::fleet::is_alive(h) => status.success,
                    _ => colors.text_muted,
                };
                let items = if page.listed { self.session_menu(workspace, repo, row, None, cx) } else { Vec::new() };
                (title, (words.to_string(), color), sections, items)
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
        let page = details::Page { key: &key, title, pill: Some(pill), sections: &sections, actions, scroll: &page.scroll, on_link: Some(on_link), top: None };
        Some(details::render(page, &self.hits, cx))
    }
}

fn folder_of(node: &WorkspaceNode, session_id: &str) -> Option<(String, PathBuf)> {
    node.repos.iter().find(|r| r.sessions.iter().any(|s| s.session.session_id == session_id)).map(|r| (r.name.clone(), r.path.clone()))
}

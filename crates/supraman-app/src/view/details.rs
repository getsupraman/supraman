//! A session's, a group's and a top-level folder's Details tab; its actions are the row's menu items, run through `run_action`.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{AnyElement, ClickEvent, Context, ScrollHandle, SharedString, div, prelude::*};
use supraman_ui::{button, session};
use supraman_ui::details::{self, Section};
use theme::ActiveTheme as _;

use super::menu::{MenuAction, MenuItem, MenuTarget, group_items};
use super::schedules::At;
use super::{FleetView, Tab, TabKind};
use crate::fleet::{EntryNode, SessionRow};
use crate::row_details::{self, EntryFacts, OnDisk, SessionFacts, StartedBy};
use supraman_claude::now_ms;

/// The tab renders the last poll's tree; `seen` keeps a session readable once the tree drops it.
pub struct DetailsPage {
    /// [`MenuTarget::Session`], [`MenuTarget::Group`] or a top-level [`MenuTarget::Folder`].
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
            MenuTarget::Group(name) => format!("group-{name}"),
            MenuTarget::Folder(key) => format!("repo-{key}"),
            MenuTarget::Schedule { folder_key, id } => format!("schedule-{folder_key}/{id}"),
            MenuTarget::Schedules { folder, .. } => format!("schedules-{}", supraman_core::schedule::folder_key(folder)),
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

    fn session_row(&self, session_id: &str) -> Option<(SessionRow, Option<(String, PathBuf)>)> {
        let tree = self.tree();
        let node = tree.entries.iter().find(|n| n.rows().any(|r| r.session.session_id == session_id))?;
        Some((node.rows().find(|r| r.session.session_id == session_id)?.clone(), folder_of(node, session_id)))
    }

    pub fn open_details(&mut self, target: MenuTarget, cx: &mut Context<Self>) -> Option<u64> {
        self.dialogs.menu = None;
        if let Some((tab, _)) = self.details_page(&target) {
            self.activate_tab(tab, cx);
            return Some(tab);
        }
        let (title, cwd, seen, folder) = match &target {
            MenuTarget::Session { session_id, .. } => {
                let (row, folder) = self.session_row(session_id)?;
                (format!("Details · {}", row_details::session_title(&row)), Some(row.session.cwd.clone()), Some(row), folder)
            }
            MenuTarget::Group(name) | MenuTarget::Folder(name) => {
                let first = self.details_node(name)?.folders.first().map(|f| f.path.clone());
                (format!("Details · {name}"), first, None, None)
            }
            MenuTarget::Schedule { .. } | MenuTarget::Schedules { .. } | MenuTarget::OtherSchedule(_) => return None,
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
            let MenuTarget::Session { session_id, .. } = &page.target else { continue };
            let node = tree.entries.iter().find(|n| n.rows().any(|r| &r.session.session_id == session_id));
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

    fn started_by(&self, row: &SessionRow) -> StartedBy {
        let name = &row.session.name;
        if let Some((_, id)) = supraman_core::schedule::schedule_of_session(name).filter(|(key, _)| self.own_names().iter().any(|k| k == key)) {
            return StartedBy::Schedule(id.to_string());
        }
        if super::menu::is_foremans(&row.session) {
            return StartedBy::Supraman;
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

    fn work_open(&self, entry: &str, cx: &gpui::App) -> bool {
        self.tabs.iter().any(|t| matches!(&t.kind, TabKind::Work(v) if v.read(cx).workspace == entry))
    }

    pub fn session_sections(&self, page: &DetailsPage, cx: &gpui::App) -> Option<(String, Vec<Section>)> {
        let MenuTarget::Session { entry, .. } = &page.target else { return None };
        let row = page.seen.as_ref()?;
        let folder = page.folder.as_ref().map(|(n, p)| (n.as_str(), p.as_path()));
        Some((row_details::session_title(row), self.row_sections(entry, row, folder, &page.disk, page.listed, cx)))
    }

    /// `row`'s facts as its Details tab shows them; `folder` the listed folder holding it, else found here.
    pub(super) fn row_sections(&self, entry: &str, row: &SessionRow, folder: Option<(&str, &std::path::Path)>, disk: &OnDisk, listed: bool, cx: &gpui::App) -> Vec<Section> {
        let shown = &self.listed.disk.shown;
        let (group, found) = crate::tab_card::place(&row.session.cwd, &self.listed.entries, shown);
        let holding = supraman_claude::holding(&row.session.cwd, &self.listed.entries, shown, |_| None).map(|at| at.folder);
        let folder = folder.or_else(|| Some((found.as_deref()?, holding.as_deref()?)));
        let claimed = self.work_open(entry, cx).then(|| self.claimed_item(entry, &row.session.name, cx));
        let facts = SessionFacts {
            row,
            group: group.as_deref(),
            folder,
            started_by: self.started_by(row),
            claimed,
            sign_in: self.hand_tab(row).and_then(|t| t.workman.clone()),
            disk,
            listed,
        };
        row_details::session(&facts, now_ms())
    }

    /// The entry named `name` in the tree, or a group's folder by its key on its own.
    fn details_node(&self, name: &str) -> Option<EntryNode> {
        let tree = self.tree();
        tree.entry(name).cloned().or_else(|| {
            let (_, folder) = tree.folders().find(|(_, f)| f.key == name)?;
            Some(EntryNode { name: name.to_string(), group: false, folders: vec![folder.clone()] })
        })
    }

    /// A group's or a folder's page.
    pub fn entry_sections(&self, name: &str, cx: &gpui::App) -> Option<(EntryNode, Vec<Section>)> {
        let node = self.details_node(name)?;
        let facts = EntryFacts {
            name: name.to_string(),
            group: node.group,
            folders: node.folders.iter().filter(|f| f.listed() || node.folders.len() == 1).map(|f| (f.name.clone(), f.path.clone())).collect(),
            sessions: node.session_count(),
            project: self.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Work(view) if view.read(cx).workspace == name => {
                    let refs: Vec<String> = view.read(cx).projects.iter().map(|p| p.item.qualified_ref.clone()).collect();
                    (!refs.is_empty()).then(|| refs.join(", "))
                }
                _ => None,
            }),
        };
        let sections = row_details::entry(&facts);
        Some((node, sections))
    }

    /// The text of the Details tab open on `target`; for tests.
    #[cfg(feature = "selftest")]
    pub fn details_text(&self, target: &MenuTarget, cx: &gpui::App) -> Option<String> {
        let (_, page) = self.details_page(target)?;
        match &page.target {
            MenuTarget::Session { .. } => self.session_sections(page, cx).map(|(title, s)| details::text(&title, &s)),
            MenuTarget::Group(name) | MenuTarget::Folder(name) => self.entry_sections(name, cx).map(|(_, s)| details::text(name, &s)),
            MenuTarget::Schedule { .. } | MenuTarget::Schedules { .. } | MenuTarget::OtherSchedule(_) => None,
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
                button::action(&self.hits, format!("details-{page_key}-{}", item.key), item.label.clone(), button::Tone::Neutral, button::Fill::Solid, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.run_action(action.clone(), window, cx);
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    /// `None` once its group or folder is gone.
    pub(super) fn render_details_page(&self, page: &DetailsPage, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page_key = page.key();
        let mut opener = None;
        let (title, pill, sections, items) = match &page.target {
            MenuTarget::Session { entry, place, session_id } => {
                let row = page.seen.as_ref()?;
                let folder = page.folder.as_ref().map(|(n, p)| (n.as_str(), p.as_path()));
                let view = self.session_view(entry, place, row, folder, &page.disk, page.listed, true, cx);
                let (fleet, sid) = (cx.weak_entity(), session_id.clone());
                let on_open = move |_: &mut gpui::Window, cx: &mut gpui::App| {
                    fleet.update(cx, |this, cx| this.open_session(&sid, cx)).ok();
                };
                let open = session::open(&self.hits, format!("details-{page_key}-open"), &view.open.opening(), button::Fill::Solid, on_open, cx);
                opener = Some(open.into_any_element());
                (view.title, Some(view.pill), view.sections, view.actions)
            }
            MenuTarget::Group(name) | MenuTarget::Folder(name) => {
                let (node, sections) = self.entry_sections(name, cx)?;
                let items = if node.group { group_items(&node, self.has_closed_background(name)) } else { self.folder_menu(name).map(|(_, items)| items).unwrap_or_default() };
                (name.clone(), None, sections, items)
            }
            MenuTarget::Schedule { .. } | MenuTarget::Schedules { .. } | MenuTarget::OtherSchedule(_) => return None,
        };
        let actions = opener.into_iter().chain(self.action_buttons(page, items, cx)).collect();
        let entry = match &page.target {
            MenuTarget::Session { entry, .. } => entry.clone(),
            _ => String::new(),
        };
        let on_link: details::OnLink<Self> = Rc::new(move |this: &mut Self, schedule: &str, _, cx| {
            this.open_schedule(&entry, schedule, At::Top, cx);
        });
        let key = format!("details-{page_key}");
        let page = details::Page { key: &key, title, pill, sections: &sections, actions, scroll: &page.scroll, on_link: Some(on_link), top: None, over: None, bar: None };
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

fn folder_of(node: &EntryNode, session_id: &str) -> Option<(String, PathBuf)> {
    node.folders.iter().find(|r| r.sessions.iter().any(|s| s.session.session_id == session_id)).map(|r| (r.name.clone(), r.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(tab: u64, session_id: &str) -> Tab {
        let target = MenuTarget::Session { entry: "demo".into(), place: "demo/api".into(), session_id: session_id.into() };
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

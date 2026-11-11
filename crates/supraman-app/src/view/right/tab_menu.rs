//! A tab's right-click menu: its folder's repository and Workman project, the folder itself, closing, and a session tab's row items.

use std::path::{Path, PathBuf};

use gpui::{App, Context, Pixels, Point};
use supraman_core::holder_of;
use supraman_core::paths::git_at;
use supraman_ui::menu::item;

use crate::fleet::WorkspaceNode;
use crate::view::menu::{ContextMenu, MenuAction, MenuItem};
use crate::view::{FleetView, Tab, TabKind};
use crate::work_view::Binding;

/// Where a tab's folder stands in git.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repo {
    /// [`repo_top`] runs off the UI thread.
    Reading,
    Top(PathBuf),
    Outside,
}

/// Where a tab's folder stands in Workman.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Work {
    /// `WorkInbox::bind_folder` runs off the UI thread.
    Reading,
    Bound { workspace: String, project: String },
    Unbound,
    NoWorkspace,
    /// Workman's own words: not set up, not signed in, unreachable.
    Unavailable(String),
}

pub struct TabFacts<'a> {
    pub id: u64,
    pub folder: Option<&'a Path>,
    pub repo: &'a Repo,
    pub others: bool,
    pub right: bool,
    pub reopenable: bool,
}

/// The folder holding the `.git` that `folder` is in: a worktree's own root, the key of its Git tab.
pub fn repo_top(folder: &Path) -> Option<PathBuf> {
    git_at(folder)?.parent().map(Path::to_path_buf)
}

/// `session`: the session row's items and their caption, for a tab that runs a listed session.
pub fn tab_items(tab: TabFacts<'_>, session: Option<(String, Vec<MenuItem>)>) -> Vec<MenuItem> {
    let id = tab.id;
    let no_folder = NO_FOLDER;
    let open_git = open_git_item(tab.folder, tab.repo);
    let folder = tab.folder.map(Path::to_path_buf);
    let copy = item("tab-copy-path", "Copy folder path", MenuAction::Copy(folder.as_deref().map(|f| f.display().to_string()).unwrap_or_default()));
    let open = item("tab-open-folder", "Open folder", MenuAction::OpenFolder(folder.clone().unwrap_or_default()));
    let (copy, open) = if folder.is_some() { (copy, open) } else { (copy.dimmed(no_folder), open.dimmed(no_folder)) };
    let only_if = |item: MenuItem, ok: bool, why: &str| if ok { item } else { item.dimmed(why) };
    let mut items = vec![
        open_git,
        copy,
        open,
        item("tab-close", "Close", MenuAction::CloseTab(id)).rule(),
        only_if(item("tab-close-others", "Close other tabs", MenuAction::CloseOtherTabs(id)), tab.others, "No other tab"),
        only_if(item("tab-close-right", "Close tabs to the right", MenuAction::CloseTabsRight(id)), tab.right, "No tab to its right"),
        only_if(item("tab-reopen", "Reopen closed tab", MenuAction::ReopenClosed), tab.reopenable, "No closed tab to reopen"),
    ];
    if let Some((caption, mut rows)) = session.filter(|(_, rows)| !rows.is_empty()) {
        rows[0] = rows[0].clone().under(caption);
        items.extend(rows);
    }
    items
}

const NO_FOLDER: &str = "This tab has no folder";

fn open_git_item(folder: Option<&Path>, repo: &Repo) -> MenuItem {
    match (folder, repo) {
        (None, _) => item("tab-open-git", "Open Git", MenuAction::OpenGit(PathBuf::new())).dimmed(NO_FOLDER),
        (Some(_), Repo::Top(top)) => item("tab-open-git", "Open Git", MenuAction::OpenGit(top.clone())),
        (Some(folder), Repo::Reading) => item("tab-open-git", "Open Git", MenuAction::OpenGit(folder.to_path_buf())).dimmed("Looking for its repository…"),
        (Some(folder), Repo::Outside) => item("tab-open-git", "Open Git", MenuAction::OpenGit(folder.to_path_buf())).dimmed("Not in a git repository"),
    }
}

pub fn workman_item(folder: Option<&Path>, work: &Work) -> MenuItem {
    let open = |workspace: &str, project: &str| item("tab-open-workman", "Open Workman", MenuAction::OpenWorkman { workspace: workspace.into(), project: project.into() });
    match (folder, work) {
        (None, _) => open("", "").dimmed(NO_FOLDER),
        (Some(_), Work::Bound { workspace, project }) => open(workspace, project),
        (Some(_), Work::Reading) => open("", "").dimmed("Looking for its Workman project…"),
        (Some(_), Work::Unbound) => open("", "").dimmed("Not bound to a Workman project"),
        (Some(_), Work::NoWorkspace) => open("", "").dimmed("Not in a Supraman workspace"),
        (Some(_), Work::Unavailable(why)) => open("", "").dimmed(why),
    }
}

/// [`tab_items`] with Open Workman right after Open Git.
pub fn with_workman(mut items: Vec<MenuItem>, folder: Option<&Path>, work: &Work) -> Vec<MenuItem> {
    items.insert(1, workman_item(folder, work));
    items
}

/// The folder a session row is listed under, as the sidebar names it for its menu.
pub(in crate::view) fn row_folder<'a>(node: &'a WorkspaceNode, session_id: &str) -> &'a str {
    let is = |row: &crate::fleet::SessionRow| row.session.session_id == session_id;
    if node.own.as_ref().is_some_and(is) {
        return "workspace";
    }
    if node.relay.as_ref().is_some_and(is) {
        return "relay";
    }
    if let Some((id, _)) = node.scheduled.iter().find(|(_, rows)| rows.iter().any(is)) {
        return id;
    }
    node.repos.iter().find(|r| r.sessions.iter().any(is)).map_or("supervised", |r| &r.name)
}

impl FleetView {
    pub(in crate::view) fn open_tab_menu(&mut self, id: u64, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(tab) = self.tab(id) else { return };
        let folder = self.tab_folder(tab, cx);
        let repo = match (&tab.kind, &folder) {
            (_, None) => Repo::Outside,
            (TabKind::Git(_), Some(own)) => Repo::Top(own.clone()),
            (_, Some(_)) => Repo::Reading,
        };
        let workspace = folder.as_deref().and_then(|f| holder_of(f, &self.workspaces)).map(|w| w.name.clone());
        let work = if workspace.is_some() { Work::Reading } else { Work::NoWorkspace };
        let title = tab.label();
        let items = with_workman(self.tab_menu_items(id, folder.as_deref(), &repo, cx), folder.as_deref(), &work);
        self.dialogs.menu = Some(ContextMenu { title, position, items });
        cx.notify();
        let Some(folder) = folder else { return };
        if repo == Repo::Reading {
            let at = folder.clone();
            let read = cx.background_executor().spawn(async move { open_git_item(Some(&at), &repo_top(&at).map_or(Repo::Outside, Repo::Top)) });
            cx.spawn(async move |this, cx| {
                let item = read.await;
                this.update(cx, |this, cx| this.replace_tab_menu_item(id, position, item, cx)).ok();
            })
            .detach();
        }
        if let Some(workspace) = workspace {
            self.read_tab_work(id, position, folder, workspace, cx);
        }
    }

    fn read_tab_work(&mut self, id: u64, position: Point<Pixels>, folder: PathBuf, workspace: String, cx: &mut Context<Self>) {
        let bind = self.work_inbox.update(cx, |inbox, cx| inbox.bind_folder(folder.clone(), cx));
        cx.spawn(async move |this, cx| {
            let work = match bind.await {
                Binding::Project(project) => Work::Bound { workspace, project: project.id },
                Binding::Unbound => Work::Unbound,
                Binding::Unavailable(why) => Work::Unavailable(why),
            };
            let item = workman_item(Some(&folder), &work);
            this.update(cx, |this, cx| this.replace_tab_menu_item(id, position, item, cx)).ok();
        })
        .detach();
    }

    /// The item under `item`'s key, while the menu shown is still this tab's at `position`.
    fn replace_tab_menu_item(&mut self, id: u64, position: Point<Pixels>, item: MenuItem, cx: &mut Context<Self>) {
        let shown = self.dialogs.menu.as_mut().filter(|m| m.position == position && m.items.iter().any(|i| i.action == MenuAction::CloseTab(id)));
        if let Some(slot) = shown.and_then(|m| m.items.iter_mut().find(|i| i.key == item.key)) {
            *slot = item;
            cx.notify();
        }
    }

    fn tab_menu_items(&self, id: u64, folder: Option<&Path>, repo: &Repo, cx: &Context<Self>) -> Vec<MenuItem> {
        let at = self.tabs.iter().position(|t| t.id == id).unwrap_or_default();
        let facts = TabFacts { id, folder, repo, others: self.tabs.len() > 1, right: at + 1 < self.tabs.len(), reopenable: self.tab_keys.read(cx).has_closed() };
        tab_items(facts, self.tab(id).and_then(|tab| self.tab_session_items(tab, cx)))
    }

    /// A Git tab's own folder, a Work tab's workspace home, a Start page's folder, else the tab's `cwd`.
    fn tab_folder(&self, tab: &Tab, cx: &App) -> Option<PathBuf> {
        match &tab.kind {
            TabKind::Git(view) => Some(view.read(cx).folder.clone()),
            TabKind::Work(view) => self.workspaces.iter().find(|w| w.name == view.read(cx).workspace)?.home().map(Path::to_path_buf),
            TabKind::Start(page) => Some(page.folder.clone()),
            _ => tab.cwd.clone(),
        }
    }

    fn tab_session_items(&self, tab: &Tab, cx: &Context<Self>) -> Option<(String, Vec<MenuItem>)> {
        let tree = self.tree();
        tree.workspaces.iter().find_map(|node| {
            let row = node.rows().find(|r| Self::tab_runs(tab, &r.session))?;
            let folder = row_folder(node, &row.session.session_id);
            let caption = format!("session · {}", crate::fleet::display_name(&row.session));
            Some((caption, self.session_menu(&node.name, folder, row, None, cx)))
        })
    }

    pub(in crate::view) fn run_tab_action(&mut self, action: MenuAction, cx: &mut Context<Self>) {
        let ids = |this: &Self, keep: &dyn Fn(usize, u64) -> bool| -> Vec<u64> { this.tabs.iter().enumerate().filter(|(i, t)| keep(*i, t.id)).map(|(_, t)| t.id).collect() };
        match action {
            MenuAction::OpenGit(top) => {
                let title = crate::view::git_tab::title(&top);
                self.open_git(top, title, cx);
            }
            MenuAction::OpenWorkman { workspace, project } => {
                let id = self.open_work(&workspace, cx);
                if let Some(Tab { kind: TabKind::Work(view), .. }) = self.tab(id) {
                    view.clone().update(cx, |view, cx| view.show_project(project, cx));
                }
            }
            MenuAction::CloseTab(id) => self.close_tab(id, cx),
            MenuAction::CloseOtherTabs(id) => {
                self.activate_tab(id, cx);
                ids(self, &|_, t| t != id).into_iter().for_each(|t| self.close_tab(t, cx));
            }
            MenuAction::CloseTabsRight(id) => {
                let Some(at) = self.tabs.iter().position(|t| t.id == id) else { return };
                ids(self, &|i, _| i > at).into_iter().for_each(|t| self.close_tab(t, cx));
            }
            MenuAction::ReopenClosed => self.reopen_closed(cx),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|i| &*i.key).collect()
    }

    fn dimmed<'a>(items: &'a [MenuItem], key: &str) -> Option<&'a str> {
        items.iter().find(|i| i.key == key).and_then(|i| i.dimmed.as_deref())
    }

    const ALL: [&str; 7] = ["tab-open-git", "tab-copy-path", "tab-open-folder", "tab-close", "tab-close-others", "tab-close-right", "tab-reopen"];

    fn facts<'a>(folder: Option<&'a Path>, repo: &'a Repo) -> TabFacts<'a> {
        TabFacts { id: 7, folder, repo, others: true, right: true, reopenable: true }
    }

    #[test]
    fn a_tab_in_a_repository_opens_its_top_and_acts_on_its_folder() {
        let (folder, top) = (Path::new(r"C:\shop\src"), Repo::Top(r"C:\shop".into()));
        let items = tab_items(facts(Some(folder), &top), None);
        assert_eq!(keys(&items), ALL);
        assert!(items.iter().all(|i| i.dimmed.is_none()));
        assert_eq!(items[0].action, MenuAction::OpenGit(r"C:\shop".into()), "the repository's top, never the subfolder");
        assert_eq!(items[1].action, MenuAction::Copy(r"C:\shop\src".into()));
        assert_eq!(items[2].action, MenuAction::OpenFolder(r"C:\shop\src".into()));
        assert_eq!(items[3].section.as_deref(), Some(""), "closing is a group of its own");
        assert_eq!([&items[3].action, &items[4].action, &items[5].action, &items[6].action], [&MenuAction::CloseTab(7), &MenuAction::CloseOtherTabs(7), &MenuAction::CloseTabsRight(7), &MenuAction::ReopenClosed]);
    }

    #[test]
    fn open_git_is_dimmed_with_why_outside_a_repository_while_reading_and_without_a_folder() {
        let folder = Path::new(r"C:\notes");
        let outside = tab_items(facts(Some(folder), &Repo::Outside), None);
        assert_eq!(dimmed(&outside, "tab-open-git"), Some("Not in a git repository"));
        assert_eq!(dimmed(&outside, "tab-copy-path"), None, "the folder is still there to copy and open");
        let reading = tab_items(facts(Some(folder), &Repo::Reading), None);
        assert_eq!(dimmed(&reading, "tab-open-git"), Some("Looking for its repository…"));
        let none = tab_items(facts(None, &Repo::Outside), None);
        assert_eq!(keys(&none), ALL);
        for key in ["tab-open-git", "tab-copy-path", "tab-open-folder"] {
            assert_eq!(dimmed(&none, key), Some("This tab has no folder"), "{key}");
        }
        assert_eq!(dimmed(&none, "tab-close"), None);
    }

    #[test]
    fn closing_items_are_dimmed_when_there_is_nothing_to_close_or_reopen() {
        let repo = Repo::Outside;
        let alone = tab_items(TabFacts { id: 1, folder: None, repo: &repo, others: false, right: false, reopenable: false }, None);
        assert_eq!(dimmed(&alone, "tab-close-others"), Some("No other tab"));
        assert_eq!(dimmed(&alone, "tab-close-right"), Some("No tab to its right"));
        assert_eq!(dimmed(&alone, "tab-reopen"), Some("No closed tab to reopen"));
        assert_eq!(dimmed(&alone, "tab-close"), None, "a tab can always close");
        let last = tab_items(TabFacts { id: 1, folder: None, repo: &repo, others: true, right: false, reopenable: true }, None);
        assert_eq!((dimmed(&last, "tab-close-others"), dimmed(&last, "tab-close-right"), dimmed(&last, "tab-reopen")), (None, Some("No tab to its right"), None));
    }

    #[test]
    fn a_session_tab_has_its_rows_items_after_its_own_under_a_caption() {
        let repo = Repo::Top(r"C:\shop".into());
        let rows = vec![item("attach", "Attach", MenuAction::ReopenClosed), item("details", "Details", MenuAction::ReopenClosed)];
        let items = tab_items(facts(Some(Path::new(r"C:\shop")), &repo), Some(("session · fix".into(), rows)));
        assert_eq!(keys(&items), [&ALL[..], &["attach", "details"]].concat());
        assert_eq!(items[7].section.as_deref(), Some("session · fix"));
        assert_eq!(items[8].section, None);
        let empty = tab_items(facts(None, &repo), Some(("session · fix".into(), vec![])));
        assert_eq!(keys(&empty), ALL, "no caption without items");
    }

    #[test]
    fn the_repository_top_is_the_folder_holding_dot_git_even_for_a_linked_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("shop");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::create_dir_all(main.join("src").join("deep")).unwrap();
        assert_eq!(repo_top(&main.join("src").join("deep")), Some(main.clone()));
        assert_eq!(repo_top(&main), Some(main.clone()));
        let linked = dir.path().join("shop-wt");
        std::fs::create_dir_all(linked.join("lib")).unwrap();
        std::fs::write(linked.join(".git"), "gitdir: ../shop/.git/worktrees/wt\n").unwrap();
        assert_eq!(repo_top(&linked.join("lib")), Some(linked), "the worktree's own root, as its Git tab is keyed");
    }
    #[test]
    fn open_workman_follows_open_git_and_opens_the_bound_projects_work_tab() {
        let (folder, top) = (Path::new(r"C:\shop\src"), Repo::Top(r"C:\shop".into()));
        let bound = Work::Bound { workspace: "shop".into(), project: "p-1".into() };
        let items = with_workman(tab_items(facts(Some(folder), &top), None), Some(folder), &bound);
        assert_eq!(keys(&items)[..3], ["tab-open-git", "tab-open-workman", "tab-copy-path"]);
        assert_eq!(items.len(), ALL.len() + 1);
        assert_eq!(dimmed(&items, "tab-open-workman"), None);
        assert_eq!(items[1].action, MenuAction::OpenWorkman { workspace: "shop".into(), project: "p-1".into() });
    }

    #[test]
    fn open_workman_is_dimmed_with_why_without_a_bound_project() {
        let folder = Some(Path::new(r"C:
otes"));
        let why = |folder, work: Work| workman_item(folder, &work).dimmed;
        assert_eq!(why(folder, Work::Unbound).as_deref(), Some("Not bound to a Workman project"));
        assert_eq!(why(folder, Work::Reading).as_deref(), Some("Looking for its Workman project…"));
        assert_eq!(why(folder, Work::NoWorkspace).as_deref(), Some("Not in a Supraman workspace"));
        assert_eq!(why(folder, Work::Unavailable("Workman is not signed in.".into())).as_deref(), Some("Workman is not signed in."));
        assert_eq!(why(None, Work::Reading).as_deref(), Some("This tab has no folder"));
    }
}

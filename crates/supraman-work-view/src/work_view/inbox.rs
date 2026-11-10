//! Supraman writes nothing to Workman (C21): dismissals are local ([`Dismissed`]).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use supraman_core::store::{JsonStore, Save};
use gpui::Task;
use supraman_workman::{Inbox, InboxRow, Overview};

use super::*;
use super::cards;

pub const DISMISSED_FILE: &str = "workstate-dismissed.json";
/// The Inbox's Housekeeping header; painted as `<key>=open` or `<key>=closed`.
pub const HOUSEKEEPING_KEY: &str = "inbox-housekeeping";

/// Per workspace: row key (`kind:item`) → the row's `since` when dismissed; see [`Dismissed::prune`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Dismissed(BTreeMap<String, BTreeMap<String, String>>);

impl Dismissed {
    pub(crate) fn load(home: Option<&Path>) -> (JsonStore<Self>, Option<String>) {
        JsonStore::load(home.map(|h| h.join(DISMISSED_FILE)), "every Workman inbox row shows")
    }

    pub(crate) fn hides(&self, workspace: &str, row: &InboxRow) -> bool {
        self.0.get(workspace).and_then(|keys| keys.get(&row.key)) == Some(&row.since)
    }

    pub(crate) fn dismiss(&mut self, workspace: &str, row: &InboxRow) {
        self.0.entry(workspace.to_string()).or_default().insert(row.key.clone(), row.since.clone());
    }

    /// Only after a successful read of `workspace`'s inbox.
    pub(crate) fn prune(&mut self, workspace: &str, rows: &[InboxRow]) -> bool {
        let Some(keys) = self.0.get_mut(workspace) else { return false };
        let before = keys.len();
        keys.retain(|key, since| rows.iter().any(|r| &r.key == key && &r.since == since));
        let changed = keys.len() != before;
        if keys.is_empty() {
            self.0.remove(workspace);
        }
        changed
    }

    pub(crate) fn retain_workspaces(&mut self, workspaces: &[String]) -> bool {
        let before = self.0.len();
        self.0.retain(|w, _| workspaces.contains(w));
        self.0.len() != before
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Touched {
    Workspaces(Vec<String>),
    /// No run, or a run the last overview did not list: every inbox, and the overview again.
    All { overview: bool },
}

pub(crate) fn touched(event: &StreamEvent, run_project: &HashMap<String, String>, projects: &HashMap<String, Vec<ItemRef>>) -> Touched {
    match event.run_id.as_deref() {
        None => Touched::All { overview: false },
        Some(run) => match run_project.get(run) {
            None => Touched::All { overview: true },
            Some(project) => {
                let mut hit: Vec<String> = projects.iter().filter(|(_, ps)| ps.iter().any(|p| &p.id == project)).map(|(w, _)| w.clone()).collect();
                hit.sort();
                Touched::Workspaces(hit)
            }
        },
    }
}

#[derive(Default)]
struct Plan {
    all: bool,
    overview: bool,
    workspaces: HashSet<String>,
}

type Keys = Vec<(&'static str, String)>;

struct Ask {
    workspace: String,
    folders: Vec<PathBuf>,
    keys: Option<Keys>,
    projects: Option<Vec<ItemRef>>,
}

enum Outcome {
    Read(Vec<ItemRef>, Inbox),
    NoProject,
    Failed(String),
}

struct Answer {
    workspace: String,
    keys: Option<Keys>,
    outcome: Outcome,
}

/// One inbox of every project bound to the workspace's repos (`projects::bound`), newest first; a fresh overview binds again.
fn read(client: &Client, asks: Vec<Ask>, overview: bool, repo_of: RepoOf) -> (Option<Overview>, Vec<Answer>) {
    let needs_overview = overview || asks.iter().any(|a| a.projects.is_none());
    let overview = if needs_overview { client.overview().map_err(|e| e.to_string()) } else { Err(String::new()) };
    let answers = asks
        .into_iter()
        .map(|ask| {
            let keys = ask.keys.unwrap_or_else(|| projects::folder_keys(&ask.folders, repo_of));
            let projects = match (&overview, ask.projects) {
                (Ok(overview), _) => Ok(projects::bound(overview, &keys).into_iter().map(|p| p.item).collect::<Vec<_>>()),
                (Err(_), Some(projects)) => Ok(projects),
                (Err(why), None) => Err(why.clone()),
            };
            let outcome = match projects {
                Ok(projects) if projects.is_empty() => Outcome::NoProject,
                Ok(projects) => merged(client, projects),
                Err(why) => Outcome::Failed(why),
            };
            Answer { workspace: ask.workspace, keys: Some(keys), outcome }
        })
        .collect();
    (overview.ok(), answers)
}

fn merged(client: &Client, projects: Vec<ItemRef>) -> Outcome {
    let mut all = Inbox::default();
    for project in &projects {
        match client.inbox(Some(&project.id)) {
            Ok(inbox) => {
                all.seq = all.seq.max(inbox.seq);
                all.as_of = inbox.as_of;
                let fresh: Vec<InboxRow> = inbox.rows.into_iter().filter(|r| !all.rows.iter().any(|seen| seen.key == r.key)).collect();
                all.rows.extend(fresh);
            }
            Err(e) => return Outcome::Failed(e.to_string()),
        }
    }
    all.rows.sort_by_key(|r| std::cmp::Reverse(supraman_core::time::rfc3339_ms(&r.since)));
    Outcome::Read(projects, absolute_urls(all, client.base()))
}

/// A row's `url` as the page to open: a path is on the Workman server.
fn absolute_urls(mut inbox: Inbox, base: &str) -> Inbox {
    for row in &mut inbox.rows {
        if let Some(url) = row.url.as_mut().filter(|u| u.starts_with('/')) {
            *url = format!("{}{url}", supraman_core::url::origin(base));
        }
    }
    inbox
}

pub struct WorkInbox {
    hub: Entity<WorkHub>,
    workspaces: Vec<String>,
    folders: HashMap<String, Vec<PathBuf>>,
    keys: HashMap<String, Keys>,
    repo_of: RepoOf,
    rebind: bool,
    projects: HashMap<String, Vec<ItemRef>>,
    inboxes: HashMap<String, Inbox>,
    /// Run id → project id, from the last overview.
    run_project: HashMap<String, String>,
    overview: Option<Arc<Overview>>,
    pub(crate) dismissed: JsonStore<Dismissed>,
    pending: Option<Plan>,
    pub(crate) failures: HashMap<String, String>,
    pub notice: Option<String>,
    pub reads: u64,
    pub applied: u64,
    pub housekeeping_open: bool,
    /// The sidebar's Inbox list: scrolled to its top when it is shown.
    pub scroll: ScrollHandle,
    flash: Option<Task<()>>,
}

pub(crate) const FLASH: Duration = Duration::from_millis(1200);

struct GlobalInbox(Entity<WorkInbox>);

impl Global for GlobalInbox {}

impl WorkInbox {
    pub fn shared(home: Option<PathBuf>, cx: &mut App) -> Entity<WorkInbox> {
        if let Some(GlobalInbox(inbox)) = cx.try_global::<GlobalInbox>() {
            return inbox.clone();
        }
        let hub = WorkHub::shared(home.clone(), cx);
        let inbox = cx.new(|cx| Self::new(hub, home.as_deref(), cx));
        cx.set_global(GlobalInbox(inbox.clone()));
        inbox
    }

    /// An inbox reading through `hub`, kept nowhere: a test's.
    #[cfg(test)]
    pub(crate) fn for_test(hub: Entity<WorkHub>, cx: &mut App) -> Entity<WorkInbox> {
        cx.new(|cx| Self::new(hub, None, cx))
    }

    /// An inbox of no workspace finding repos with `repo_of`: a test's.
    #[cfg(test)]
    pub(crate) fn of_repos(hub: Entity<WorkHub>, repo_of: RepoOf, cx: &mut App) -> Entity<WorkInbox> {
        cx.new(|cx| WorkInbox { repo_of, ..Self::new(hub, None, cx) })
    }

    fn new(hub: Entity<WorkHub>, home: Option<&Path>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&hub, Self::on_hub).detach();
        let (dismissed, notice) = Dismissed::load(home);
        WorkInbox {
            hub,
            workspaces: Vec::new(),
            folders: HashMap::new(),
            keys: HashMap::new(),
            repo_of: supraman_supervisor::git_repo,
            rebind: false,
            projects: HashMap::new(),
            inboxes: HashMap::new(),
            run_project: HashMap::new(),
            overview: None,
            dismissed,
            pending: None,
            failures: HashMap::new(),
            notice,
            reads: 0,
            applied: 0,
            housekeeping_open: false,
            scroll: ScrollHandle::new(),
            flash: None,
        }
    }

    pub fn use_repo_of(&mut self, repo_of: RepoOf) {
        self.rebind |= repo_of as usize != self.repo_of as usize;
        self.repo_of = repo_of;
    }

    pub fn repo_of(&self) -> RepoOf {
        self.repo_of
    }

    /// Each workspace with its folders; a workspace whose folders changed is bound again.
    pub fn set_workspaces(&mut self, workspaces: Vec<(String, Vec<PathBuf>)>, cx: &mut Context<Self>) {
        let names: Vec<String> = workspaces.iter().map(|(w, _)| w.clone()).collect();
        let rebind = self.rebind;
        let changed: Vec<String> = workspaces.iter().filter(|(w, f)| rebind || self.folders.get(w) != Some(f)).map(|(w, _)| w.clone()).collect();
        if changed.is_empty() && names == self.workspaces {
            return;
        }
        self.rebind = false;
        self.folders = workspaces.into_iter().collect();
        for w in &changed {
            self.keys.remove(w);
            self.projects.remove(w);
        }
        self.projects.retain(|w, _| names.contains(w));
        self.keys.retain(|w, _| names.contains(w));
        self.inboxes.retain(|w, _| names.contains(w));
        self.failures.retain(|w, _| names.contains(w));
        self.workspaces = names;
        let save = self.dismissed.edit(|d| d.retain_workspaces(&self.workspaces));
        Self::save(save, cx);
        let asks = changed.into_iter().map(|w| self.ask(w)).collect();
        self.read(asks, true, cx);
        cx.notify();
    }

    fn ask(&self, workspace: String) -> Ask {
        Ask {
            folders: self.folders.get(&workspace).cloned().unwrap_or_default(),
            keys: self.keys.get(&workspace).cloned(),
            projects: self.projects.get(&workspace).cloned(),
            workspace,
        }
    }

    /// Needs you: the list back at its top and highlighted for [`FLASH`].
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.flash = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH).await;
            this.update(cx, |this, cx| {
                this.flash = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn flashing(&self) -> bool {
        self.flash.is_some()
    }

    pub fn rows(&self) -> Vec<(&str, &InboxRow)> {
        self.workspaces
            .iter()
            .filter_map(|w| self.inboxes.get(w).map(|inbox| (w, inbox)))
            .flat_map(|(w, inbox)| inbox.rows.iter().filter(|r| r.kind != cards::NOTE && !self.dismissed.get().hides(w, r)).map(move |r| (w.as_str(), r)))
            .collect()
    }

    pub fn dismiss(&mut self, workspace: &str, key: &str, cx: &mut Context<Self>) {
        let Some(row) = self.inboxes.get(workspace).and_then(|i| i.rows.iter().find(|r| r.key == key)) else { return };
        let save = self.dismissed.edit(|d| {
            d.dismiss(workspace, row);
            true
        });
        Self::save(save, cx);
        cx.notify();
    }

    /// The first project `folder`'s repo binds (`projects::bound`); git, and Workman when no overview is kept, run off the UI thread.
    pub fn bind_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) -> Task<Binding> {
        let client = match self.hub.read(cx).access() {
            Access::Ready(client) => client.clone(),
            Access::Starting => return Task::ready(Binding::Unavailable("Connecting to Workman…".into())),
            Access::Unconfigured(why) => return Task::ready(Binding::Unavailable(why.to_string())),
        };
        let (kept, repo_of) = (self.overview.clone(), self.repo_of);
        cx.spawn(async move |this, cx| {
            let (binding, fresh) = cx.background_executor().spawn(async move { bind_folder(&client, folder, kept, repo_of) }).await;
            if let Some(fresh) = fresh {
                this.update(cx, |this, _| {
                    this.overview.get_or_insert(fresh);
                })
                .ok();
            }
            binding
        })
    }

    pub fn toggle_housekeeping(&mut self, cx: &mut Context<Self>) {
        self.housekeeping_open = !self.housekeeping_open;
        cx.notify();
    }

    fn save(save: Option<Save>, cx: &mut Context<Self>) {
        supraman_ui::store::save_off_thread(save, cx, |this, saved, cx| {
            this.notice = saved.err().map(|e| format!("Cannot save the dismissed rows ({e}): a dismissed row shows again after a restart"));
            cx.notify();
        });
    }

    fn on_hub(&mut self, _: Entity<WorkHub>, event: &HubEvent, cx: &mut Context<Self>) {
        match event {
            HubEvent::Configured => match self.hub.read(cx).access() {
                Access::Ready(_) => {
                    self.projects.clear();
                    self.read_all(true, cx);
                }
                _ => {
                    self.projects.clear();
                    self.overview = None;
                    self.inboxes.clear();
                    self.failures.clear();
                    cx.notify();
                }
            },
            HubEvent::Resync => self.read_all(true, cx),
            HubEvent::Changed(event) => {
                let first = self.pending.is_none();
                let touched = touched(event, &self.run_project, &self.projects);
                let plan = self.pending.get_or_insert_default();
                match touched {
                    Touched::Workspaces(workspaces) => plan.workspaces.extend(workspaces),
                    Touched::All { overview } => {
                        plan.all = true;
                        plan.overview |= overview;
                    }
                }
                if first {
                    after_debounce(cx, Self::flush);
                }
            }
            HubEvent::Connection => {}
        }
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.pending.take() else { return };
        if plan.all {
            self.read_all(plan.overview, cx);
            return;
        }
        let asks = self
            .workspaces
            .iter()
            .filter(|w| plan.workspaces.contains(*w))
            .map(|w| self.ask(w.clone()))
            .collect();
        self.read(asks, false, cx);
    }

    fn read_all(&mut self, overview: bool, cx: &mut Context<Self>) {
        let asks = self.workspaces.iter().map(|w| self.ask(w.clone())).collect();
        self.read(asks, overview, cx);
    }

    fn read(&mut self, asks: Vec<Ask>, overview: bool, cx: &mut Context<Self>) {
        let Access::Ready(client) = self.hub.read(cx).access() else { return };
        if asks.is_empty() && !overview {
            return;
        }
        let (client, repo_of) = (client.clone(), self.repo_of);
        self.reads += 1;
        let began = std::time::Instant::now();
        cx.spawn(async move |this, cx| {
            let (overview, answers) = cx.background_executor().spawn(async move { read(&client, asks, overview, repo_of) }).await;
            this.update(cx, |this, cx| {
                if this.applied == 0 {
                    supraman_ui::timeline("workman inbox done", began.elapsed());
                }
                this.applied += 1;
                this.apply(overview, answers, cx);
            })
            .ok();
        })
        .detach();
    }

    fn apply(&mut self, overview: Option<Overview>, answers: Vec<Answer>, cx: &mut Context<Self>) {
        if let Some(overview) = overview {
            self.run_project = overview
                .projects
                .iter()
                .flat_map(|p| p.runs.iter().map(|r| (r.item.id.clone(), p.item.id.clone())))
                .collect();
            self.overview = Some(Arc::new(overview));
        }
        let mut save = None;
        for Answer { workspace, keys, outcome } in answers {
            if !self.workspaces.contains(&workspace) {
                continue;
            }
            if let Some(keys) = keys {
                self.keys.insert(workspace.clone(), keys);
            }
            match outcome {
                Outcome::Read(projects, inbox) => {
                    self.failures.remove(&workspace);
                    self.projects.insert(workspace.clone(), projects);
                    if self.inboxes.get(&workspace).is_some_and(|shown| inbox.seq < shown.seq) {
                        continue;
                    }
                    save = self.dismissed.edit(|d| d.prune(&workspace, &inbox.rows)).or(save);
                    self.inboxes.insert(workspace, inbox);
                }
                Outcome::NoProject => {
                    self.failures.remove(&workspace);
                    self.projects.remove(&workspace);
                    self.inboxes.remove(&workspace);
                }
                Outcome::Failed(why) => {
                    self.failures.insert(workspace, why);
                }
            }
        }
        Self::save(save, cx);
        cx.notify();
    }
}

/// Where a folder stands in Workman ([`WorkInbox::bind_folder`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Project(ItemRef),
    Unbound,
    /// Workman's own words: not set up, not signed in, unreachable.
    Unavailable(String),
}

fn bind_folder(client: &Client, folder: PathBuf, kept: Option<Arc<Overview>>, repo_of: RepoOf) -> (Binding, Option<Arc<Overview>>) {
    let keys = projects::folder_keys(&[folder], repo_of);
    if keys.is_empty() {
        return (Binding::Unbound, None);
    }
    let (overview, fresh) = match kept {
        Some(kept) => (kept, None),
        None => match client.overview() {
            Ok(read) => {
                let read = Arc::new(read);
                (read.clone(), Some(read))
            }
            Err(e) => return (Binding::Unavailable(read_failed(e)), None),
        },
    };
    let binding = projects::bound(&overview, &keys).into_iter().next().map_or(Binding::Unbound, |p| Binding::Project(p.item));
    (binding, fresh)
}

/// The key the Inbox paints a Workman row under; its × is [`dismiss_key_of`].
pub fn row_key_of(key: &str) -> String {
    format!("inbox-ws-{key}")
}

pub fn dismiss_key_of(key: &str) -> String {
    format!("inbox-ws-dismiss-{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, since: &str) -> InboxRow {
        InboxRow { key: key.into(), kind: key.split(':').next().unwrap_or("").into(), since: since.into(), ..Default::default() }
    }

    #[test]
    fn a_dismissal_holds_while_the_row_is_unchanged() {
        let mut d = Dismissed::default();
        let blocked = row("blocked:t1", "2026-10-26T10:00:00+00:00");
        d.dismiss("w", &blocked);
        assert!(d.hides("w", &blocked));
        assert!(!d.hides("other", &blocked), "dismissals are per workspace");
        assert!(!d.hides("w", &row("done_unverified:t1", "2026-10-26T10:00:00+00:00")));
        assert!(!d.hides("w", &row("blocked:t1", "2026-10-26T11:00:00+00:00")));
        assert!(!d.prune("w", std::slice::from_ref(&blocked)));
        assert!(d.hides("w", &blocked));
    }

    #[test]
    fn a_row_seen_gone_is_new_when_it_comes_back() {
        let mut d = Dismissed::default();
        let blocked = row("blocked:t1", "s1");
        d.dismiss("w", &blocked);
        assert!(d.prune("w", &[row("question:t2", "s2")]));
        assert!(!d.hides("w", &blocked));
        assert_eq!(d, Dismissed::default(), "an emptied workspace leaves nothing behind");
        d.dismiss("w", &blocked);
        assert!(d.prune("w", &[row("blocked:t1", "s9")]), "started over");
    }

    #[test]
    fn dismissals_round_trip_through_the_file() {
        let home = tempfile::tempdir().unwrap();
        let mut d = Dismissed::default();
        d.dismiss("w", &row("blocked:t1", "s1"));
        d.dismiss("v", &row("question:t2", "s2"));
        std::fs::write(home.path().join(DISMISSED_FILE), serde_json::to_string_pretty(&d).unwrap()).unwrap();
        let (read, notice) = Dismissed::load(Some(home.path()));
        assert_eq!((read.get(), notice), (&d, None));
        std::fs::write(home.path().join(DISMISSED_FILE), "not json").unwrap();
        let (none, notice) = Dismissed::load(Some(home.path()));
        assert_eq!(none.get(), &Dismissed::default());
        assert!(notice.is_some_and(|n| n.contains(DISMISSED_FILE)), "an unreadable file says so");
        let (none, notice) = Dismissed::load(None);
        assert_eq!((none.get(), notice), (&Dismissed::default(), None));
        assert!(d.retain_workspaces(&["w".to_string()]));
        assert!(!d.hides("v", &row("question:t2", "s2")));
    }

    #[test]
    fn an_event_re_reads_the_inboxes_of_its_runs_project() {
        let event = |run: Option<&str>| StreamEvent { seq: 1, run_id: run.map(str::to_string), item_id: None, kind: "transition".into() };
        let runs: HashMap<String, String> = [("r1".to_string(), "p1".to_string()), ("r9".to_string(), "p9".to_string())].into();
        let project = |id: &str| ItemRef { id: id.into(), ..Default::default() };
        let projects: HashMap<String, Vec<ItemRef>> = [("w1".to_string(), vec![project("p0"), project("p1")]), ("w2".to_string(), vec![project("p2")])].into();
        assert_eq!(touched(&event(Some("r1")), &runs, &projects), Touched::Workspaces(vec!["w1".into()]));
        assert_eq!(touched(&event(Some("r9")), &runs, &projects), Touched::Workspaces(vec![]), "an unregistered project's run");
        assert_eq!(touched(&event(Some("new")), &runs, &projects), Touched::All { overview: true });
        assert_eq!(touched(&event(None), &runs, &projects), Touched::All { overview: false });
    }

    #[test]
    fn a_rows_path_url_is_on_the_workman_server() {
        let mut inbox = Inbox::default();
        for url in [Some("/r/ws/sw2/2"), Some("https://w.example/r/x"), None] {
            inbox.rows.push(InboxRow { url: url.map(str::to_string), ..Default::default() });
        }
        let urls: Vec<_> = absolute_urls(inbox, "http://127.0.0.1:9/").rows.into_iter().map(|r| r.url).collect();
        assert_eq!(urls, [Some("http://127.0.0.1:9/r/ws/sw2/2".to_string()), Some("https://w.example/r/x".to_string()), None]);
    }

    #[test]
    fn keys_and_labels() {
        let r = row("blocker_finding:f1", "s");
        assert_eq!(row_key_of(&r.key), "inbox-ws-blocker_finding:f1");
        assert_eq!(dismiss_key_of(&r.key), "inbox-ws-dismiss-blocker_finding:f1");
        assert_eq!(super::super::cards::badge("stale_in_progress"), "stalled");
        assert_eq!(super::super::cards::badge("blocker_finding"), "blocker");
    }
}

//! Supraman writes nothing to Workman (C21): dismissals are local ([`Dismissed`]).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use supraman_core::paths::path_key;
use supraman_core::store::{JsonStore, Save};
use gpui::Task;
use supraman_workman::{Inbox, InboxRow, Overview};

use super::*;
use super::cards::{self, Row};

pub const DISMISSED_FILE: &str = "workstate-dismissed.json";
/// The Inbox's Housekeeping header; painted as `<key>=open` or `<key>=closed`.
pub const HOUSEKEEPING_KEY: &str = "inbox-housekeeping";

/// Per project id: row key (`kind:item`) → the row's `since` when dismissed; see [`Dismissed::prune`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Dismissed(BTreeMap<String, BTreeMap<String, String>>);

impl Dismissed {
    pub(crate) fn load(home: Option<&Path>) -> (JsonStore<Self>, Option<String>) {
        JsonStore::load(home.map(|h| h.join(DISMISSED_FILE)), "every Workman inbox row shows")
    }

    pub(crate) fn hides(&self, project: &str, row: &InboxRow) -> bool {
        self.0.get(project).and_then(|keys| keys.get(&row.key)) == Some(&row.since)
    }

    pub(crate) fn dismiss(&mut self, project: &str, row: &InboxRow) {
        self.0.entry(project.to_string()).or_default().insert(row.key.clone(), row.since.clone());
    }

    /// Only after a successful read of `project`'s inbox.
    pub(crate) fn prune(&mut self, project: &str, rows: &[&InboxRow]) -> bool {
        let Some(keys) = self.0.get_mut(project) else { return false };
        let before = keys.len();
        keys.retain(|key, since| rows.iter().any(|r| &r.key == key && &r.since == since));
        let changed = keys.len() != before;
        if keys.is_empty() {
            self.0.remove(project);
        }
        changed
    }

    fn absorb(&mut self, other: Dismissed) -> bool {
        let mut changed = false;
        for (project, keys) in other.0 {
            let mine = self.0.entry(project).or_default();
            for (key, since) in keys {
                changed |= mine.insert(key, since.clone()).as_ref() != Some(&since);
            }
        }
        changed
    }

    /// Keeps only the projects some target binds; an earlier file's other keys go too.
    pub(crate) fn retain_projects(&mut self, projects: &HashSet<&str>) -> bool {
        let before = self.0.len();
        self.0.retain(|p, _| projects.contains(p.as_str()));
        self.0.len() != before
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Touched {
    Targets(Vec<Target>),
    /// No run, or a run the last overview did not list: every inbox, and the overview again.
    All { overview: bool },
}

pub(crate) fn touched(event: &StreamEvent, run_project: &HashMap<String, String>, projects: &HashMap<Target, Vec<ItemRef>>) -> Touched {
    match event.run_id.as_deref() {
        None => Touched::All { overview: false },
        Some(run) => match run_project.get(run) {
            None => Touched::All { overview: true },
            Some(project) => {
                let mut hit: Vec<Target> = projects.iter().filter(|(_, ps)| ps.iter().any(|p| &p.id == project)).map(|(t, _)| t.clone()).collect();
                hit.sort_by_key(Target::key);
                Touched::Targets(hit)
            }
        },
    }
}

#[derive(Default)]
struct Plan {
    all: bool,
    overview: bool,
    targets: HashSet<Target>,
}

type Keys = Vec<(&'static str, String)>;

struct Ask {
    target: Target,
    folders: Vec<PathBuf>,
    projects: Option<Vec<ItemRef>>,
}

/// One project's inbox rows, the project with each.
#[derive(Debug, Clone, Default)]
struct Merged {
    seq: i64,
    rows: Vec<(ItemRef, InboxRow)>,
}

enum Outcome {
    Read(Vec<ItemRef>, Merged),
    NoProject,
    Failed(String),
}

struct Answer {
    target: Target,
    outcome: Outcome,
}

/// Each folder's repo keys by `path_key`: the ones kept, and those of `missing` read now (git, so off the UI thread).
fn keys_of(kept: &HashMap<String, Keys>, missing: &[PathBuf], repo_of: RepoOf) -> HashMap<String, Keys> {
    let mut all = kept.clone();
    for folder in missing {
        all.insert(path_key(folder), projects::folder_keys(std::slice::from_ref(folder), repo_of));
    }
    all
}

fn union(folders: &[PathBuf], keys: &HashMap<String, Keys>) -> Keys {
    let mut out = Keys::new();
    for key in folders.iter().filter_map(|f| keys.get(&path_key(f))).flatten() {
        if !out.contains(key) {
            out.push(key.clone());
        }
    }
    out
}

/// One inbox of every project bound to the target's repos (`projects::bound`), newest first; a fresh overview binds again.
fn read(client: &Client, asks: Vec<Ask>, overview: bool, keys: &HashMap<String, Keys>) -> (Option<Overview>, Vec<Answer>) {
    let needs_overview = overview || asks.iter().any(|a| a.projects.is_none());
    let overview = if needs_overview { client.overview().map_err(|e| e.to_string()) } else { Err(String::new()) };
    let answers = asks
        .into_iter()
        .map(|ask| {
            let projects = match (&overview, ask.projects) {
                (Ok(overview), _) => Ok(projects::bound(overview, &union(&ask.folders, keys)).into_iter().map(|p| p.item).collect::<Vec<_>>()),
                (Err(_), Some(projects)) => Ok(projects),
                (Err(why), None) => Err(why.clone()),
            };
            let outcome = match projects {
                Ok(projects) if projects.is_empty() => Outcome::NoProject,
                Ok(projects) => merged(client, projects),
                Err(why) => Outcome::Failed(why),
            };
            Answer { target: ask.target, outcome }
        })
        .collect();
    (overview.ok(), answers)
}

fn merged(client: &Client, projects: Vec<ItemRef>) -> Outcome {
    let mut all = Merged::default();
    for project in &projects {
        match client.inbox(Some(&project.id)) {
            Ok(inbox) => {
                all.seq = all.seq.max(inbox.seq);
                let inbox = absolute_urls(inbox, client.base());
                let fresh: Vec<InboxRow> = inbox.rows.into_iter().filter(|r| !all.rows.iter().any(|(_, seen)| seen.key == r.key)).collect();
                all.rows.extend(fresh.into_iter().map(|r| (project.clone(), r)));
            }
            Err(e) => return Outcome::Failed(e.to_string()),
        }
    }
    all.rows.sort_by_key(|(_, r)| std::cmp::Reverse(supraman_core::time::rfc3339_ms(&r.since)));
    Outcome::Read(projects, all)
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
    targets: Vec<Target>,
    folders: HashMap<Target, Vec<PathBuf>>,
    /// Every folder row a Work tab may open: their repo keys are read for [`WorkInbox::folder_binding`].
    rows: Vec<PathBuf>,
    keys: HashMap<String, Keys>,
    repo_of: RepoOf,
    rebind: bool,
    projects: HashMap<Target, Vec<ItemRef>>,
    inboxes: HashMap<Target, Merged>,
    /// Run id → project id, from the last overview.
    run_project: HashMap<String, String>,
    overview: Option<Arc<Overview>>,
    pub(crate) dismissed: JsonStore<Dismissed>,
    pending: Option<Plan>,
    pub(crate) failures: HashMap<Target, String>,
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

    /// An inbox of no target finding repos with `repo_of`: a test's.
    #[cfg(test)]
    pub(crate) fn of_repos(hub: Entity<WorkHub>, repo_of: RepoOf, cx: &mut App) -> Entity<WorkInbox> {
        cx.new(|cx| WorkInbox { repo_of, ..Self::new(hub, None, cx) })
    }

    fn new(hub: Entity<WorkHub>, home: Option<&Path>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&hub, Self::on_hub).detach();
        if let Some(home) = home.map(Path::to_path_buf) {
            cx.spawn(async move |this, cx| {
                let loaded = cx.background_spawn(async move { Dismissed::load(Some(&home)) }).await;
                this.update(cx, |this, cx| this.dismissed_loaded(loaded, cx)).ok();
            })
            .detach();
        }
        WorkInbox {
            hub,
            targets: Vec::new(),
            folders: HashMap::new(),
            rows: Vec::new(),
            keys: HashMap::new(),
            repo_of: supraman_supervisor::git_repo,
            rebind: false,
            projects: HashMap::new(),
            inboxes: HashMap::new(),
            run_project: HashMap::new(),
            overview: None,
            dismissed: JsonStore::default(),
            pending: None,
            failures: HashMap::new(),
            notice: None,
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

    /// Each target with its folders, and every folder row; a target whose folders changed is bound again.
    pub fn set_targets(&mut self, targets: Vec<(Target, Vec<PathBuf>)>, rows: Vec<PathBuf>, cx: &mut Context<Self>) {
        let listed: Vec<Target> = targets.iter().map(|(t, _)| t.clone()).collect();
        let rebind = self.rebind;
        let changed: Vec<Target> = targets.iter().filter(|(t, f)| rebind || self.folders.get(t) != Some(f)).map(|(t, _)| t.clone()).collect();
        if changed.is_empty() && listed == self.targets && rows == self.rows {
            return;
        }
        if rebind {
            self.keys.clear();
        }
        self.rebind = false;
        for t in &changed {
            for folder in self.folders.get(t).into_iter().flatten() {
                self.keys.remove(&path_key(folder));
            }
            self.projects.remove(t);
        }
        self.folders = targets.into_iter().collect();
        self.rows = rows;
        self.projects.retain(|t, _| listed.contains(t));
        self.inboxes.retain(|t, _| listed.contains(t));
        self.failures.retain(|t, _| listed.contains(t));
        self.targets = listed;
        let asks = changed.into_iter().map(|t| self.ask(t)).collect();
        self.read(asks, true, cx);
        cx.notify();
    }

    fn dismissed_loaded(&mut self, (store, notice): (JsonStore<Dismissed>, Option<String>), cx: &mut Context<Self>) {
        let early = std::mem::replace(&mut self.dismissed, store).get().clone();
        let save = self.dismissed.edit(|d| d.absorb(early));
        Self::save(save, cx);
        self.notice = notice.or(self.notice.take());
        cx.notify();
    }

    fn ask(&self, target: Target) -> Ask {
        let projects = self.projects.get(&target).filter(|p| !p.is_empty()).cloned();
        Ask { folders: self.folders.get(&target).cloned().unwrap_or_default(), projects, target }
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

    /// Every row shown, once per project, with the target first binding it; its context the project's name, else the target's.
    pub fn rows(&self) -> Vec<Row> {
        let mut out: Vec<Row> = Vec::new();
        for target in &self.targets {
            let Some(inbox) = self.inboxes.get(target) else { continue };
            for (project, row) in &inbox.rows {
                if row.kind == cards::NOTE || self.dismissed.get().hides(&project.id, row) {
                    continue;
                }
                if out.iter().any(|r| r.project == project.id && r.row.key == row.key) {
                    continue;
                }
                let context = if project.title.is_empty() { target.name() } else { project.title.clone() };
                out.push(Row { target: target.clone(), project: project.id.clone(), context, row: row.clone() });
            }
        }
        out
    }

    pub fn dismiss(&mut self, project: &str, key: &str, cx: &mut Context<Self>) {
        let found = self.inboxes.values().flat_map(|i| &i.rows).find(|(p, r)| p.id == project && r.key == key);
        let Some((_, row)) = found else { return };
        let row = row.clone();
        let save = self.dismissed.edit(|d| {
            d.dismiss(project, &row);
            true
        });
        Self::save(save, cx);
        cx.notify();
    }

    /// The projects a folder row's repo binds, from the kept overview and the keys read for it; `None` until both are,
    /// `Err` with Workman's words.
    pub fn folder_binding(&self, folder: &Path, cx: &App) -> Option<Result<Vec<ItemRef>, String>> {
        match self.hub.read(cx).access() {
            Access::Ready(_) => {}
            Access::Starting => return None,
            Access::Unconfigured(why) => return Some(Err(why.to_string())),
        }
        if let Some(why) = self.overview.is_none().then(|| self.failures.values().next()).flatten() {
            return Some(Err(why.clone()));
        }
        let keys = self.keys.get(&path_key(folder))?;
        let overview = self.overview.as_ref()?;
        Some(Ok(projects::bound(overview, keys).into_iter().map(|p| p.item).collect()))
    }

    /// The projects the Inbox last bound to `target`, when it has read them.
    pub fn target_projects(&self, target: &Target) -> Option<&[ItemRef]> {
        self.projects.get(target).map(Vec::as_slice)
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

    /// A save of rows Supraman pruned itself: a failure leaves them in the file, harmless, so it is logged.
    fn save_pruned(save: Option<Save>, cx: &mut Context<Self>) {
        supraman_ui::store::save_off_thread(save, cx, |this, saved, cx| match saved {
            Ok(()) => {
                this.notice = None;
                cx.notify();
            }
            Err(e) => log::warn!("Cannot save the dismissed rows ({e}): a dismissed row shows again after a restart"),
        });
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
                    Touched::Targets(targets) => plan.targets.extend(targets),
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
        let asks = self.targets.iter().filter(|t| plan.targets.contains(*t)).map(|t| self.ask(t.clone())).collect();
        self.read(asks, false, cx);
    }

    fn read_all(&mut self, overview: bool, cx: &mut Context<Self>) {
        let asks = self.targets.iter().map(|t| self.ask(t.clone())).collect();
        self.read(asks, overview, cx);
    }

    fn read(&mut self, asks: Vec<Ask>, overview: bool, cx: &mut Context<Self>) {
        let Access::Ready(client) = self.hub.read(cx).access() else { return };
        if asks.is_empty() && !overview {
            return;
        }
        let missing: Vec<PathBuf> = {
            let mut seen = HashSet::new();
            asks.iter().flat_map(|a| a.folders.iter()).chain(&self.rows).filter(|f| !self.keys.contains_key(&path_key(f)) && seen.insert(path_key(f))).cloned().collect()
        };
        let (client, repo_of, kept) = (client.clone(), self.repo_of, self.keys.clone());
        self.reads += 1;
        let began = std::time::Instant::now();
        cx.spawn(async move |this, cx| {
            let (overview, answers, keys) = cx
                .background_executor()
                .spawn(async move {
                    let keys = keys_of(&kept, &missing, repo_of);
                    let (overview, answers) = read(&client, asks, overview, &keys);
                    (overview, answers, keys)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.applied == 0 {
                    supraman_ui::timeline("workman inbox done", began.elapsed());
                }
                this.applied += 1;
                this.keys.extend(keys);
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
        for Answer { target, outcome } in answers {
            if !self.targets.contains(&target) {
                continue;
            }
            match outcome {
                Outcome::Read(projects, inbox) => {
                    self.failures.remove(&target);
                    if self.inboxes.get(&target).is_some_and(|shown| inbox.seq < shown.seq) {
                        self.projects.insert(target, projects);
                        continue;
                    }
                    for project in &projects {
                        let rows: Vec<&InboxRow> = inbox.rows.iter().filter(|(p, _)| p.id == project.id).map(|(_, r)| r).collect();
                        save = self.dismissed.edit(|d| d.prune(&project.id, &rows)).or(save);
                    }
                    self.projects.insert(target.clone(), projects);
                    self.inboxes.insert(target, inbox);
                }
                Outcome::NoProject => {
                    self.failures.remove(&target);
                    self.projects.insert(target.clone(), Vec::new());
                    self.inboxes.remove(&target);
                }
                Outcome::Failed(why) => {
                    self.failures.insert(target, why);
                }
            }
        }
        if self.targets.iter().all(|t| self.projects.contains_key(t)) {
            let bound: HashSet<&str> = self.projects.values().flatten().map(|p| p.id.as_str()).collect();
            save = self.dismissed.edit(|d| d.retain_projects(&bound)).or(save);
        }
        Self::save_pruned(save, cx);
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
        d.dismiss("p", &blocked);
        assert!(d.hides("p", &blocked));
        assert!(!d.hides("other", &blocked), "dismissals are per project");
        assert!(!d.hides("p", &row("done_unverified:t1", "2026-10-26T10:00:00+00:00")));
        assert!(!d.hides("p", &row("blocked:t1", "2026-10-26T11:00:00+00:00")));
        assert!(!d.prune("p", &[&blocked]));
        assert!(d.hides("p", &blocked));
    }

    #[test]
    fn a_row_seen_gone_is_new_when_it_comes_back() {
        let mut d = Dismissed::default();
        let blocked = row("blocked:t1", "s1");
        d.dismiss("p", &blocked);
        assert!(d.prune("p", &[&row("question:t2", "s2")]));
        assert!(!d.hides("p", &blocked));
        assert_eq!(d, Dismissed::default(), "an emptied project leaves nothing behind");
        d.dismiss("p", &blocked);
        assert!(d.prune("p", &[&row("blocked:t1", "s9")]), "started over");
    }

    #[test]
    fn a_dismissal_made_before_the_file_is_read_is_kept_with_the_files() {
        let mut read = Dismissed::default();
        read.dismiss("p", &row("blocked:t1", "s1"));
        let mut early = Dismissed::default();
        early.dismiss("p", &row("question:t2", "s2"));
        assert!(read.absorb(early.clone()));
        assert!(read.hides("p", &row("blocked:t1", "s1")) && read.hides("p", &row("question:t2", "s2")));
        assert!(!read.absorb(early), "nothing new the second time");
    }

    #[test]
    fn dismissals_round_trip_through_the_file_and_keep_only_bound_projects() {
        let home = tempfile::tempdir().unwrap();
        let mut d = Dismissed::default();
        d.dismiss("p", &row("blocked:t1", "s1"));
        d.dismiss("q", &row("question:t2", "s2"));
        std::fs::write(home.path().join(DISMISSED_FILE), serde_json::to_string_pretty(&d).unwrap()).unwrap();
        let (read, notice) = Dismissed::load(Some(home.path()));
        assert_eq!((read.get(), notice), (&d, None));
        std::fs::write(home.path().join(DISMISSED_FILE), "not json").unwrap();
        let (none, notice) = Dismissed::load(Some(home.path()));
        assert_eq!(none.get(), &Dismissed::default());
        assert!(notice.is_some_and(|n| n.contains(DISMISSED_FILE)), "an unreadable file says so");
        let (none, notice) = Dismissed::load(None);
        assert_eq!((none.get(), notice), (&Dismissed::default(), None));
        assert!(d.retain_projects(&["p"].into()));
        assert!(!d.hides("q", &row("question:t2", "s2")));
        assert!(d.hides("p", &row("blocked:t1", "s1")));
    }

    #[test]
    fn an_event_re_reads_the_inboxes_of_its_runs_project() {
        let event = |run: Option<&str>| StreamEvent { seq: 1, run_id: run.map(str::to_string), item_id: None, kind: "transition".into() };
        let runs: HashMap<String, String> = [("r1".to_string(), "p1".to_string()), ("r9".to_string(), "p9".to_string())].into();
        let project = |id: &str| ItemRef { id: id.into(), ..Default::default() };
        let (group, folder) = (Target::Group("g".into()), Target::Folder("C:/f".into()));
        let projects: HashMap<Target, Vec<ItemRef>> = [(group.clone(), vec![project("p0"), project("p1")]), (folder, vec![project("p2")])].into();
        assert_eq!(touched(&event(Some("r1")), &runs, &projects), Touched::Targets(vec![group]));
        assert_eq!(touched(&event(Some("r9")), &runs, &projects), Touched::Targets(vec![]), "an unregistered project's run");
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

//! Foreman writes nothing to WorkState (C21): dismissals are local ([`Dismissed`]).

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use foreman_core::fs::{load_json_or_notice, write_atomic};
use foreman_workstate::{Inbox, InboxRow, Overview};

use super::*;

pub const DISMISSED_FILE: &str = "workstate-dismissed.json";

/// Per workspace: row key (`kind:item`) → the row's `since` when dismissed; see [`Dismissed::prune`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dismissed(BTreeMap<String, BTreeMap<String, String>>);

impl Dismissed {
    pub fn load(home: Option<&Path>) -> (Self, Option<String>) {
        let Some(home) = home else { return (Self::default(), None) };
        let (dismissed, notice) = load_json_or_notice(&home.join(DISMISSED_FILE), "every WorkState inbox row shows");
        (Dismissed(dismissed), notice)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.0).unwrap_or_else(|_| "{}".into())
    }

    pub fn hides(&self, workspace: &str, row: &InboxRow) -> bool {
        self.0.get(workspace).and_then(|keys| keys.get(&row.key)) == Some(&row.since)
    }

    pub fn dismiss(&mut self, workspace: &str, row: &InboxRow) {
        self.0.entry(workspace.to_string()).or_default().insert(row.key.clone(), row.since.clone());
    }

    /// Only after a successful read of `workspace`'s inbox.
    pub fn prune(&mut self, workspace: &str, rows: &[InboxRow]) -> bool {
        let Some(keys) = self.0.get_mut(workspace) else { return false };
        let before = keys.len();
        keys.retain(|key, since| rows.iter().any(|r| &r.key == key && &r.since == since));
        let changed = keys.len() != before;
        if keys.is_empty() {
            self.0.remove(workspace);
        }
        changed
    }

    pub fn retain_workspaces(&mut self, workspaces: &[String]) -> bool {
        let before = self.0.len();
        self.0.retain(|w, _| workspaces.contains(w));
        self.0.len() != before
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Touched {
    Workspaces(Vec<String>),
    /// No run, or a run the last overview did not list: every inbox, and the overview again.
    All { overview: bool },
}

pub fn touched(event: &StreamEvent, run_project: &HashMap<String, String>, projects: &HashMap<String, ItemRef>) -> Touched {
    match event.run_id.as_deref() {
        None => Touched::All { overview: false },
        Some(run) => match run_project.get(run) {
            None => Touched::All { overview: true },
            Some(project) => {
                let mut hit: Vec<String> = projects.iter().filter(|(_, p)| &p.id == project).map(|(w, _)| w.clone()).collect();
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

struct Ask {
    workspace: String,
    project: Option<ItemRef>,
}

enum Outcome {
    Read(ItemRef, Inbox),
    NoProject,
    Failed(String),
}

fn read(client: &Client, asks: Vec<Ask>, overview: bool) -> (Option<Overview>, Vec<(String, Outcome)>) {
    let overview = if overview { client.overview().ok() } else { None };
    let outcomes = asks
        .into_iter()
        .map(|ask| {
            let project = match ask.project {
                Some(project) => Ok(project),
                None => match client.resolve(&ask.workspace) {
                    Ok(resolved) => pick_project(resolved, &ask.workspace).map_err(|_| Outcome::NoProject),
                    Err(e) => Err(Outcome::Failed(e.to_string())),
                },
            };
            let outcome = match project {
                Ok(project) => match client.inbox(Some(&project.id)) {
                    Ok(inbox) => Outcome::Read(project, inbox),
                    Err(e) => Outcome::Failed(e.to_string()),
                },
                Err(outcome) => outcome,
            };
            (ask.workspace, outcome)
        })
        .collect();
    (overview, outcomes)
}

pub struct WorkInbox {
    hub: Entity<WorkHub>,
    home: Option<PathBuf>,
    workspaces: Vec<String>,
    projects: HashMap<String, ItemRef>,
    inboxes: HashMap<String, Inbox>,
    /// Run id → project id, from the last overview.
    run_project: HashMap<String, String>,
    pub dismissed: Dismissed,
    /// The newest dismissals not yet on disk; one writer at a time takes them.
    unsaved: Arc<Mutex<Option<String>>>,
    pending: Option<Plan>,
    pub failures: HashMap<String, String>,
    pub notice: Option<String>,
    pub reads: u64,
}

struct GlobalInbox(Entity<WorkInbox>);

impl Global for GlobalInbox {}

impl WorkInbox {
    pub fn shared(home: Option<PathBuf>, cx: &mut App) -> Entity<WorkInbox> {
        if let Some(GlobalInbox(inbox)) = cx.try_global::<GlobalInbox>() {
            return inbox.clone();
        }
        let hub = WorkHub::shared(home.clone(), cx);
        let inbox = cx.new(|cx| {
            cx.subscribe(&hub, Self::on_hub).detach();
            let (dismissed, notice) = Dismissed::load(home.as_deref());
            WorkInbox {
                hub,
                home,
                workspaces: Vec::new(),
                projects: HashMap::new(),
                inboxes: HashMap::new(),
                run_project: HashMap::new(),
                dismissed,
                unsaved: Arc::default(),
                pending: None,
                failures: HashMap::new(),
                notice,
                reads: 0,
            }
        });
        cx.set_global(GlobalInbox(inbox.clone()));
        inbox
    }

    pub fn set_workspaces(&mut self, workspaces: Vec<String>, cx: &mut Context<Self>) {
        if workspaces == self.workspaces {
            return;
        }
        let added: Vec<String> = workspaces.iter().filter(|w| !self.workspaces.contains(w)).cloned().collect();
        self.projects.retain(|w, _| workspaces.contains(w));
        self.inboxes.retain(|w, _| workspaces.contains(w));
        self.failures.retain(|w, _| workspaces.contains(w));
        self.workspaces = workspaces;
        if self.dismissed.retain_workspaces(&self.workspaces) {
            self.save(cx);
        }
        let asks = added.into_iter().map(|workspace| Ask { workspace, project: None }).collect();
        self.read(asks, true, cx);
        cx.notify();
    }

    pub fn rows(&self) -> Vec<(&str, &InboxRow)> {
        self.workspaces
            .iter()
            .filter_map(|w| self.inboxes.get(w).map(|inbox| (w, inbox)))
            .flat_map(|(w, inbox)| inbox.rows.iter().filter(|r| !self.dismissed.hides(w, r)).map(move |r| (w.as_str(), r)))
            .collect()
    }

    pub fn dismiss(&mut self, workspace: &str, key: &str, cx: &mut Context<Self>) {
        let Some(row) = self.inboxes.get(workspace).and_then(|i| i.rows.iter().find(|r| r.key == key)) else { return };
        self.dismissed.dismiss(workspace, row);
        self.save(cx);
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.home.as_ref().map(|h| h.join(DISMISSED_FILE)) else { return };
        *self.unsaved.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.dismissed.to_json());
        let unsaved = self.unsaved.clone();
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move {
                    // Held while writing, so the last state written is the newest one.
                    let mut latest = unsaved.lock().unwrap_or_else(|e| e.into_inner());
                    let json = latest.take()?;
                    Some(write_atomic(&path, json).map_err(|e| format!("Cannot save {} ({e}): a dismissed row shows again after a restart", path.display())))
                })
                .await;
            let Some(saved) = saved else { return };
            this.update(cx, |this, cx| {
                this.notice = saved.err();
                cx.notify();
            })
            .ok();
        })
        .detach();
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
            .map(|w| Ask { workspace: w.clone(), project: self.projects.get(w).cloned() })
            .collect();
        self.read(asks, false, cx);
    }

    fn read_all(&mut self, overview: bool, cx: &mut Context<Self>) {
        let asks = self.workspaces.iter().map(|w| Ask { workspace: w.clone(), project: self.projects.get(w).cloned() }).collect();
        self.read(asks, overview, cx);
    }

    fn read(&mut self, asks: Vec<Ask>, overview: bool, cx: &mut Context<Self>) {
        let Access::Ready(client) = self.hub.read(cx).access() else { return };
        if asks.is_empty() && !overview {
            return;
        }
        let client = client.clone();
        self.reads += 1;
        cx.spawn(async move |this, cx| {
            let (overview, outcomes) = cx.background_executor().spawn(async move { read(&client, asks, overview) }).await;
            this.update(cx, |this, cx| this.apply(overview, outcomes, cx)).ok();
        })
        .detach();
    }

    fn apply(&mut self, overview: Option<Overview>, outcomes: Vec<(String, Outcome)>, cx: &mut Context<Self>) {
        if let Some(overview) = overview {
            self.run_project = overview
                .projects
                .iter()
                .flat_map(|p| p.runs.iter().map(|r| (r.item.id.clone(), p.item.id.clone())))
                .collect();
        }
        let mut pruned = false;
        for (workspace, outcome) in outcomes {
            if !self.workspaces.contains(&workspace) {
                continue;
            }
            match outcome {
                Outcome::Read(project, inbox) => {
                    self.failures.remove(&workspace);
                    self.projects.insert(workspace.clone(), project);
                    if self.inboxes.get(&workspace).is_some_and(|shown| inbox.seq < shown.seq) {
                        continue;
                    }
                    pruned |= self.dismissed.prune(&workspace, &inbox.rows);
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
        if pruned {
            self.save(cx);
        }
        cx.notify();
    }
}

/// The key the Inbox paints a WorkState row under; its × is `inbox-ws-dismiss-<key>`.
pub fn row_key(row: &InboxRow) -> String {
    format!("inbox-ws-{}", row.key)
}

pub fn dismiss_key(row: &InboxRow) -> String {
    format!("inbox-ws-dismiss-{}", row.key)
}

pub fn kind_label(kind: &str) -> String {
    kind.replace('_', " ")
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
        std::fs::write(home.path().join(DISMISSED_FILE), d.to_json()).unwrap();
        assert_eq!(Dismissed::load(Some(home.path())), (d.clone(), None));
        std::fs::write(home.path().join(DISMISSED_FILE), "not json").unwrap();
        let (none, notice) = Dismissed::load(Some(home.path()));
        assert_eq!(none, Dismissed::default());
        assert!(notice.is_some_and(|n| n.contains(DISMISSED_FILE)), "an unreadable file says so");
        assert_eq!(Dismissed::load(None), (Dismissed::default(), None));
        assert!(d.retain_workspaces(&["w".to_string()]));
        assert!(!d.hides("v", &row("question:t2", "s2")));
    }

    #[test]
    fn an_event_re_reads_the_inboxes_of_its_runs_project() {
        let event = |run: Option<&str>| StreamEvent { seq: 1, run_id: run.map(str::to_string), item_id: None, kind: "transition".into() };
        let runs: HashMap<String, String> = [("r1".to_string(), "p1".to_string()), ("r9".to_string(), "p9".to_string())].into();
        let project = |id: &str| ItemRef { id: id.into(), ..Default::default() };
        let projects: HashMap<String, ItemRef> = [("w1".to_string(), project("p1")), ("w2".to_string(), project("p2"))].into();
        assert_eq!(touched(&event(Some("r1")), &runs, &projects), Touched::Workspaces(vec!["w1".into()]));
        assert_eq!(touched(&event(Some("r9")), &runs, &projects), Touched::Workspaces(vec![]), "an unregistered project's run");
        assert_eq!(touched(&event(Some("new")), &runs, &projects), Touched::All { overview: true });
        assert_eq!(touched(&event(None), &runs, &projects), Touched::All { overview: false });
    }

    #[test]
    fn keys_and_labels() {
        let r = row("blocker_finding:f1", "s");
        assert_eq!(row_key(&r), "inbox-ws-blocker_finding:f1");
        assert_eq!(dismiss_key(&r), "inbox-ws-dismiss-blocker_finding:f1");
        assert_eq!(kind_label("stale_in_progress"), "stale in progress");
    }
}

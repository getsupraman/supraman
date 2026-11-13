use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use supraman_claude::{Health, Kind, Session, assign_with, main_repo};
use supraman_core::paths::{normalize, path_key};
use supraman_core::time::ago;
use supraman_core::{Event, FolderKind, Workspace, owners};
use supraman_git_view::occupancy::LiveSession;
use gpui::App;
use supraman_ui::icon::Icon;
use supraman_ui::marks::{Activity, Mark};
use supraman_ui::session::Opening;
use theme::ActiveTheme as _;
use supraman_supervisor::{relay_session_name, repo_session_name, workspace_session_name};

#[derive(Debug, Clone)]
pub struct WorkspaceNode {
    pub name: String,
    pub home: Option<PathBuf>,
    /// `None`: no desired-state file.
    pub started: Option<bool>,
    /// The workspace session, while wanted; the workspace row shows it.
    pub own: Option<SessionRow>,
    /// The relay, while wanted; the workspace row's relay glyph stands for it.
    pub relay: Option<SessionRow>,
    /// Wanted sessions of no folder here.
    pub supervised: Vec<SessionRow>,
    /// Scheduled runs' sessions by schedule id ([`supraman_core::schedule::schedule_of_session`]), shown under it.
    pub scheduled: BTreeMap<String, Vec<SessionRow>>,
    pub repos: Vec<RepoNode>,
}

#[derive(Debug, Clone)]
pub struct RepoNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: FolderKind,
    pub branch: Option<String>,
    pub enabled: bool,
    /// False: a folder of the home that holds sessions without being one of the workspace's folders.
    pub listed: bool,
    /// Another workspace owns the folder's supervised session ([`supraman_core::owners`]).
    pub supervised_in: Option<String>,
    /// The other workspaces that enable this folder, while it is enabled here too.
    pub also_in: Vec<String>,
    pub sessions: Vec<SessionRow>,
    pub more: usize,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session: Session,
    pub health: Health,
    pub attachable: bool,
    pub hook: Option<HookSignal>,
    pub foreign: bool,
    pub starred: bool,
    /// From its transcript: an interactive session `claude agents` does not list.
    pub closed: bool,
    /// The supervising (else first) workspace, on every other occurrence of a session; its keys end in `@<that workspace>`.
    pub also_in: Option<String>,
}

impl SessionRow {
    pub fn new(session: Session, health: Health) -> Self {
        Self {
            attachable: attachable(&session, &health),
            session,
            health,
            hook: None,
            foreign: false,
            starred: false,
            closed: false,
            also_in: None,
        }
    }

    /// `""`, or `@<workspace>` for an occurrence [`Self::also_in`] marks.
    pub fn scope(&self, workspace: &str) -> String {
        if self.also_in.is_some() { format!("@{workspace}") } else { String::new() }
    }
}

pub const CLOSED_WINDOW_MS: i64 = 14 * 24 * 3_600_000;
pub const CLOSED_CAP: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSignal {
    pub last: Event,
    pub compacted_at: Option<i64>,
}

impl HookSignal {
    pub fn record(signals: &mut HashMap<String, HookSignal>, event: Event) {
        let compacted = (event.event == "SessionStart" && event.source.as_deref() == Some("compact")).then_some(event.ts);
        match signals.get_mut(&event.session_id) {
            Some(s) => {
                if compacted.is_some() {
                    s.compacted_at = compacted;
                }
                if event.ts >= s.last.ts {
                    s.last = event;
                }
            }
            None => {
                signals.insert(
                    event.session_id.clone(),
                    HookSignal {
                        last: event,
                        compacted_at: compacted,
                    },
                );
            }
        }
    }

    pub fn is_permission_prompt(&self) -> bool {
        self.last.event == "Notification" && self.last.notification_type.as_deref() == Some("permission_prompt")
    }

    pub fn label(&self, now_ms: i64) -> String {
        let e = &self.last;
        let what = match (e.event.as_str(), e.source.as_deref(), e.notification_type.as_deref()) {
            ("SessionStart", Some("startup"), _) => "started".to_string(),
            ("SessionStart", Some("resume"), _) => "resumed".to_string(),
            ("SessionStart", Some("compact"), _) => "compacted".to_string(),
            ("SessionStart", Some(other), _) => format!("session start ({other})"),
            ("UserPromptSubmit", ..) => "prompt submitted".to_string(),
            ("Stop", ..) => "turn ended".to_string(),
            ("PreCompact", ..) => "compacting".to_string(),
            ("Notification", _, Some(kind)) => kind.replace('_', " "),
            (other, ..) => other.to_string(),
        };
        let mut text = format!("{what} {}", ago(now_ms - e.ts));
        if let Some(t) = self.compacted_at
            && what != "compacted"
        {
            text.push_str(&format!(" · compacted {}", ago(now_ms - t)));
        }
        text
    }
}

/// A session's health as the sidebar, the agent rows and the Work tab show it.
pub fn shown_health(session: &Session, now_ms: i64, hooks: Option<&HashMap<String, HookSignal>>, polled_at: i64) -> Health {
    with_hook(session.supervised_health(now_ms), hooks.and_then(|h| h.get(&session.session_id)), polled_at)
}

/// A prompt the hook reported after the poll marks the row blocked before the next poll shows it.
fn with_hook(health: Health, hook: Option<&HookSignal>, polled_at: i64) -> Health {
    match (health, hook) {
        (Health::Running | Health::Idle, Some(h)) if h.is_permission_prompt() && h.last.ts > polled_at => Health::Blocked {
            waiting_for: Some("permission prompt".into()),
        },
        (health, _) => health,
    }
}

#[derive(Default)]
pub struct Supervised<'a> {
    pub desired: Option<&'a BTreeMap<String, (bool, Vec<String>)>>,
    pub hooks: Option<&'a HashMap<String, HookSignal>>,
    pub polled_at: i64,
    /// `None` walks the filesystem instead.
    pub main_repos: Option<&'a HashMap<PathBuf, Option<PathBuf>>>,
    /// A session `claude agents` lists is shown from there instead.
    pub closed: &'a [Session],
    pub starred: Option<&'a BTreeSet<String>>,
    pub expanded: Option<&'a HashSet<String>>,
    /// Keeps a disabled folder that holds no session (search); the tree leaves it out.
    pub disabled_folders: bool,
    /// A selftest's run prefix: its `fm-selftest-<hex>` sessions are this Supraman's own.
    pub own_prefix: Option<&'a str>,
}

/// Filled once per poll, off the UI thread, because [`build`] runs on every frame.
pub fn main_repos(sessions: &[Session]) -> HashMap<PathBuf, Option<PathBuf>> {
    let mut out = HashMap::new();
    for s in sessions {
        out.entry(s.cwd.clone()).or_insert_with(|| main_repo(&s.cwd));
    }
    out
}

#[derive(Debug)]
pub struct Tree {
    pub workspaces: Vec<WorkspaceNode>,
    pub hidden_stopped: usize,
}

impl WorkspaceNode {
    /// The rows under the workspace row; the workspace session and the relay have none.
    pub fn session_count(&self) -> usize {
        self.supervised.len() + self.scheduled.values().map(Vec::len).sum::<usize>() + self.repos.iter().map(|r| r.sessions.len()).sum::<usize>()
    }

    /// Every session of the workspace, the workspace session and the relay first.
    pub fn rows(&self) -> impl Iterator<Item = &SessionRow> {
        self.own.iter().chain(&self.relay).chain(&self.supervised).chain(self.scheduled.values().flatten()).chain(self.repos.iter().flat_map(|r| &r.sessions))
    }

    fn rows_mut(&mut self) -> impl Iterator<Item = &mut SessionRow> {
        self.own
            .iter_mut()
            .chain(&mut self.relay)
            .chain(&mut self.supervised)
            .chain(self.scheduled.values_mut().flatten())
            .chain(self.repos.iter_mut().flat_map(|r| r.sessions.iter_mut()))
    }

    pub fn into_rows(self) -> impl Iterator<Item = SessionRow> {
        self.own.into_iter().chain(self.relay).chain(self.supervised).chain(self.scheduled.into_values().flatten()).chain(self.repos.into_iter().flat_map(|r| r.sessions))
    }
}

pub fn attachable(session: &Session, health: &Health) -> bool {
    session.kind == Kind::Background
        && matches!(health, Health::Running | Health::Blocked { .. } | Health::Idle)
}

pub fn is_alive(health: &Health) -> bool {
    activity(health).is_some()
}

/// A live session's state as its dot shows it; `None` for a session not alive.
pub fn activity(health: &Health) -> Option<Activity> {
    match health {
        Health::Running => Some(Activity::Working),
        Health::Blocked { .. } => Some(Activity::Waiting),
        Health::Idle => Some(Activity::Idle),
        Health::Stopped | Health::Orphaned { .. } | Health::Unknown => None,
    }
}

/// The live sessions as every tab shows them, by [`display_name`], their attach id while [`attachable`].
pub fn live_sessions(sessions: &[Session], now_ms: i64, hooks: &HashMap<String, HookSignal>, polled_at: i64) -> Vec<LiveSession> {
    sessions
        .iter()
        .filter_map(|s| {
            let health = shown_health(s, now_ms, Some(hooks), polled_at);
            let activity = activity(&health)?;
            Some(LiveSession { name: display_name(s).to_string(), claude_session_id: s.session_id.clone(), cwd: s.cwd.clone(), activity, attach: attachable(s, &health).then(|| s.id.clone()) })
        })
        .collect()
}

pub fn is_foreign(name: &str, workspaces: &[Workspace], desired: Option<&BTreeMap<String, (bool, Vec<String>)>>, own_prefix: Option<&str>) -> bool {
    if own_prefix.is_some_and(|p| name.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))) {
        return false;
    }
    let wanted = desired.into_iter().flat_map(|d| d.values()).flat_map(|(_, wanted)| wanted.iter().map(String::as_str));
    supraman_supervisor::is_foreign(name, workspaces.iter().map(|w| w.name.as_str()), wanted)
}

fn rank(row: &SessionRow) -> u8 {
    match &row.health {
        Health::Blocked { .. } if row.foreign => 1,
        Health::Blocked { .. } => 0,
        Health::Running => 1,
        Health::Idle => 2,
        Health::Unknown => 3,
        Health::Orphaned { .. } => 4,
        Health::Stopped => 5,
    }
}

fn sort(rows: &mut [SessionRow]) {
    rows.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| b.session.started_at.cmp(&a.session.started_at))
    });
}

/// `closed` without each session that continued into one listed or closed, which is its row.
pub fn without_continued(closed: Vec<Session>, continued: &HashMap<String, String>, listed: &[Session]) -> Vec<Session> {
    let known: HashSet<&str> = listed.iter().chain(&closed).map(|s| s.session_id.as_str()).collect();
    let gone: HashSet<String> = closed
        .iter()
        .filter(|s| continued.get(&s.session_id).is_some_and(|newest| *newest != s.session_id && known.contains(newest.as_str())))
        .map(|s| s.session_id.clone())
        .collect();
    closed.into_iter().filter(|s| !gone.contains(&s.session_id)).collect()
}

/// The first instant after `now_ms` at which [`build`] would say something else of the same inputs.
pub fn next_change(sessions: &[Session], closed: &[Session], now_ms: i64) -> Option<i64> {
    use supraman_claude::SessionState as S;
    let orphaned = sessions
        .iter()
        .filter(|s| s.kind == Kind::Background && s.pid.is_none() && matches!(s.state, Some(S::Working | S::Blocked)))
        .filter_map(|s| s.started_at.map(|t| t + supraman_claude::ORPHAN_AFTER.as_millis() as i64 + 1));
    let aged_out = closed.iter().filter_map(|s| s.started_at.map(|t| t + CLOSED_WINDOW_MS + 1));
    orphaned.chain(aged_out).filter(|t| *t > now_ms).min()
}

pub fn build(workspaces: &[Workspace], sessions: &[Session], now_ms: i64, show_stopped: bool, sup: &Supervised) -> Tree {
    let mut b = Build::new(workspaces, sessions, now_ms, show_stopped, sup);
    let mut nodes: Vec<WorkspaceNode> = workspaces.iter().map(|w| b.workspace(w)).collect();
    mark_second_occurrences(&mut nodes, sup.desired);
    Tree {
        workspaces: nodes,
        hidden_stopped: b.hidden.len(),
    }
}

/// One [`build`]'s inputs and what it reads once from them; `hidden` gathers the rows left out.
struct Build<'a> {
    workspaces: &'a [Workspace],
    sessions: &'a [Session],
    now_ms: i64,
    show_stopped: bool,
    sup: &'a Supervised<'a>,
    closed: Vec<Session>,
    owner_of: HashMap<String, String>,
    enabled_in: HashMap<String, Vec<&'a str>>,
    placed_in: HashMap<String, Vec<&'a str>>,
    hidden: HashSet<String>,
}

/// A workspace's wanted sessions: its own, the relay, and the rest by the folder they are wanted in.
#[derive(Default)]
struct Wanted {
    own: Option<SessionRow>,
    relay: Option<SessionRow>,
    supervised: Vec<SessionRow>,
    first_in: HashMap<String, Vec<SessionRow>>,
}

impl<'a> Build<'a> {
    fn new(workspaces: &'a [Workspace], sessions: &'a [Session], now_ms: i64, show_stopped: bool, sup: &'a Supervised<'a>) -> Self {
        let mut b = Build {
            workspaces,
            sessions,
            now_ms,
            show_stopped,
            sup,
            closed: Vec::new(),
            owner_of: owners(workspaces),
            enabled_in: HashMap::new(),
            placed_in: HashMap::new(),
            hidden: HashSet::new(),
        };
        b.closed = b.closed_shown();
        for w in workspaces {
            for f in w.folders.iter().filter(|f| f.enabled) {
                b.enabled_in.entry(path_key(&f.path)).or_default().push(&w.name);
            }
        }
        b.placed_in = b.placed_in();
        b
    }

    fn starred(&self, id: &str) -> bool {
        self.sup.starred.is_some_and(|s| s.contains(id))
    }

    fn main_of(&self, cwd: &Path) -> Option<PathBuf> {
        match self.sup.main_repos {
            Some(known) => known.get(cwd).cloned().flatten(),
            None => main_repo(cwd),
        }
    }

    fn row(&self, session: Session) -> SessionRow {
        let hook = self.sup.hooks.and_then(|h| h.get(&session.session_id)).cloned();
        let health = shown_health(&session, self.now_ms, self.sup.hooks, self.sup.polled_at);
        SessionRow {
            foreign: is_foreign(&session.name, self.workspaces, self.sup.desired, self.sup.own_prefix),
            starred: self.starred(&session.session_id),
            hook,
            ..SessionRow::new(session, health)
        }
    }

    /// Whether the row shows; a stopped one left out is counted in `hidden`.
    fn keep(&mut self, row: &SessionRow) -> bool {
        let keep = self.show_stopped || row.health != Health::Stopped || row.starred;
        if !keep {
            self.hidden.insert(row.session.session_id.clone());
        }
        keep
    }

    fn closed_shown(&self) -> Vec<Session> {
        let listed: HashSet<&str> = self.sessions.iter().map(|s| s.session_id.as_str()).collect();
        // a Supraman session's transcript (a selftest's, a supervised one removed) is not the user's to resume
        self.sup
            .closed
            .iter()
            .filter(|s| !listed.contains(s.session_id.as_str()))
            .filter(|s| self.starred(&s.session_id) || (!s.name.starts_with(supraman_core::FM_PREFIX) && s.started_at.is_some_and(|t| self.now_ms - t <= CLOSED_WINDOW_MS)))
            .cloned()
            .collect()
    }

    /// The workspaces each session sits in through an enabled folder.
    fn placed_in(&self) -> HashMap<String, Vec<&'a str>> {
        let mut placed_in: HashMap<String, Vec<&'a str>> = HashMap::new();
        let main_of = |p: &Path| self.main_of(p);
        for w in self.workspaces {
            let assigned = assign_with(self.sessions, w, main_of).by_repo.into_iter().chain(assign_with(&self.closed, w, main_of).by_repo);
            for (folder, held) in assigned {
                if w.folders.iter().any(|f| f.name == folder && f.enabled) {
                    for s in held {
                        placed_in.entry(s.session_id).or_default().push(&w.name);
                    }
                }
            }
        }
        placed_in
    }

    fn placed_elsewhere(&self, s: &Session, here: &str) -> bool {
        self.placed_in.get(&s.session_id).is_some_and(|ws| ws.iter().any(|w| *w != here))
    }

    fn others(&self, workspace: &str, path: &Path) -> Vec<String> {
        self.enabled_in.get(&path_key(path)).into_iter().flatten().filter(|n| **n != workspace).map(|n| n.to_string()).collect()
    }

    fn workspace(&mut self, workspace: &Workspace) -> WorkspaceNode {
        let (started, wanted) = match self.sup.desired.and_then(|d| d.get(&workspace.name)) {
            Some((started, wanted)) => (Some(*started), wanted.as_slice()),
            None => (None, &[][..]),
        };
        let (theirs, rest): (Vec<Session>, Vec<Session>) = self.sessions.iter().cloned().partition(|s| wanted.contains(&s.name));
        let (scheduled, rest) = self.scheduled(&workspace.name, rest);
        let mut wanted = self.wanted(workspace, theirs);
        let main_of = |p: &Path| self.main_of(p);
        let (assigned, gone) = (assign_with(&rest, workspace, main_of), assign_with(&self.closed, workspace, main_of));
        let (mut repos, parked) = self.folders(workspace, assigned.by_repo, gone.by_repo, &mut wanted.first_in);
        if let Some(home) = workspace.home() {
            repos.extend(self.home_groups(workspace, home, assigned.in_home, gone.in_home));
        }
        repos.extend(parked);
        let mut supervised = wanted.supervised;
        supervised.extend(wanted.first_in.into_values().flatten());
        WorkspaceNode {
            name: workspace.name.clone(),
            home: workspace.home().map(Path::to_path_buf),
            started,
            own: wanted.own,
            relay: wanted.relay,
            supervised,
            scheduled,
            repos,
        }
    }

    /// The schedule runs' rows by schedule id, and the sessions that are not runs.
    fn scheduled(&mut self, workspace: &str, sessions: Vec<Session>) -> (BTreeMap<String, Vec<SessionRow>>, Vec<Session>) {
        let mut scheduled: BTreeMap<String, Vec<SessionRow>> = BTreeMap::new();
        let mut rest = Vec::new();
        for s in sessions {
            match supraman_core::schedule::schedule_of_session(workspace, &s.name) {
                Some(id) => {
                    let r = self.row(s.clone());
                    if self.keep(&r) {
                        scheduled.entry(id.to_string()).or_default().push(r);
                    }
                }
                None => rest.push(s),
            }
        }
        scheduled.values_mut().for_each(|rows| sort(rows));
        (scheduled, rest)
    }

    fn wanted(&self, workspace: &Workspace, theirs: Vec<Session>) -> Wanted {
        let (own, relay) = (workspace_session_name(&workspace.name), relay_session_name(&workspace.name));
        let folder_of_wanted = |name: &str| {
            workspace.folders.iter().find(|f| repo_session_name(&workspace.name, &f.name).as_deref() == Some(name)).map(|f| f.name.clone())
        };
        let mut out = Wanted::default();
        for s in theirs {
            if s.name == own {
                out.own = Some(self.row(s));
            } else if s.name == relay {
                out.relay = Some(self.row(s));
            } else {
                match folder_of_wanted(&s.name) {
                    Some(folder) => out.first_in.entry(folder).or_default().push(self.row(s)),
                    None => out.supervised.push(self.row(s)),
                }
            }
        }
        sort(&mut out.supervised);
        out
    }

    /// A folder's rows: the wanted ones, the open ones, then the closed ones up to [`CLOSED_CAP`]; and how many that cap left out.
    fn place(&mut self, workspace: &str, name: &str, open: Vec<Session>, closed: Vec<Session>, wanted: Vec<SessionRow>) -> (Vec<SessionRow>, usize) {
        let mut rows: Vec<SessionRow> = Vec::new();
        for s in open {
            let r = self.row(s);
            if self.keep(&r) {
                rows.push(r);
            }
        }
        sort(&mut rows);
        let mut gone: Vec<SessionRow> = closed.into_iter().map(|s| SessionRow { closed: true, ..self.row(s) }).collect();
        gone.sort_by(|a, b| b.session.started_at.cmp(&a.session.started_at));
        let all = self.sup.expanded.is_some_and(|e| e.contains(&format!("{workspace}/{name}")));
        let (mut shown, mut more) = (0, 0);
        for r in gone {
            if r.starred {
                rows.push(r);
            } else if !self.show_stopped {
                self.hidden.insert(r.session.session_id.clone());
            } else if all || shown < CLOSED_CAP {
                shown += 1;
                rows.push(r);
            } else {
                more += 1;
            }
        }
        let mut sessions = wanted;
        sort(&mut sessions);
        sessions.extend(rows);
        (sessions, more)
    }

    /// The workspace's folders: the enabled ones, and the disabled ones still listed (parked).
    fn folders(
        &mut self,
        workspace: &Workspace,
        mut by_repo: BTreeMap<String, Vec<Session>>,
        mut closed_by_repo: BTreeMap<String, Vec<Session>>,
        first_in: &mut HashMap<String, Vec<SessionRow>>,
    ) -> (Vec<RepoNode>, Vec<RepoNode>) {
        let (mut repos, mut parked) = (Vec::new(), Vec::new());
        for folder in &workspace.folders {
            let supervised_in = self.owner_of.get(&path_key(&folder.path)).filter(|o| **o != workspace.name).cloned();
            let also_in = if folder.enabled { self.others(&workspace.name, &folder.path) } else { Vec::new() };
            let node = |sessions: Vec<SessionRow>, more: usize| RepoNode {
                name: folder.name.clone(),
                path: folder.path.clone(),
                kind: folder.kind,
                branch: folder.branch.clone(),
                enabled: folder.enabled,
                listed: true,
                supervised_in: supervised_in.clone(),
                also_in: also_in.clone(),
                sessions,
                more,
            };
            let (sessions, more) = self.place(
                &workspace.name,
                &folder.name,
                by_repo.remove(&folder.name).unwrap_or_default(),
                closed_by_repo.remove(&folder.name).unwrap_or_default(),
                first_in.remove(&folder.name).unwrap_or_default(),
            );
            if folder.enabled {
                repos.push(node(sessions, more));
            } else if supervised_in.is_some() {
                // its sessions show where it is enabled; search still finds the folder
                if self.sup.disabled_folders {
                    parked.push(node(Vec::new(), 0));
                }
            } else if !sessions.is_empty() || more > 0 || self.sup.disabled_folders {
                parked.push(node(sessions, more));
            }
        }
        (repos, parked)
    }

    /// The home's sessions in no folder, grouped by [`home_folder`]; a group with no row is left out.
    fn home_groups(&mut self, workspace: &Workspace, home: &Path, open: Vec<Session>, gone: Vec<Session>) -> Vec<RepoNode> {
        let mut groups: BTreeMap<String, (PathBuf, Vec<Session>, Vec<Session>)> = BTreeMap::new();
        let in_home = open.into_iter().map(|s| (s, false)).chain(gone.into_iter().map(|s| (s, true)));
        for (s, is_closed) in in_home.filter(|(s, _)| !self.placed_elsewhere(s, &workspace.name)) {
            let (name, path) = home_folder(home, &s.cwd);
            let group = groups.entry(name).or_insert_with(|| (path, Vec::new(), Vec::new()));
            if is_closed { group.2.push(s) } else { group.1.push(s) }
        }
        let mut repos = Vec::new();
        for (name, (path, open, gone)) in groups {
            let (sessions, more) = self.place(&workspace.name, &name, open, gone, Vec::new());
            if sessions.is_empty() && more == 0 {
                continue;
            }
            repos.push(RepoNode {
                name,
                path,
                kind: FolderKind::Plain,
                branch: None,
                enabled: false,
                listed: false,
                supervised_in: None,
                also_in: Vec::new(),
                sessions,
                more,
            });
        }
        repos
    }
}

/// The home's folder `cwd` lies in, named by its first segment below the home; the home itself for a session there.
fn home_folder(home: &Path, cwd: &Path) -> (String, PathBuf) {
    let (home_n, cwd_n) = (normalize(home), normalize(cwd));
    let below = cwd_n.get(home_n.len()..).unwrap_or("").trim_start_matches('/');
    match below.split('/').next().filter(|s| !s.is_empty()) {
        Some(first) => (first.to_string(), home.join(first)),
        None => (home.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| home_n.clone()), home.to_path_buf()),
    }
}

/// A session under several workspaces: the wanting (else first) occurrence keeps its keys; the others name it.
fn mark_second_occurrences(nodes: &mut [WorkspaceNode], desired: Option<&BTreeMap<String, (bool, Vec<String>)>>) {
    let wants = |ws: &str, name: &str| desired.and_then(|d| d.get(ws)).is_some_and(|(_, wanted)| wanted.iter().any(|w| w == name));
    let mut seen: HashMap<String, Vec<(usize, bool)>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        for r in n.rows() {
            seen.entry(r.session.session_id.clone()).or_default().push((i, wants(&n.name, &r.session.name)));
        }
    }
    let names: Vec<String> = nodes.iter().map(|n| n.name.clone()).collect();
    for (id, places) in seen {
        if places.len() < 2 {
            continue;
        }
        let owner = places.iter().find(|p| p.1).unwrap_or(&places[0]).0;
        for (_, n) in nodes.iter_mut().enumerate().filter(|(i, _)| *i != owner) {
            for r in n.rows_mut() {
                if r.session.session_id == id {
                    r.also_in = Some(names[owner].clone());
                }
            }
        }
    }
}

/// A session's state as every view words and marks it: a live one by its [`Activity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Live(Activity),
    Interrupted,
    Stopped,
    Closed,
    Unknown,
}

impl State {
    pub fn of(health: &Health, closed: bool) -> State {
        match (activity(health), health) {
            _ if closed => State::Closed,
            (Some(a), _) => State::Live(a),
            (None, Health::Orphaned { .. }) => State::Interrupted,
            (None, Health::Unknown) => State::Unknown,
            (None, _) => State::Stopped,
        }
    }

    pub fn of_row(row: &SessionRow) -> State {
        State::of(&row.health, row.closed)
    }

    pub fn word(self) -> &'static str {
        match self {
            State::Live(a) => a.word(),
            State::Interrupted => "interrupted",
            State::Stopped => "stopped",
            State::Closed => "closed",
            State::Unknown => "unknown",
        }
    }

    pub fn mark(self, cx: &App) -> Mark {
        let (status, colors) = (cx.theme().status(), cx.theme().colors());
        match self {
            State::Live(a) => Mark::Filled(a.color(cx)),
            State::Interrupted => Mark::Ring(status.warning),
            State::Unknown => Mark::Ring(status.hint),
            State::Stopped | State::Closed => Mark::Ring(colors.text_disabled),
        }
    }
}

/// What a session's Open does: the row's action slot, its double-click and every page's Open ([`supraman_ui::session::open`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Open {
    Tab(u64),
    Shown,
    Attach,
    Resume,
    Foreign,
    Elsewhere,
    Supervised,
    /// The page's session is not in the list Supraman reads now.
    Unlisted,
    Nothing,
}

impl Open {
    /// `tab`: the tab running it, `active`: the tab shown now, `resumable`: [`crate::view::offers_resume`].
    pub fn of(row: &SessionRow, tab: Option<u64>, active: Option<u64>, resumable: bool) -> Self {
        let s = &row.session;
        match tab {
            _ if row.foreign => Self::Foreign,
            Some(id) if active == Some(id) => Self::Shown,
            Some(id) => Self::Tab(id),
            None if row.attachable => Self::Attach,
            None if resumable => Self::Resume,
            None if is_alive(&row.health) && s.kind != Kind::Background => Self::Elsewhere,
            None if !is_alive(&row.health) && s.name.starts_with(supraman_core::FM_PREFIX) => Self::Supervised,
            None => Self::Nothing,
        }
    }

    /// Every word said of a session's Open: its tooltip, its why while dimmed, its Details fact.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Tab(_) => "Show its tab",
            Self::Shown => "shown now",
            Self::Attach => "Attach in a new tab",
            Self::Resume => "Resume in a new tab",
            Self::Foreign => "another Supraman session",
            Self::Elsewhere => "runs in another terminal: resuming here would start a second claude on it",
            Self::Supervised => "the supervisor runs it while it is wanted: Start the workspace from its menu",
            Self::Unlisted => "not in the sessions Supraman lists now",
            Self::Nothing => "neither running in the background to attach nor resumable",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Tab(_) | Self::Shown => Icon::Tab,
            Self::Resume => Icon::Resume,
            _ => Icon::Attach,
        }
    }

    pub fn runs(self) -> bool {
        matches!(self, Self::Tab(_) | Self::Attach | Self::Resume)
    }

    pub fn opening(self) -> Opening {
        Opening { icon: self.icon(), hint: self.hint().into(), runs: self.runs() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adoption {
    /// It continues as `name`, `fm-<workspace>-<folder>`.
    Offer { name: String },
    /// Why there is none, for its hover card.
    No(String),
}

/// Only a background session Supraman did not start, under `folder` enabled here and supervised by this workspace.
pub fn adoption(
    workspaces: &[Workspace],
    workspace: &str,
    folder: &str,
    row: &SessionRow,
    sessions: &[Session],
    desired: &BTreeMap<String, (bool, Vec<String>)>,
) -> Option<Adoption> {
    let s = &row.session;
    let unmanaged = s.kind == Kind::Background
        && !row.foreign
        && !row.closed
        && !s.name.starts_with(supraman_core::FM_PREFIX)
        && !s.id.is_empty()
        && !s.session_id.is_empty()
        && matches!(row.health, Health::Running | Health::Idle | Health::Blocked { .. } | Health::Stopped);
    let f = workspaces.iter().find(|w| w.name == workspace)?.folder(folder)?;
    if !unmanaged || !f.enabled || owners(workspaces).get(&path_key(&f.path)).map(String::as_str) != Some(workspace) {
        return None;
    }
    let name = repo_session_name(workspace, &f.name)?;
    if desired.get(workspace).is_some_and(|(_, wanted)| wanted.contains(&name)) {
        return Some(Adoption::No(format!("none: {name} is wanted already, and a folder has one supervised session")));
    }
    if sessions.iter().any(|x| x.name == name) {
        return Some(Adoption::No(format!("none: {name} exists already; a folder has one supervised session")));
    }
    Some(Adoption::Offer { name })
}

pub fn short_id(session: &Session) -> &str {
    session.id.get(..8).unwrap_or(&session.id)
}

pub fn display_name(session: &Session) -> &str {
    if session.name.is_empty() { short_id(session) } else { &session.name }
}

/// The home of the workspace holding `active` (the deepest), else of the first workspace, else `home`.
pub fn shell_cwd(active: Option<&Path>, workspaces: &[Workspace], home: Option<PathBuf>) -> Option<PathBuf> {
    let holder = active.and_then(|cwd| holder_of(cwd, workspaces));
    holder.or(workspaces.first()).and_then(|w| w.home()).map(Path::to_path_buf).or(home)
}

pub use supraman_core::holder_of;

#[cfg(test)]
mod tests {
    use super::*;
    use supraman_claude::parse;
    use supraman_core::Folder;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn shell_cwd_is_the_active_tabs_workspace_then_the_first_then_home() {
        let a = Workspace::new("a", "P:/Projects/A", vec![]);
        let b = Workspace::new("b", r"P:\Projects\B", vec![]);
        let inner = Workspace::new("inner", r"P:\Projects\B\inner", vec![]);
        let all = [a.clone(), b.clone(), inner.clone()];
        let home = Some(PathBuf::from(r"C:\Users\me"));
        let cwd = |active: Option<&str>, workspaces: &[Workspace]| shell_cwd(active.map(Path::new), workspaces, home.clone());
        let h = |w: &Workspace| w.home().map(Path::to_path_buf);
        assert_eq!(cwd(Some(r"p:\projects\b\repo\src"), &all), h(&b), "another drive-letter case");
        assert_eq!(cwd(Some(r"P:\Projects\A\x"), &all), h(&a), "forward-slash home");
        assert_eq!(cwd(Some(r"P:\Projects\B\inner\r"), &all), h(&inner), "innermost home");
        assert_eq!(cwd(Some(r"P:\Projects\Bee"), &all), h(&a), "a sibling with a common prefix is not inside");
        assert_eq!(cwd(None, &all), h(&a), "no active tab: the first workspace");
        assert_eq!(cwd(Some(r"D:\elsewhere"), &[]), home, "no workspace: home");
        let virt = Workspace::new("virt", r"D:\Gaia", vec![Folder::new("Gaia", r"D:\Gaia"), Folder::new("NodeApi", r"P:\Projects\B\inner\NodeApi")]);
        let all = [a.clone(), b.clone(), inner.clone(), virt];
        assert_eq!(cwd(Some(r"P:\Projects\B\inner\NodeApi\src"), &all), Some(PathBuf::from(r"D:\Gaia")), "a folder deeper than any home");
    }

    fn workspace() -> Workspace {
        let repo = |name: &str, enabled: bool| Folder { branch: Some("main".into()), enabled, ..Folder::new(name, format!(r"C:\demo\{name}")) };
        Workspace::new(
            "demo",
            r"C:\demo",
            vec![repo("Alpha", true), repo("Beta", true), repo("Off", false)],
        )
    }

    fn sessions() -> Vec<Session> {
        let old = NOW - 3_600_000;
        parse(&format!(
            r#"[
  {{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"busy","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"working","status":"busy","pid":1}},
  {{"id":"aaaa0002","sessionId":"aaaa0002-x","name":"parked","cwd":"C:\\demo\\Alpha\\src","kind":"background","startedAt":{old},"state":"blocked","waitingFor":"permission prompt","pid":2}},
  {{"id":"aaaa0003","sessionId":"aaaa0003-x","name":"gone","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"blocked"}},
  {{"id":"aaaa0004","sessionId":"aaaa0004-x","name":"done","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"stopped"}},
  {{"sessionId":"bbbb0005-x","name":"term","cwd":"C:\\demo\\Beta","kind":"interactive","startedAt":{old},"status":"idle","pid":5}},
  {{"id":"aaaa0006","sessionId":"aaaa0006-x","name":"elsewhere","cwd":"C:\\other","kind":"background","startedAt":{old},"state":"working","status":"idle","pid":6}},
  {{"id":"aaaa0007","sessionId":"aaaa0007-x","name":"off","cwd":"C:\\demo\\Off","kind":"background","startedAt":{old},"state":"working","status":"idle","pid":7}}
]"#
        ))
        .unwrap()
    }

    fn names(repo: &RepoNode) -> Vec<&str> {
        repo.sessions.iter().map(|r| r.session.name.as_str()).collect()
    }

    #[test]
    fn a_polls_main_repos_table_places_worktree_sessions_without_touching_the_disk() {
        // C:\other has no .git, so only the table can say it is a worktree of Beta
        let known = HashMap::from([(PathBuf::from(r"C:\other"), Some(PathBuf::from("C:/demo/Beta")))]);
        let sup = Supervised { main_repos: Some(&known), ..Supervised::default() };
        let tree = build(&[workspace()], &sessions(), NOW, false, &sup);
        assert!(names(&tree.workspaces[0].repos[1]).contains(&"elsewhere"));
        let tree = build(&[workspace()], &sessions(), NOW, false, &Supervised::default());
        assert!(!names(&tree.workspaces[0].repos[1]).contains(&"elsewhere"));
    }

    #[test]
    fn groups_enabled_repos_blocked_first_and_hides_stopped() {
        let tree = build(&[workspace()], &sessions(), NOW, false, &Supervised::default());
        let p = &tree.workspaces[0];
        let repos: Vec<_> = p.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(repos, ["Alpha", "Beta", "Off"], "disabled repo Off comes last");
        assert!(!p.repos[2].enabled, "shown, as it holds a session");
        assert_eq!(names(&p.repos[2]), ["off"]);
        assert_eq!(names(&p.repos[0]), ["parked", "busy"]);
        assert_eq!(names(&p.repos[1]), ["term", "gone"]);
        assert_eq!(tree.hidden_stopped, 1);
        assert_eq!(p.session_count(), 5);
    }

    #[test]
    fn a_disabled_folder_without_sessions_leaves_the_tree_and_stays_for_search() {
        let quiet: Vec<Session> = sessions().into_iter().filter(|s| s.name != "off").collect();
        let tree = build(&[workspace()], &quiet, NOW, false, &Supervised::default());
        assert_eq!(tree.workspaces[0].repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Alpha", "Beta"]);
        let all = build(&[workspace()], &quiet, NOW, false, &Supervised { disabled_folders: true, ..Supervised::default() });
        let off = all.workspaces[0].repos.last().unwrap();
        assert_eq!((off.name.as_str(), off.enabled, off.sessions.len()), ("Off", false, 0));
    }

    #[test]
    fn show_stopped_keeps_them_last() {
        let tree = build(&[workspace()], &sessions(), NOW, true, &Supervised::default());
        assert_eq!(names(&tree.workspaces[0].repos[1]), ["term", "gone", "done"]);
        assert_eq!(tree.hidden_stopped, 0);
    }

    #[test]
    fn attach_only_for_live_background_sessions() {
        let tree = build(&[workspace()], &sessions(), NOW, true, &Supervised::default());
        let flags: Vec<(&str, bool)> = tree.workspaces[0]
            .repos
            .iter()
            .flat_map(|r| &r.sessions)
            .map(|r| (r.session.name.as_str(), r.attachable))
            .collect();
        assert_eq!(
            flags,
            [
                ("parked", true),
                ("busy", true),
                ("term", false),
                ("gone", false),
                ("done", false),
                ("off", true)
            ]
        );
    }

    #[test]
    fn a_rows_one_open_action_follows_tab_then_attach_then_resume_and_refuses_the_rest() {
        let tree = build(&[workspace()], &sessions(), NOW, true, &Supervised::default());
        let row = |name: &str| tree.workspaces[0].rows().find(|r| r.session.name == name).unwrap().clone();
        let (busy, term, done, gone) = (row("busy"), row("term"), row("done"), row("gone"));
        assert_eq!(Open::of(&busy, None, None, false), Open::Attach);
        assert_eq!(Open::of(&busy, Some(3), Some(1), false), Open::Tab(3), "open in a tab beats attach");
        assert_eq!(Open::of(&busy, Some(3), Some(3), false), Open::Shown);
        assert_eq!(Open::of(&done, None, None, true), Open::Resume);
        assert_eq!(Open::of(&gone, None, None, true), Open::Resume, "interrupted resumes");
        assert_eq!(Open::of(&done, None, None, false), Open::Nothing);
        assert_eq!(Open::of(&term, None, None, false), Open::Elsewhere, "live interactive outside a tab");
        assert_eq!(Open::of(&term, Some(2), None, false), Open::Tab(2));

        let mut sm = done.clone();
        sm.session.name = "fm-demo-alpha".into();
        assert_eq!(Open::of(&sm, None, None, false), Open::Supervised);
        let mut foreign = busy.clone();
        foreign.foreign = true;
        assert_eq!(Open::of(&foreign, None, None, true), Open::Foreign);
        assert_eq!(Open::of(&foreign, Some(3), None, true), Open::Foreign, "nothing for another Supraman's, even in a tab");
        assert_eq!(Open::Foreign.hint(), "another Supraman session");
        let runs: Vec<bool> = [Open::Tab(1), Open::Attach, Open::Resume, Open::Shown, Open::Foreign, Open::Elsewhere, Open::Supervised, Open::Unlisted, Open::Nothing].map(Open::runs).into();
        assert_eq!(runs, [true, true, true, false, false, false, false, false, false], "only a tab, an attach or a resume opens");
    }

    #[test]
    fn labels() {
        let tree = build(&[workspace()], &sessions(), NOW, true, &Supervised::default());
        let beta = &tree.workspaces[0].repos[1];
        assert_eq!(State::of_row(&beta.sessions[1]).word(), "interrupted");
        let alpha = &tree.workspaces[0].repos[0];
        assert_eq!(State::of_row(&alpha.sessions[0]).word(), "waiting");
        assert_eq!(
            alpha.sessions[0].health,
            Health::Blocked {
                waiting_for: Some("permission prompt".into())
            }
        );
        assert_eq!(short_id(&beta.sessions[0].session), "bbbb0005");
    }

    #[test]
    fn supraman_session_waiting_on_a_peer_is_idle_a_parked_one_blocked() {
        let old = NOW - 3_600_000;
        let rows = parse(&format!(
            r#"[
  {{"id":"aaaa0011","sessionId":"aaaa0011-x","name":"fm-demo-alpha","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"blocked","pid":11}},
  {{"id":"aaaa0012","sessionId":"aaaa0012-x","name":"fm-demo-beta","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":12}},
  {{"id":"aaaa0013","sessionId":"aaaa0013-x","name":"mine","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"blocked","pid":13}}
]"#
        ))
        .unwrap();
        let tree = build(&[workspace()], &rows, NOW, false, &Supervised::default());
        let health: Vec<(&str, &Health)> =
            tree.workspaces[0].repos.iter().flat_map(|r| &r.sessions).map(|r| (r.session.name.as_str(), &r.health)).collect();
        let parked = Health::Blocked { waiting_for: Some("permission prompt".into()) };
        let asks = Health::Blocked { waiting_for: None };
        assert_eq!(health, [("fm-demo-alpha", &Health::Idle), ("fm-demo-beta", &parked), ("mine", &asks)]);

        let mut hooks = HashMap::new();
        let mut prompt = event("Notification", Some("permission_prompt"), NOW);
        prompt.session_id = "aaaa0011-x".into();
        HookSignal::record(&mut hooks, prompt);
        let sup = Supervised { hooks: Some(&hooks), polled_at: NOW - 1_000, ..Supervised::default() };
        let tree = build(&[workspace()], &rows, NOW, false, &sup);
        assert_eq!(tree.workspaces[0].repos[0].sessions[0].health, parked, "the hook's permission prompt still blocks it");
    }

    #[test]
    fn another_homes_fm_rows_are_foreign_shown_and_never_ranked_blocked() {
        let mut desired = BTreeMap::new();
        desired.insert("demo".to_string(), (true, vec!["fm-demo-workspace".to_string(), "fm-x-workspace".to_string()]));
        let ws = [workspace()];
        let foreign = |name: &str| is_foreign(name, &ws, Some(&desired), None);
        assert!(foreign("fm-selftest-abc123-park"), "a selftest's session");
        assert!(foreign("fm-other-workspace"), "another home's workspace");
        assert!(!foreign("fm-demo-workspace"), "wanted here");
        assert!(!foreign("fm-x-workspace"), "wanted here, whatever its name");
        assert!(!foreign("fm-demo-alpha"), "named after a workspace of this home: retired, or wanted before the first tick");
        assert!(!is_foreign("fm-demo-alpha", &ws, None, None));
        let run = |name: &str| is_foreign(name, &ws, Some(&desired), Some("fm-selftest-abc123"));
        assert!(!run("fm-selftest-abc123") && !run("fm-selftest-abc123-park"), "this selftest run's own");
        assert!(run("fm-selftest-def456-park"), "another selftest run's");
        assert!(run("fm-selftest-abc1234"), "a longer hex is another run");
        assert!(!foreign("mine"), "the user's own session");

        let old = NOW - 3_600_000;
        let rows = parse(&format!(
            r#"[
  {{"id":"aaaa0021","sessionId":"aaaa0021-x","name":"fm-selftest-abc123-park","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":21}},
  {{"id":"aaaa0022","sessionId":"aaaa0022-x","name":"mine","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{},"state":"working","status":"busy","pid":22}},
  {{"id":"aaaa0023","sessionId":"aaaa0023-x","name":"fm-demo-alpha","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":23}}
]"#,
            NOW - 1_000
        ))
        .unwrap();
        let sup = Supervised { desired: Some(&desired), ..Supervised::default() };
        let tree = build(&ws, &rows, NOW, false, &sup);
        let alpha: Vec<(&str, bool)> = tree.workspaces[0].repos[0].sessions.iter().map(|r| (r.session.name.as_str(), r.foreign)).collect();
        assert_eq!(alpha, [("fm-demo-alpha", false), ("mine", false), ("fm-selftest-abc123-park", true)], "own blocked first, the foreign one as running");
        let park = &tree.workspaces[0].repos[0].sessions[2];
        assert_eq!(park.health, Health::Blocked { waiting_for: Some("permission prompt".into()) }, "its health is still shown");
    }

    fn closed(id: &str, name: &str, cwd: &str, age_ms: i64) -> Session {
        supraman_claude::Transcript {
            session_id: id.into(),
            cwd: Some(cwd.into()),
            custom_title: Some(name.into()),
            modified_ms: NOW - age_ms,
            ..Default::default()
        }
        .to_session()
    }

    #[test]
    fn a_continued_session_shows_as_one_row_its_newest() {
        const OLD: &str = "0a000000-0000-4000-8000-00000000000a";
        const NEW: &str = "0b000000-0000-4000-8000-00000000000b";
        const LOST: &str = "0c000000-0000-4000-8000-00000000000c";
        let continued = HashMap::from([(OLD.to_string(), NEW.to_string()), (LOST.to_string(), "0d000000-0000-4000-8000-00000000000d".to_string())]);
        let rows = vec![closed(OLD, "workman", r"C:\demo\alpha", 0), closed(LOST, "other", r"C:\demo\alpha", 0)];
        let live = Session { kind: Kind::Background, state: Some(supraman_claude::SessionState::Blocked), ..closed(NEW, "workman", r"C:\demo\alpha", 0) };
        let shown: Vec<String> = without_continued(rows.clone(), &continued, std::slice::from_ref(&live)).into_iter().map(|s| s.session_id).collect();
        assert_eq!(shown, [LOST], "the old row goes while its continuation is listed; a chain to no known session keeps its row");
        assert_eq!(without_continued(rows.clone(), &continued, &[]).len(), 2, "its continuation not listed: the row stays");
        let both = vec![rows[0].clone(), closed(NEW, "workman", r"C:\demo\alpha", 0)];
        assert_eq!(without_continued(both, &continued, &[]).iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(), [NEW], "both closed: the newest");
    }

    #[test]
    fn closed_sessions_show_with_stopped_newest_first_capped_and_starred_ones_always() {
        const DAY: i64 = 24 * 3_600_000;
        let mut gone = vec![
            closed("cccc0001-x", "yesterday", r"C:\demo\Beta", DAY),
            closed("bbbb0005-x", "term again", r"C:\demo\Beta", DAY),
            closed("cccc0002-x", "old", r"C:\demo\Beta", 20 * DAY),
            closed("cccc0003-x", "fm-selftest-abc123", r"C:\demo\Beta", DAY),
        ];
        gone.extend((0..7).map(|i| closed(&format!("dddd000{i}-x"), &format!("alpha{i}"), r"C:\demo\Alpha", (i + 1) * 3_600_000)));
        let sup = |starred, expanded| Supervised { closed: &gone, starred, expanded, ..Supervised::default() };

        let tree = build(&[workspace()], &sessions(), NOW, true, &sup(None, None));
        let beta = &tree.workspaces[0].repos[1];
        assert_eq!(names(beta), ["term", "gone", "done", "yesterday"], "listed by claude agents once; not past the window; no fm- transcript");
        assert!(beta.sessions[3].closed && !beta.sessions[0].closed);
        assert_eq!(beta.sessions[3].health, Health::Stopped);
        assert_eq!(beta.sessions[3].session.kind, Kind::Interactive);
        let alpha = &tree.workspaces[0].repos[0];
        assert_eq!(names(alpha), ["parked", "busy", "alpha0", "alpha1", "alpha2", "alpha3", "alpha4"], "newest first, capped");
        assert_eq!(alpha.more, 2);

        let expanded = HashSet::from(["demo/Alpha".to_string()]);
        let tree = build(&[workspace()], &sessions(), NOW, true, &sup(None, Some(&expanded)));
        assert_eq!(tree.workspaces[0].repos[0].sessions.len(), 9, "more… shows the rest");
        assert_eq!(tree.workspaces[0].repos[0].more, 0);

        let hidden = build(&[workspace()], &sessions(), NOW, false, &sup(None, None));
        assert_eq!(names(&hidden.workspaces[0].repos[1]), ["term", "gone"]);
        assert_eq!(hidden.hidden_stopped, 1 + 1 + 7, "done, yesterday and Alpha's seven");

        let starred = BTreeSet::from(["cccc0002-x".to_string(), "aaaa0004-x".to_string(), "dddd0006-x".to_string()]);
        let tree = build(&[workspace()], &sessions(), NOW, false, &sup(Some(&starred), None));
        assert_eq!(names(&tree.workspaces[0].repos[1]), ["term", "gone", "done", "old"], "stopped hidden, yet the starred stopped and 20-day-old ones stay");
        assert!(tree.workspaces[0].repos[1].sessions[2].starred);
        assert_eq!(names(&tree.workspaces[0].repos[0]), ["parked", "busy", "alpha6"]);
        let tree = build(&[workspace()], &sessions(), NOW, true, &sup(Some(&starred), None));
        let alpha = &tree.workspaces[0].repos[0];
        assert_eq!(names(alpha).len(), 2 + 1 + CLOSED_CAP, "a starred one is not counted in the cap");
        assert_eq!(alpha.more, 1);
    }

    #[test]
    fn no_workspaces_is_empty() {
        let tree = build(&[], &sessions(), NOW, false, &Supervised::default());
        assert!(tree.workspaces.is_empty());
    }

    fn event(event: &str, detail: Option<&str>, ts: i64) -> Event {
        let mut e = Event::from_payload(event, r#"{"session_id":"aaaa0001-x"}"#, ts).unwrap();
        match event {
            "SessionStart" => e.source = detail.map(Into::into),
            _ => e.notification_type = detail.map(Into::into),
        }
        e
    }

    #[test]
    fn wanted_sessions_sit_under_the_workspace_stopped_ones_included() {
        let mut desired = BTreeMap::new();
        desired.insert("demo".to_string(), (true, vec!["busy".to_string(), "done".to_string()]));
        let sup = Supervised {
            desired: Some(&desired),
            ..Supervised::default()
        };
        let tree = build(&[workspace()], &sessions(), NOW, false, &sup);
        let p = &tree.workspaces[0];
        assert_eq!(p.started, Some(true));
        let supervised: Vec<&str> = p.supervised.iter().map(|r| r.session.name.as_str()).collect();
        assert_eq!(supervised, ["busy", "done"]);
        assert_eq!(names(&p.repos[0]), ["parked"], "not listed twice");
        assert!(p.own.is_none() && p.relay.is_none() && p.supervised.iter().all(|r| r.also_in.is_none()));
        assert_eq!(tree.hidden_stopped, 0, "the stopped wanted row is shown, not hidden");
        let never = build(&[workspace()], &sessions(), NOW, false, &Supervised::default());
        assert_eq!(never.workspaces[0].started, None);
    }

    #[test]
    fn a_scheduled_runs_session_sits_under_its_schedule_not_its_folder() {
        let mut list = rows(&[
            ("eeee0001", "fm-larchsys-sched-nightly-20261102-090000", r"P:\L\Gaia\.claude\worktrees\fm-sched-nightly-20261102-090000"),
            ("eeee0002", "fm-larchsys-sched-nightly-20261101-090000", r"P:\L\Gaia"),
            ("eeee0003", "fm-larchsys-sched-weekly-fix-20261102-090000-2", r"P:\L\Skills"),
            ("eeee0004", "mine", r"P:\L\Gaia"),
        ]);
        list[1].state = Some(supraman_claude::SessionState::Stopped);
        list[1].pid = None;
        let ws = overlapping();
        let tree = build(&ws[1..], &list, NOW, false, &Supervised::default());
        let w = &tree.workspaces[0];
        let ids: Vec<(&str, Vec<&str>)> = w.scheduled.iter().map(|(id, r)| (id.as_str(), r.iter().map(|r| r.session.id.as_str()).collect())).collect();
        assert_eq!(ids, [("nightly", vec!["eeee0001"]), ("weekly-fix", vec!["eeee0003"])], "the stopped run is hidden with stopped rows");
        assert_eq!(tree.hidden_stopped, 1);
        assert!(w.repos.iter().all(|r| r.sessions.iter().all(|s| !s.session.name.contains("-sched-"))), "not under a folder too");
        assert_eq!(w.session_count(), 3);
        assert!(w.rows().any(|r| r.session.id == "eeee0003"), "menus find it");
        let shown = build(&ws[1..], &list, NOW, true, &Supervised::default());
        assert_eq!(shown.workspaces[0].scheduled["nightly"].len(), 2);
    }

    #[test]
    fn hook_signal_labels_and_early_block() {
        let mut hooks = HashMap::new();
        HookSignal::record(&mut hooks, event("SessionStart", Some("startup"), NOW - 300_000));
        HookSignal::record(&mut hooks, event("SessionStart", Some("compact"), NOW - 120_000));
        HookSignal::record(&mut hooks, event("Stop", None, NOW - 5_000));
        let h = &hooks["aaaa0001-x"];
        assert_eq!(h.label(NOW), "turn ended 5 s ago · compacted 2 min ago");
        let polled_at = NOW - 1_000;
        assert_eq!(with_hook(Health::Running, Some(h), polled_at), Health::Running);

        HookSignal::record(&mut hooks, event("Notification", Some("permission_prompt"), NOW));
        let h = &hooks["aaaa0001-x"];
        assert_eq!(h.label(NOW + 61_000), "permission prompt 1 min ago · compacted 3 min ago");
        let blocked = Health::Blocked { waiting_for: Some("permission prompt".into()) };
        assert_eq!(with_hook(Health::Running, Some(h), polled_at), blocked, "newer than the poll");
        assert_eq!(with_hook(Health::Running, Some(h), NOW + 1), Health::Running, "the poll is newer: trust it");
        assert_eq!(with_hook(Health::Stopped, Some(h), polled_at), Health::Stopped);

        let sup = Supervised {
            hooks: Some(&hooks),
            polled_at,
            ..Supervised::default()
        };
        let tree = build(&[workspace()], &sessions(), NOW, false, &sup);
        let busy = tree.workspaces[0].repos[0].sessions.iter().find(|r| r.session.name == "busy").unwrap();
        assert_eq!(busy.health, blocked);
        assert!(busy.hook.is_some());
    }

    #[test]
    fn live_sessions_show_the_sidebars_state() {
        let list = sessions();
        let activity_of = |hooks: &HashMap<String, HookSignal>, polled_at: i64| {
            live_sessions(&list, NOW, hooks, polled_at).into_iter().find(|s| s.name == "busy").unwrap().activity
        };
        let health_of = |hooks: &HashMap<String, HookSignal>, polled_at: i64| {
            let sup = Supervised { hooks: Some(hooks), polled_at, ..Supervised::default() };
            let tree = build(&[workspace()], &list, NOW, false, &sup);
            tree.workspaces[0].repos[0].sessions.iter().find(|r| r.session.name == "busy").unwrap().health.clone()
        };
        let mut hooks = HashMap::new();
        HookSignal::record(&mut hooks, event("Notification", Some("permission_prompt"), NOW - 500));

        let blocked = Health::Blocked { waiting_for: Some("permission prompt".into()) };
        assert_eq!(activity_of(&hooks, NOW - 1_000), Activity::Waiting, "a prompt newer than the poll");
        assert_eq!(health_of(&hooks, NOW - 1_000), blocked);
        assert_eq!(activity_of(&hooks, NOW), Activity::Working, "the poll is newer: trust it");
        assert_eq!(health_of(&hooks, NOW), Health::Running);
    }

    fn rows(list: &[(&str, &str, &str)]) -> Vec<Session> {
        let rows: Vec<serde_json::Value> = list
            .iter()
            .map(|(id, name, cwd)| {
                serde_json::json!({"id": id, "sessionId": format!("{id}-x"), "name": name, "cwd": cwd, "kind": "background",
                    "startedAt": NOW - 1_000, "state": "working", "status": "idle", "pid": 1})
            })
            .collect();
        parse(&serde_json::Value::Array(rows).to_string()).unwrap()
    }

    /// gaia: home P:\L\Gaia, folders Gaia and the plain NodeApi; larchsys: home P:\L, folders Gaia and Skills.
    fn overlapping() -> Vec<Workspace> {
        let gaia = Workspace::new(
            "gaia",
            r"P:\L\Gaia",
            vec![Folder::new("Gaia", r"P:\L\Gaia"), Folder { kind: FolderKind::Plain, ..Folder::new("NodeApi", r"P:\L\NodeApi") }],
        );
        let larchsys = Workspace::new("larchsys", r"P:\L", vec![Folder::new("Gaia", r"P:\L\Gaia"), Folder::new("Skills", r"P:\L\Skills")]);
        vec![gaia, larchsys]
    }

    fn find<'a>(w: &'a WorkspaceNode, name: &str) -> &'a SessionRow {
        w.rows().find(|r| r.session.name == name).unwrap()
    }

    fn folders(w: &WorkspaceNode) -> Vec<(String, bool, usize)> {
        w.repos.iter().map(|r| (r.name.clone(), r.listed, r.sessions.len())).collect()
    }

    fn f(name: &str, listed: bool, n: usize) -> (String, bool, usize) {
        (name.to_string(), listed, n)
    }

    #[test]
    fn a_folder_enabled_in_two_workspaces_shows_in_both_and_later_occurrences_name_the_first() {
        let ws = overlapping();
        let sessions = rows(&[("aaaa0101", "node bindings", r"P:\L\NodeApi\src"), ("aaaa0102", "gaia-ui", r"P:\L\Gaia"), ("aaaa0103", "loose", r"P:\L")]);
        let tree = build(&ws, &sessions, NOW, false, &Supervised::default());
        let (gaia, larchsys) = (&tree.workspaces[0], &tree.workspaces[1]);
        assert_eq!(folders(gaia), [f("Gaia", true, 1), f("NodeApi", true, 1)]);
        assert_eq!((gaia.repos[0].supervised_in.as_deref(), gaia.repos[0].also_in.as_slice()), (None, &["larchsys".to_string()][..]));
        assert_eq!((gaia.repos[1].kind, gaia.repos[1].also_in.len()), (FolderKind::Plain, 0));
        assert_eq!(
            folders(larchsys),
            [f("Gaia", true, 1), f("Skills", true, 0), f("L", false, 1)],
            "the home's folders with sessions follow the listed ones; NodeApi's session shows only in gaia, which enables it"
        );
        assert_eq!(larchsys.repos[2].path, PathBuf::from(r"P:\L"), "a session in the home itself");
        assert_eq!(larchsys.repos[0].supervised_in.as_deref(), Some("gaia"));

        assert_eq!(find(gaia, "gaia-ui").also_in, None, "the first workspace keeps its keys");
        assert_eq!(find(larchsys, "gaia-ui").also_in.as_deref(), Some("gaia"));
        assert_eq!(find(larchsys, "gaia-ui").scope("larchsys"), "@larchsys");
        assert_eq!(find(gaia, "gaia-ui").scope("gaia"), "");
        assert_eq!(find(gaia, "node bindings").also_in, None, "shown once");
        assert_eq!(find(larchsys, "loose").also_in, None, "shown once");
    }

    /// René's registry: three workspaces, the scanned larchsys holding the other two's folders disabled.
    fn renes() -> Vec<Workspace> {
        let off = |name: &str| Folder { enabled: false, ..Folder::new(name, format!(r"P:\L\{name}")) };
        let on = |name: &str| Folder::new(name, format!(r"P:\L\{name}"));
        let supraman = Workspace { home: None, ..Workspace::new("supraman", r"P:\L\AutoWork", vec![on("AutoWork"), on("Skills")]) };
        let gaia = Workspace::new("gaia", r"P:\L\Gaia", vec![on("Gaia"), Folder { kind: FolderKind::Plain, ..on("NodeApi") }, on("node-api-dotnet")]);
        let larchsys = Workspace::new("larchsys", r"P:\L", vec![off("AutoWork"), off("Gaia"), on("Healthy"), off("Old"), off("Quiet"), off("Skills")]);
        vec![supraman, gaia, larchsys]
    }

    #[test]
    fn a_folder_unchecked_here_and_enabled_elsewhere_leaves_this_workspace_with_its_sessions() {
        let ws = renes();
        let sessions = rows(&[
            ("aaaa0301", "aw", r"P:\L\AutoWork"),
            ("aaaa0302", "aw-unit", r"P:\L\AutoWork.worktrees\m5-64"),
            ("aaaa0303", "skills", r"P:\L\Skills\src"),
            ("aaaa0304", "gaia-ui", r"P:\L\Gaia"),
            ("aaaa0305", "node", r"P:\L\NodeApi"),
            ("aaaa0306", "healthy", r"P:\L\Healthy"),
            ("aaaa0307", "old", r"P:\L\Old"),
            ("aaaa0308", "loose", r"P:\L\Other"),
        ]);
        let known = HashMap::from([(PathBuf::from(r"P:\L\AutoWork.worktrees\m5-64"), Some(PathBuf::from(r"P:\L\AutoWork")))]);
        let sup = Supervised { main_repos: Some(&known), ..Supervised::default() };
        let tree = build(&ws, &sessions, NOW, false, &sup);
        let (supraman, gaia, larchsys) = (&tree.workspaces[0], &tree.workspaces[1], &tree.workspaces[2]);
        assert_eq!(folders(supraman), [f("AutoWork", true, 2), f("Skills", true, 1)]);
        assert_eq!(folders(gaia), [f("Gaia", true, 1), f("NodeApi", true, 1), f("node-api-dotnet", true, 0)]);
        assert_eq!(
            folders(larchsys),
            [f("Healthy", true, 1), f("Other", false, 1), f("Old", true, 1)],
            "AutoWork, Gaia and Skills are enabled elsewhere; Old is disabled and nobody else's, so it shows while it holds a session"
        );
        assert!(tree.workspaces.iter().flat_map(|w| w.rows()).all(|r| r.also_in.is_none()), "every session shows once");
        assert!(tree.workspaces.iter().flat_map(|w| &w.repos).all(|r| r.also_in.is_empty()), "no folder is enabled in two");
        let old = &larchsys.repos[2];
        assert_eq!((old.enabled, old.supervised_in.as_deref()), (false, None));

        let all = build(&ws, &sessions, NOW, false, &Supervised { disabled_folders: true, ..sup });
        let parked: Vec<(&str, Option<&str>, usize)> =
            all.workspaces[2].repos.iter().filter(|r| !r.enabled && r.listed).map(|r| (r.name.as_str(), r.supervised_in.as_deref(), r.sessions.len())).collect();
        assert_eq!(parked, [("AutoWork", Some("supraman"), 0), ("Gaia", Some("gaia"), 0), ("Old", None, 1), ("Quiet", None, 0), ("Skills", Some("supraman"), 0)]);
        let found = crate::search::search(&all, "autowork");
        let hit = |w: &str| found.workspaces.iter().find(|h| h.name == w).and_then(|h| h.repos.iter().find(|r| r.name == "AutoWork")).map(|r| (r.supervised_in.clone(), r.disabled, r.sessions.len()));
        assert_eq!(hit("larchsys"), Some((Some("supraman".to_string()), false, 0)), "search finds it in larchsys, labelled with its workspace");
        assert_eq!(hit("supraman"), Some((None, false, 0)));
        let quiet = crate::search::search(&all, "quiet");
        assert!(quiet.workspaces[0].repos[0].disabled && quiet.workspaces[0].repos[0].supervised_in.is_none());
    }

    #[test]
    fn the_workspace_that_wants_a_session_keeps_its_keys_and_its_rows_sit_by_role_and_folder() {
        let ws = overlapping();
        let sessions = rows(&[
            ("aaaa0201", "fm-larchsys-workspace", r"P:\L"),
            ("aaaa0202", "fm-larchsys-relay", r"P:\L"),
            ("aaaa0203", "fm-larchsys-gaia", r"P:\L\Gaia"),
            ("aaaa0204", "mine", r"P:\L\Gaia"),
            ("aaaa0205", "fm-larchsys-park", r"P:\L\Skills"),
        ]);
        let wanted = ["fm-larchsys-relay", "fm-larchsys-workspace", "fm-larchsys-gaia", "fm-larchsys-park"];
        let desired = BTreeMap::from([("larchsys".to_string(), (true, wanted.map(String::from).to_vec()))]);
        let sup = Supervised { desired: Some(&desired), ..Supervised::default() };
        let tree = build(&ws, &sessions, NOW, false, &sup);
        let (gaia, larchsys) = (&tree.workspaces[0], &tree.workspaces[1]);
        let name = |r: Option<&SessionRow>| r.map(|r| r.session.name.clone());
        assert_eq!(name(larchsys.own.as_ref()).as_deref(), Some("fm-larchsys-workspace"), "the workspace row's own session");
        assert_eq!(name(larchsys.relay.as_ref()).as_deref(), Some("fm-larchsys-relay"));
        let block: Vec<&str> = larchsys.supervised.iter().map(|r| r.session.name.as_str()).collect();
        assert_eq!(block, ["fm-larchsys-park"], "a wanted session of no folder stays in the block; the workspace session and relay have no row");
        assert!(gaia.own.is_none() && gaia.relay.is_none());
        assert_eq!(names(&larchsys.repos[0]), ["fm-larchsys-gaia", "mine"], "the folder's supervised session first");
        assert_eq!(names(&gaia.repos[0]), ["fm-larchsys-gaia", "mine"]);
        assert_eq!(find(larchsys, "fm-larchsys-gaia").also_in, None, "larchsys wants it");
        assert_eq!(find(gaia, "fm-larchsys-gaia").also_in.as_deref(), Some("larchsys"));
        assert_eq!(find(gaia, "mine").also_in, None, "first shown in gaia");
        assert_eq!(find(larchsys, "mine").also_in.as_deref(), Some("gaia"));
        assert_eq!(tree.workspaces.iter().map(WorkspaceNode::session_count).collect::<Vec<_>>(), [2, 3], "rows under the workspace row");
        assert_eq!(tree.workspaces[1].rows().count(), 5);
    }

    #[test]
    fn adopt_is_offered_for_an_unmanaged_background_session_of_an_enabled_folder_without_a_supervised_one() {
        let all = sessions();
        let ws = [workspace()];
        let none = BTreeMap::new();
        let row_of = |name: &str| {
            let s = all.iter().find(|s| s.name == name).unwrap().clone();
            let health = s.health(NOW);
            SessionRow::new(s, health)
        };
        let adopt = |folder: &str, row: &SessionRow, sessions: &[Session], desired: &BTreeMap<String, (bool, Vec<String>)>| {
            adoption(&ws, "demo", folder, row, sessions, desired)
        };
        let offer = Some(Adoption::Offer { name: "fm-demo-alpha".into() });
        assert_eq!(adopt("Alpha", &row_of("busy"), &all, &none), offer);
        assert_eq!(adopt("Alpha", &row_of("parked"), &all, &none), offer, "blocked: running");
        assert_eq!(adopt("Beta", &row_of("done"), &all, &none), Some(Adoption::Offer { name: "fm-demo-beta".into() }), "stopped");
        assert_eq!(adopt("Beta", &row_of("gone"), &all, &none), None, "orphaned: the daemon may still respawn it");
        assert_eq!(adopt("Beta", &row_of("term"), &all, &none), None, "interactive: it runs in someone's terminal");
        assert_eq!(adopt("Off", &row_of("off"), &all, &none), None, "disabled folder");
        assert_eq!(adopt("Nope", &row_of("busy"), &all, &none), None);
        let mut foreign = row_of("busy");
        foreign.foreign = true;
        assert_eq!(adopt("Alpha", &foreign, &all, &none), None, "another Supraman's");
        let mut closed = row_of("done");
        closed.closed = true;
        assert_eq!(adopt("Beta", &closed, &all, &none), None);
        let mut second = row_of("busy");
        second.also_in = Some("other".into());
        assert_eq!(adopt("Alpha", &second, &all, &none), offer, "shown first elsewhere, e.g. under a folder that holds this one");
        let mut sm = row_of("busy");
        sm.session.name = "fm-demo-x".into();
        assert_eq!(adopt("Alpha", &sm, &all, &none), None, "Supraman's own");

        let wanted = BTreeMap::from([("demo".to_string(), (false, vec!["fm-demo-alpha".to_string()]))]);
        assert!(matches!(adopt("Alpha", &row_of("busy"), &all, &wanted), Some(Adoption::No(why)) if why.contains("fm-demo-alpha is wanted already")));
        let mut with_row = all.clone();
        with_row.push(Session { name: "fm-demo-alpha".into(), ..all[3].clone() });
        assert!(matches!(adopt("Alpha", &row_of("busy"), &with_row, &none), Some(Adoption::No(why)) if why.contains("fm-demo-alpha exists already")));

        let other = Workspace::new("first", r"C:\x", vec![Folder::new("Alpha", r"C:\demo\Alpha")]);
        assert_eq!(adoption(&[other, workspace()], "demo", "Alpha", &row_of("busy"), &all, &none), None, "another workspace supervises the folder");
    }
}

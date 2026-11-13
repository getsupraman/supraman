use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use supraman_claude::{Health, Kind, Session, main_repo, place_with};
use supraman_core::folders::{Entries, Entry, FolderEntry, OnDisk};
use supraman_core::paths::{folder_name, path_key};
use supraman_core::time::ago;
use supraman_core::{Event, FolderKind};
use supraman_git_view::occupancy::LiveSession;
use gpui::App;
use supraman_ui::icon::Icon;
use supraman_ui::marks::{Activity, Mark};
use supraman_ui::session::Opening;
use theme::ActiveTheme as _;

/// A top-level entry: a group with its folders, or a folder alone.
#[derive(Debug, Clone)]
pub struct EntryNode {
    /// The group's name, or the folder's [`FolderNode::key`].
    pub name: String,
    pub group: bool,
    /// Scheduled runs' sessions by schedule id ([`supraman_core::schedule::schedule_of_session`]), shown under it.
    pub scheduled: BTreeMap<String, Vec<SessionRow>>,
    pub folders: Vec<FolderNode>,
}

#[derive(Debug, Clone)]
pub struct FolderNode {
    /// Unique in the tree: the folder's name, `<group>/<name>` in a group, with `-2`, `-3`, … for a name taken.
    pub key: String,
    pub name: String,
    pub path: PathBuf,
    pub kind: FolderKind,
    pub branch: Option<String>,
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
    /// `""`, or `@<folder key>` under every row of a path listed before: the end of the row's keys.
    pub scope: String,
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
            scope: String::new(),
        }
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
pub struct TreeInputs<'a> {
    pub hooks: Option<&'a HashMap<String, HookSignal>>,
    pub polled_at: i64,
    /// `None` walks the filesystem instead.
    pub main_repos: Option<&'a HashMap<PathBuf, Option<PathBuf>>>,
    /// Each listed folder's kind and branch by [`path_key`]; `None` reads the disk instead.
    pub disk: Option<&'a HashMap<String, OnDisk>>,
    /// A session `claude agents` lists is shown from there instead.
    pub closed: &'a [Session],
    pub starred: Option<&'a BTreeSet<String>>,
    pub expanded: Option<&'a HashSet<String>>,
    /// The names this home's scheduled runs are named after ([`is_foreign`]).
    pub own_names: &'a [String],
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

/// Each listed folder's kind and branch, read once per poll off the UI thread.
pub fn on_disk(entries: &Entries) -> HashMap<String, OnDisk> {
    entries.folders().map(|f| (path_key(&f.path), supraman_core::folders::on_disk(&f.path))).collect()
}

#[derive(Debug)]
pub struct Tree {
    pub entries: Vec<EntryNode>,
    pub hidden_stopped: usize,
}

impl Tree {
    /// Every folder row with the entry it is under.
    pub fn folders(&self) -> impl Iterator<Item = (&EntryNode, &FolderNode)> {
        self.entries.iter().flat_map(|e| e.folders.iter().map(move |f| (e, f)))
    }

    pub fn folder(&self, key: &str) -> Option<&FolderNode> {
        self.folders().map(|(_, f)| f).find(|f| f.key == key)
    }

    pub fn entry(&self, name: &str) -> Option<&EntryNode> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn rows(&self) -> impl Iterator<Item = &SessionRow> {
        self.entries.iter().flat_map(EntryNode::rows)
    }
}

impl FolderNode {
    /// The folder as a title's first part: `group · name`, or its name alone.
    pub fn place(&self) -> String {
        self.key.replace('/', " · ")
    }
}

impl EntryNode {
    /// The rows under the entry's row.
    pub fn session_count(&self) -> usize {
        self.rows().count()
    }

    pub fn rows(&self) -> impl Iterator<Item = &SessionRow> {
        self.scheduled.values().flatten().chain(self.folders.iter().flat_map(|r| &r.sessions))
    }

    pub fn into_rows(self) -> impl Iterator<Item = SessionRow> {
        self.scheduled.into_values().flatten().chain(self.folders.into_iter().flat_map(|r| r.sessions))
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

pub fn is_foreign(name: &str, own_names: &[String], own_prefix: Option<&str>) -> bool {
    if own_prefix.is_some_and(|p| name.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))) {
        return false;
    }
    supraman_supervisor::is_foreign(name, own_names.iter().map(String::as_str))
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

pub fn build(entries: &Entries, sessions: &[Session], now_ms: i64, show_stopped: bool, sup: &TreeInputs) -> Tree {
    let mut b = Build::new(sessions, now_ms, show_stopped, sup);
    let groups: Vec<&str> = entries.groups().map(|(name, _)| name).collect();
    let (mut scheduled, rest) = b.scheduled(&groups, sessions.to_vec());
    let main_of = |p: &Path| b.main_of(p);
    let (open, gone) = (place_with(&rest, entries, main_of).by_folder, place_with(&b.closed, entries, main_of).by_folder);
    let mut nodes = Vec::new();
    for entry in &entries.list {
        let node = match entry {
            Entry::Folder(f) => {
                let folder = b.folder(None, f, &open, &gone);
                EntryNode { name: folder.key.clone(), group: false, scheduled: BTreeMap::new(), folders: vec![folder] }
            }
            Entry::Group { name, folders } => EntryNode {
                name: name.clone(),
                group: true,
                scheduled: scheduled.remove(name.as_str()).unwrap_or_default(),
                folders: folders.iter().map(|f| b.folder(Some(name), f, &open, &gone)).collect(),
            },
        };
        nodes.push(node);
    }
    Tree { entries: nodes, hidden_stopped: b.hidden.len() }
}

/// One [`build`]'s inputs and what it reads once from them; `hidden` gathers the rows left out.
struct Build<'a> {
    sessions: &'a [Session],
    now_ms: i64,
    show_stopped: bool,
    sup: &'a TreeInputs<'a>,
    closed: Vec<Session>,
    keys: HashSet<String>,
    listed: HashSet<String>,
    hidden: HashSet<String>,
}

impl<'a> Build<'a> {
    fn new(sessions: &'a [Session], now_ms: i64, show_stopped: bool, sup: &'a TreeInputs<'a>) -> Self {
        let mut b = Build { sessions, now_ms, show_stopped, sup, closed: Vec::new(), keys: HashSet::new(), listed: HashSet::new(), hidden: HashSet::new() };
        b.closed = b.closed_shown();
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
            foreign: is_foreign(&session.name, self.sup.own_names, self.sup.own_prefix),
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
        // a Supraman session's transcript (a selftest's, a scheduled run's) is not the user's to resume
        self.sup
            .closed
            .iter()
            .filter(|s| !listed.contains(s.session_id.as_str()))
            .filter(|s| self.starred(&s.session_id) || (!s.name.starts_with(supraman_core::FM_PREFIX) && s.started_at.is_some_and(|t| self.now_ms - t <= CLOSED_WINDOW_MS)))
            .cloned()
            .collect()
    }

    /// The scheduled runs' rows by group and schedule id, and the sessions that are not runs.
    fn scheduled<'g>(&mut self, groups: &[&'g str], sessions: Vec<Session>) -> (HashMap<&'g str, BTreeMap<String, Vec<SessionRow>>>, Vec<Session>) {
        let mut scheduled: HashMap<&str, BTreeMap<String, Vec<SessionRow>>> = HashMap::new();
        let mut rest = Vec::new();
        for s in sessions {
            let run = groups.iter().find_map(|g| supraman_core::schedule::schedule_of_session(g, &s.name).map(|id| (*g, id.to_string())));
            match run {
                Some((group, id)) => {
                    let r = self.row(s);
                    if self.keep(&r) {
                        scheduled.entry(group).or_default().entry(id).or_default().push(r);
                    }
                }
                None => rest.push(s),
            }
        }
        scheduled.values_mut().flat_map(|by_id| by_id.values_mut()).for_each(|rows| sort(rows));
        (scheduled, rest)
    }

    fn unique_key(&mut self, base: String) -> String {
        let key = if self.keys.contains(&base) { (2..).map(|i| format!("{base}-{i}")).find(|k| !self.keys.contains(k)).unwrap() } else { base };
        self.keys.insert(key.clone());
        key
    }

    /// A listed folder's row with the sessions it holds; a path listed before shows them again, its keys scoped.
    fn folder(&mut self, group: Option<&str>, f: &FolderEntry, open: &HashMap<String, Vec<Session>>, gone: &HashMap<String, Vec<Session>>) -> FolderNode {
        let name = folder_name(&f.path);
        let key = self.unique_key(group.map_or_else(|| name.clone(), |g| format!("{g}/{name}")));
        let at = path_key(&f.path);
        let held = |by: &HashMap<String, Vec<Session>>| by.get(&at).cloned().unwrap_or_default();
        let (mut sessions, more) = self.place(&key, held(open), held(gone));
        if !self.listed.insert(at.clone()) {
            sessions.iter_mut().for_each(|r| r.scope = format!("@{key}"));
        }
        let disk = match self.sup.disk {
            Some(known) => known.get(&at).cloned(),
            None => Some(supraman_core::folders::on_disk(&f.path)),
        };
        let OnDisk { kind, branch } = disk.unwrap_or(OnDisk { kind: FolderKind::Plain, branch: None });
        FolderNode { key, name, path: f.path.clone(), kind, branch, sessions, more }
    }

    /// A folder's rows: the open ones, then the closed ones up to [`CLOSED_CAP`]; and how many that cap left out.
    fn place(&mut self, key: &str, open: Vec<Session>, closed: Vec<Session>) -> (Vec<SessionRow>, usize) {
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
        let all = self.sup.expanded.is_some_and(|e| e.contains(key));
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
        (rows, more)
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

pub fn short_id(session: &Session) -> &str {
    session.id.get(..8).unwrap_or(&session.id)
}

pub fn display_name(session: &Session) -> &str {
    if session.name.is_empty() { short_id(session) } else { &session.name }
}

/// The listed folder holding `active` (the deepest), else the first listed folder, else `home`.
pub fn shell_cwd(active: Option<&Path>, entries: &Entries, home: Option<PathBuf>) -> Option<PathBuf> {
    let holder = active.and_then(|cwd| supraman_claude::holding(cwd, entries, |_| None));
    holder.or(entries.folders().next()).map(|f| f.path.clone()).or(home)
}

#[cfg(test)]
mod tests;

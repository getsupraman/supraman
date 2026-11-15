use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use supraman_claude::{Health, Kind, Session, main_repo, place_with};
use supraman_core::folders::{Entries, Entry, FolderEntry, OnDisk, Pos, Shown};
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
    /// Its scheduled runs' sessions by schedule id ([`supraman_core::schedule::schedule_of_session`]), shown under
    /// its Schedules; only the row [`Tree::schedule_row`] names has them.
    pub scheduled: BTreeMap<String, Vec<SessionRow>>,
    /// Its place in folders.toml; `None` for a shown child folder.
    pub pos: Option<Pos>,
    /// A shown child's: the key of the folder row it shows under.
    pub parent: Option<String>,
    pub shows_subfolders: bool,
    /// A shown child its parent excludes: its row only with the stopped sessions shown, and no sessions.
    pub excluded: bool,
}

impl FolderNode {
    pub fn bare(key: String, name: String, path: PathBuf, kind: FolderKind, branch: Option<String>) -> Self {
        FolderNode { key, name, path, kind, branch, sessions: Vec::new(), more: 0, scheduled: BTreeMap::new(), pos: None, parent: None, shows_subfolders: false, excluded: false }
    }
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
    /// The listed and shown folders' kinds and the shown children; `None` reads the disk instead.
    pub disk: Option<&'a Disk>,
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

/// What the disk says of the listed folders: the children shown under those that show them, and each listed or
/// shown folder's kind and branch by [`path_key`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Disk {
    pub kinds: HashMap<String, OnDisk>,
    pub shown: Shown,
}

/// Read once per poll off the UI thread.
pub fn on_disk(entries: &Entries) -> Disk {
    let shown = supraman_core::folders::shown(entries);
    let read = entries.folders().map(|f| f.path.as_path()).chain(shown.values().flatten().map(PathBuf::as_path));
    let kinds = read.map(|p| (path_key(p), supraman_core::folders::on_disk(p))).collect();
    Disk { kinds, shown }
}

/// The name and folder key of the entry holding the sessions no listed or shown folder holds; `:` is in no folder's
/// or group's name.
pub const UNLISTED: &str = ":unlisted";
pub const UNLISTED_TITLE: &str = "Other sessions";
/// The name of the entry holding the runs of schedules whose folder has no row: a folder each, named by its folder key.
pub const UNROWED: &str = ":unrowed";

#[derive(Debug, Default)]
pub struct Tree {
    pub entries: Vec<EntryNode>,
    /// Other sessions: one folder of the sessions no listed or shown folder holds; `None` while it has none.
    pub unlisted: Option<EntryNode>,
    /// The runs of schedules whose folder has no row ([`Tree::schedule_row`]), shown under Other schedules.
    pub unrowed: Option<EntryNode>,
    pub hidden_stopped: usize,
    pub(crate) schedule_rows: BTreeMap<String, (usize, usize)>,
}

impl Tree {
    /// Every folder row with the entry it is under.
    pub fn folders(&self) -> impl Iterator<Item = (&EntryNode, &FolderNode)> {
        self.entries.iter().flat_map(|e| e.folders.iter().map(move |f| (e, f)))
    }

    pub fn folder(&self, key: &str) -> Option<&FolderNode> {
        self.folders().map(|(_, f)| f).find(|f| f.key == key)
    }

    /// The entry a session in `cwd` belongs to, its listed folder's, and the folder it shows under
    /// ([`supraman_claude::holding`]).
    pub fn holding(&self, entries: &Entries, shown: &Shown, cwd: &Path) -> Option<(String, PathBuf)> {
        let at = supraman_claude::holding(cwd, entries, shown, |_| None)?;
        let key = path_key(&at.listed.path);
        let (entry, _) = self.folders().find(|(_, f)| f.listed() && path_key(&f.path) == key)?;
        Some((entry.name.clone(), at.folder))
    }

    pub fn entry(&self, name: &str) -> Option<&EntryNode> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Every node holding session rows: the entries, then Other sessions, then the runs of Other schedules.
    pub fn nodes(&self) -> impl Iterator<Item = &EntryNode> {
        self.entries.iter().chain(&self.unlisted).chain(&self.unrowed)
    }

    /// Where the schedules filed under `folder_key` show: the first row of their folder's path that is not excluded,
    /// at any depth; `None` puts them under Other schedules.
    pub fn schedule_row(&self, folder_key: &str) -> Option<(usize, usize)> {
        self.schedule_rows.get(folder_key).copied()
    }

    /// The runs of the schedules filed under `folder_key` that have no row.
    pub fn unrowed_runs(&self, folder_key: &str) -> Option<&BTreeMap<String, Vec<SessionRow>>> {
        self.unrowed.as_ref()?.folders.iter().find(|f| f.name == folder_key).map(|f| &f.scheduled)
    }

    /// The folder keys whose runs have no row.
    pub fn unrowed_keys(&self) -> impl Iterator<Item = &str> {
        self.unrowed.iter().flat_map(|e| e.folders.iter().map(|f| f.name.as_str()))
    }

    /// The `e`th of [`Self::nodes`].
    pub fn node(&self, e: usize) -> &EntryNode {
        self.entries.get(e).unwrap_or_else(|| self.unlisted.as_ref().expect("a node index past the entries is Other sessions"))
    }

    pub fn rows(&self) -> impl Iterator<Item = &SessionRow> {
        self.nodes().flat_map(EntryNode::rows)
    }
}

impl FolderNode {
    /// A row of folders.toml, not a shown child.
    pub fn listed(&self) -> bool {
        self.pos.is_some()
    }

    pub fn row_place(&self, row: &SessionRow) -> String {
        row_place(&self.key, row)
    }

    /// The folder as a title's first part: `group · name`, or its name alone.
    pub fn place(&self) -> String {
        self.key.replace('/', " · ")
    }
}

impl EntryNode {
    /// Other sessions, not a listed entry.
    pub fn unlisted(&self) -> bool {
        self.name == UNLISTED
    }

    /// Other sessions or the runs of Other schedules: no folder of the user's.
    pub fn apart(&self) -> bool {
        self.unlisted() || self.name == UNROWED
    }

    /// The rows under the entry's row.
    pub fn session_count(&self) -> usize {
        self.rows().count()
    }

    pub fn rows(&self) -> impl Iterator<Item = &SessionRow> {
        self.folders.iter().flat_map(|f| f.scheduled.values().flatten().chain(&f.sessions))
    }

    pub fn into_rows(self) -> impl Iterator<Item = SessionRow> {
        self.folders.into_iter().flat_map(|f| f.scheduled.into_values().flatten().chain(f.sessions))
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
    let read;
    let disk = match sup.disk {
        Some(disk) => disk,
        None => {
            read = on_disk(entries);
            &read
        }
    };
    let rows: HashSet<String> = entries.placed(&disk.shown).map(supraman_core::schedule::folder_key).collect();
    let rest = b.scheduled(|key| rows.contains(key) || sup.own_names.iter().any(|k| k == key), sessions.to_vec());
    let main_of = |p: &Path| b.main_of(p);
    let (open, gone) = (place_with(&rest, entries, &disk.shown, main_of), place_with(&b.closed, entries, &disk.shown, main_of));
    let placed = Placed { open: open.by_folder, gone: gone.by_folder, disk };
    let mut nodes = Vec::new();
    for (e, entry) in entries.list.iter().enumerate() {
        let node = match entry {
            Entry::Folder(f) => {
                let folders = b.listed(None, f, Pos::top(e), &placed);
                EntryNode { name: folders[0].key.clone(), group: false, folders }
            }
            Entry::Group { name, folders } => EntryNode {
                name: name.clone(),
                group: true,
                folders: folders.iter().enumerate().flat_map(|(i, f)| b.listed(Some(name), f, Pos::member(e, i), &placed)).collect(),
            },
        };
        nodes.push(node);
    }
    let unlisted = b.unlisted(open.unplaced, gone.unplaced);
    let schedule_rows = schedule_rows(&nodes);
    let mut unrowed = Vec::new();
    for (key, runs) in std::mem::take(&mut b.runs) {
        match schedule_rows.get(&key) {
            Some(&(e, f)) => nodes[e].folders[f].scheduled = runs,
            None => unrowed.push(FolderNode { scheduled: runs, ..FolderNode::bare(format!("{UNROWED}/{key}"), key, PathBuf::new(), FolderKind::Plain, None) }),
        }
    }
    unrowed.sort_by(|a, b| a.name.cmp(&b.name));
    let unrowed = (!unrowed.is_empty()).then(|| EntryNode { name: UNROWED.into(), group: false, folders: unrowed });
    Tree { entries: nodes, unlisted, unrowed, hidden_stopped: b.hidden.len(), schedule_rows }
}

/// The one rule of where a folder's schedules show: by folder key, the first row of that path, excluded rows aside.
fn schedule_rows(nodes: &[EntryNode]) -> BTreeMap<String, (usize, usize)> {
    let mut out = BTreeMap::new();
    for (e, node) in nodes.iter().enumerate() {
        for (f, folder) in node.folders.iter().enumerate().filter(|(_, f)| !f.excluded) {
            out.entry(supraman_core::schedule::folder_key(&folder.path)).or_insert((e, f));
        }
    }
    out
}

/// The sessions placed by folder, open and closed, and what the disk says of the folders.
struct Placed<'a> {
    open: HashMap<String, Vec<Session>>,
    gone: HashMap<String, Vec<Session>>,
    disk: &'a Disk,
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
    /// Scheduled runs' rows by folder key and schedule id, each given to [`schedule_rows`]' row or Other schedules.
    runs: HashMap<String, BTreeMap<String, Vec<SessionRow>>>,
}

impl<'a> Build<'a> {
    fn new(sessions: &'a [Session], now_ms: i64, show_stopped: bool, sup: &'a TreeInputs<'a>) -> Self {
        let mut b = Build { sessions, now_ms, show_stopped, sup, closed: Vec::new(), keys: HashSet::new(), listed: HashSet::new(), hidden: HashSet::new(), runs: HashMap::new() };
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

    /// The scheduled runs of a folder key `ours` claims into [`Self::runs`]; the sessions that are not, back.
    fn scheduled(&mut self, ours: impl Fn(&str) -> bool, sessions: Vec<Session>) -> Vec<Session> {
        let mut rest = Vec::new();
        for s in sessions {
            let run = supraman_core::schedule::schedule_of_session(&s.name).filter(|(key, _)| ours(key)).map(|(key, id)| (key.to_string(), id.to_string()));
            match run {
                Some((key, id)) => {
                    let r = self.row(s);
                    if self.keep(&r) {
                        self.runs.entry(key).or_default().entry(id).or_default().push(r);
                    }
                }
                None => rest.push(s),
            }
        }
        self.runs.values_mut().flat_map(|by_id| by_id.values_mut()).for_each(|rows| sort(rows));
        rest
    }

    fn unique_key(&mut self, base: String) -> String {
        let key = if self.keys.contains(&base) { (2..).map(|i| format!("{base}-{i}")).find(|k| !self.keys.contains(k)).unwrap() } else { base };
        self.keys.insert(key.clone());
        key
    }

    /// A listed folder's row, then a row for each child it shows.
    fn listed(&mut self, group: Option<&str>, f: &FolderEntry, pos: Pos, placed: &Placed) -> Vec<FolderNode> {
        let name = folder_name(&f.path);
        let base = group.map_or_else(|| name.clone(), |g| format!("{g}/{name}"));
        let mut parent = self.folder(base, &f.path, placed);
        parent.pos = Some(pos);
        parent.shows_subfolders = f.show_subfolders;
        let mut out = vec![parent];
        for child in f.read_children(&placed.disk.shown) {
            let base = format!("{}/{}", out[0].key, folder_name(child));
            let mut node = match f.excludes(child) {
                false => self.folder(base, child, placed),
                true if self.show_stopped => self.excluded(base, child, placed),
                true => continue,
            };
            node.parent = Some(out[0].key.clone());
            out.push(node);
        }
        out
    }

    /// An excluded child's row, shown only with the stopped sessions: no sessions, they show under its parent.
    fn excluded(&mut self, base: String, path: &Path, placed: &Placed) -> FolderNode {
        let key = self.unique_key(base);
        let OnDisk { kind, branch } = placed.disk.kinds.get(&path_key(path)).cloned().unwrap_or(OnDisk { kind: FolderKind::Plain, branch: None });
        FolderNode { excluded: true, ..FolderNode::bare(key, folder_name(path), path.to_path_buf(), kind, branch) }
    }

    /// A folder's row with the sessions it holds; a path shown before shows them again, its keys scoped.
    fn folder(&mut self, base: String, path: &Path, placed: &Placed) -> FolderNode {
        let name = folder_name(path);
        let key = self.unique_key(base);
        let at = path_key(path);
        let held = |by: &HashMap<String, Vec<Session>>| by.get(&at).cloned().unwrap_or_default();
        let (mut sessions, more) = self.place(&key, held(&placed.open), held(&placed.gone));
        if !self.listed.insert(at.clone()) {
            sessions.iter_mut().for_each(|r| r.scope = format!("@{key}"));
        }
        let OnDisk { kind, branch } = placed.disk.kinds.get(&at).cloned().unwrap_or(OnDisk { kind: FolderKind::Plain, branch: None });
        FolderNode { sessions, more, ..FolderNode::bare(key, name, path.to_path_buf(), kind, branch) }
    }

    /// Other sessions' node, its rows placed as a folder's; `None` while none shows.
    fn unlisted(&mut self, open: Vec<Session>, closed: Vec<Session>) -> Option<EntryNode> {
        let (sessions, more) = self.place(UNLISTED, open, closed);
        let folder = FolderNode { sessions, more, ..FolderNode::bare(UNLISTED.into(), UNLISTED_TITLE.into(), PathBuf::new(), FolderKind::Plain, None) };
        (!folder.sessions.is_empty() || more > 0).then(|| EntryNode { name: UNLISTED.into(), group: false, folders: vec![folder] })
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

/// What a row under the folder `key` is titled and its menu placed by: the key, or under Other sessions the row's own
/// folder's name.
pub fn row_place(key: &str, row: &SessionRow) -> String {
    if key == UNLISTED { folder_name(&row.session.cwd) } else { key.to_string() }
}

pub fn short_id(session: &Session) -> &str {
    session.id.get(..8).unwrap_or(&session.id)
}

pub fn display_name(session: &Session) -> &str {
    if session.name.is_empty() { short_id(session) } else { &session.name }
}

/// The folder holding `active` ([`supraman_claude::holding`]), else the first listed folder, else `home`.
pub fn shell_cwd(active: Option<&Path>, entries: &Entries, shown: &Shown, home: Option<PathBuf>) -> Option<PathBuf> {
    let holder = active.and_then(|cwd| supraman_claude::holding(cwd, entries, shown, |_| None)).map(|at| at.folder);
    holder.or_else(|| entries.folders().next().map(|f| f.path.clone())).or(home)
}

#[cfg(test)]
mod tests;

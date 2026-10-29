use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use foreman_claude::{Health, Kind, Session, assign_with, main_repo};
use foreman_core::paths::{normalize, path_key};
use foreman_core::{Event, FolderKind, Workspace, owners};
use foreman_supervisor::{RELAY_ROLE, WORKSPACE_ROLE, relay_session_name, repo_session_name, workspace_session_name};

#[derive(Debug, Clone)]
pub struct WorkspaceNode {
    pub name: String,
    pub home: Option<PathBuf>,
    /// `None`: no desired-state file.
    pub started: Option<bool>,
    /// The workspace session, the relay, and wanted sessions of no folder here.
    pub supervised: Vec<SessionRow>,
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
    /// Another workspace owns the folder's supervised session ([`foreman_core::owners`]).
    pub supervised_in: Option<String>,
    /// The other workspaces that list this folder.
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
    /// [`WORKSPACE_ROLE`] or [`RELAY_ROLE`] for the workspace's own sessions, shown in place of the name.
    pub role: Option<&'static str>,
    /// Set on every occurrence but the first of a session shown under several workspaces: the workspace
    /// that supervises it, else the first that shows it. Such a row's keys end in `@<its workspace>`.
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
            role: None,
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

pub fn ago(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..60 => format!("{s} s ago"),
        60..3600 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

/// A prompt the hook reported after the poll marks the row blocked before the next poll shows it.
pub fn with_hook(health: Health, hook: Option<&HookSignal>, polled_at: i64) -> Health {
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
    /// Keeps a disabled folder that holds no session (search); the tree leaves it out (m5/56).
    pub disabled_folders: bool,
}

/// Filled once per poll, off the UI thread, because [`build`] runs on every frame.
pub fn main_repos(sessions: &[Session]) -> HashMap<PathBuf, Option<PathBuf>> {
    let mut out = HashMap::new();
    for s in sessions {
        out.entry(s.cwd.clone()).or_insert_with(|| main_repo(&s.cwd));
    }
    out
}

pub struct Tree {
    pub workspaces: Vec<WorkspaceNode>,
    pub hidden_stopped: usize,
}

impl WorkspaceNode {
    pub fn session_count(&self) -> usize {
        self.supervised.len() + self.repos.iter().map(|r| r.sessions.len()).sum::<usize>()
    }
}

pub fn attachable(session: &Session, health: &Health) -> bool {
    session.kind == Kind::Background
        && matches!(health, Health::Running | Health::Blocked { .. } | Health::Idle)
}

pub fn is_alive(health: &Health) -> bool {
    matches!(health, Health::Running | Health::Blocked { .. } | Health::Idle)
}

pub fn is_foreign(name: &str, workspaces: &[Workspace], desired: Option<&BTreeMap<String, (bool, Vec<String>)>>) -> bool {
    let wanted = desired.into_iter().flat_map(|d| d.values()).flat_map(|(_, wanted)| wanted.iter().map(String::as_str));
    foreman_supervisor::is_foreign(name, workspaces.iter().map(|w| w.name.as_str()), wanted)
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

pub fn build(workspaces: &[Workspace], sessions: &[Session], now_ms: i64, show_stopped: bool, sup: &Supervised) -> Tree {
    let mut hidden: HashSet<String> = HashSet::new();
    let starred = |id: &str| sup.starred.is_some_and(|s| s.contains(id));
    let row = |session: Session| {
        let hook = sup.hooks.and_then(|h| h.get(&session.session_id)).cloned();
        let health = with_hook(session.supervised_health(now_ms), hook.as_ref(), sup.polled_at);
        SessionRow {
            foreign: is_foreign(&session.name, workspaces, sup.desired),
            starred: starred(&session.session_id),
            hook,
            ..SessionRow::new(session, health)
        }
    };
    let listed: HashSet<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
    // a Foreman session's transcript (a selftest's, a supervised one removed) is not the user's to resume
    let closed: Vec<Session> = sup
        .closed
        .iter()
        .filter(|s| !listed.contains(s.session_id.as_str()))
        .filter(|s| starred(&s.session_id) || (!s.name.starts_with(foreman_core::FM_PREFIX) && s.started_at.is_some_and(|t| now_ms - t <= CLOSED_WINDOW_MS)))
        .cloned()
        .collect();
    let main_of = |cwd: &Path| match sup.main_repos {
        Some(known) => known.get(cwd).cloned().flatten(),
        None => main_repo(cwd),
    };
    let owner_of = owners(workspaces);
    let mut members: HashMap<String, Vec<&str>> = HashMap::new();
    for w in workspaces {
        for f in &w.folders {
            members.entry(path_key(&f.path)).or_default().push(&w.name);
        }
    }
    let wanted_of = |name: &str| match sup.desired.and_then(|d| d.get(name)) {
        Some((started, wanted)) => (Some(*started), wanted.as_slice()),
        None => (None, &[][..]),
    };
    let mut nodes: Vec<WorkspaceNode> = workspaces
        .iter()
        .map(|workspace| {
            let (started, wanted) = wanted_of(&workspace.name);
            let (theirs, rest): (Vec<Session>, Vec<Session>) =
                sessions.iter().cloned().partition(|s| wanted.contains(&s.name));
            let (own, relay) = (workspace_session_name(&workspace.name), relay_session_name(&workspace.name));
            let folder_of_wanted = |name: &str| {
                workspace.folders.iter().find(|f| repo_session_name(&workspace.name, &f.name).as_deref() == Some(name)).map(|f| f.name.clone())
            };
            let mut supervised: Vec<SessionRow> = Vec::new();
            let mut first_in: HashMap<String, Vec<SessionRow>> = HashMap::new();
            for s in theirs {
                let role = if s.name == own { Some(WORKSPACE_ROLE) } else if s.name == relay { Some(RELAY_ROLE) } else { None };
                match folder_of_wanted(&s.name).filter(|_| role.is_none()) {
                    Some(folder) => first_in.entry(folder).or_default().push(row(s)),
                    None => supervised.push(SessionRow { role, ..row(s) }),
                }
            }
            sort(&mut supervised);
            let assigned = assign_with(&rest, workspace, &main_of);
            let mut by_repo = assigned.by_repo;
            let gone = assign_with(&closed, workspace, &main_of);
            let mut closed_by_repo = gone.by_repo;
            let key = |path: &Path| path_key(path);
            let others = |path: &Path| -> Vec<String> {
                members.get(&key(path)).into_iter().flatten().filter(|n| **n != workspace.name).map(|n| n.to_string()).collect()
            };
            let mut place = |name: &str, open: Vec<Session>, closed: Vec<Session>, wanted: Vec<SessionRow>| {
                let mut rows: Vec<SessionRow> = open
                    .into_iter()
                    .map(row)
                    .filter(|row| {
                        let keep = show_stopped || row.health != Health::Stopped || row.starred;
                        if !keep {
                            hidden.insert(row.session.session_id.clone());
                        }
                        keep
                    })
                    .collect();
                sort(&mut rows);
                let mut gone: Vec<SessionRow> = closed.into_iter().map(|s| SessionRow { closed: true, ..row(s) }).collect();
                gone.sort_by(|a, b| b.session.started_at.cmp(&a.session.started_at));
                let all = sup.expanded.is_some_and(|e| e.contains(&format!("{}/{name}", workspace.name)));
                let (mut shown, mut more) = (0, 0);
                for r in gone {
                    if r.starred {
                        rows.push(r);
                    } else if !show_stopped {
                        hidden.insert(r.session.session_id.clone());
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
            };
            let mut repos: Vec<RepoNode> = Vec::new();
            let mut parked: Vec<RepoNode> = Vec::new();
            for folder in &workspace.folders {
                let supervised_in = owner_of.get(&key(&folder.path)).filter(|o| **o != workspace.name).cloned();
                let node = |sessions: Vec<SessionRow>, more: usize| RepoNode {
                    name: folder.name.clone(),
                    path: folder.path.clone(),
                    kind: folder.kind,
                    branch: folder.branch.clone(),
                    enabled: folder.enabled,
                    listed: true,
                    supervised_in: supervised_in.clone(),
                    also_in: others(&folder.path),
                    sessions,
                    more,
                };
                let (sessions, more) = place(
                    &folder.name,
                    by_repo.remove(&folder.name).unwrap_or_default(),
                    closed_by_repo.remove(&folder.name).unwrap_or_default(),
                    first_in.remove(&folder.name).unwrap_or_default(),
                );
                if folder.enabled || supervised_in.is_some() {
                    repos.push(node(sessions, more));
                } else if !sessions.is_empty() || more > 0 || sup.disabled_folders {
                    parked.push(node(sessions, more));
                }
            }
            if let Some(home) = workspace.home() {
                let mut groups: BTreeMap<String, (PathBuf, Vec<Session>, Vec<Session>)> = BTreeMap::new();
                for (s, is_closed) in assigned.in_home.into_iter().map(|s| (s, false)).chain(gone.in_home.into_iter().map(|s| (s, true))) {
                    let (name, path) = home_folder(home, &s.cwd);
                    let group = groups.entry(name).or_insert_with(|| (path, Vec::new(), Vec::new()));
                    if is_closed { group.2.push(s) } else { group.1.push(s) }
                }
                for (name, (path, open, gone)) in groups {
                    let (sessions, more) = place(&name, open, gone, Vec::new());
                    if sessions.is_empty() && more == 0 {
                        continue;
                    }
                    repos.push(RepoNode {
                        also_in: others(&path),
                        name,
                        path,
                        kind: FolderKind::Plain,
                        branch: None,
                        enabled: false,
                        listed: false,
                        supervised_in: None,
                        sessions,
                        more,
                    });
                }
            }
            repos.extend(parked);
            supervised.extend(first_in.into_values().flatten());
            WorkspaceNode {
                name: workspace.name.clone(),
                home: workspace.home().map(Path::to_path_buf),
                started,
                supervised,
                repos,
            }
        })
        .collect();
    mark_second_occurrences(&mut nodes, sup.desired);
    Tree {
        workspaces: nodes,
        hidden_stopped: hidden.len(),
    }
}

/// The folder of the home that `cwd` lies in, named after its first path segment below the home; the home
/// itself, named after its own folder, for a session in the home.
fn home_folder(home: &Path, cwd: &Path) -> (String, PathBuf) {
    let (home_n, cwd_n) = (normalize(home), normalize(cwd));
    let below = cwd_n.get(home_n.len()..).unwrap_or("").trim_start_matches('/');
    match below.split('/').next().filter(|s| !s.is_empty()) {
        Some(first) => (first.to_string(), home.join(first)),
        None => (home.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| home_n.clone()), home.to_path_buf()),
    }
}

/// A session under several workspaces: the occurrence in the workspace that wants it, else the first, keeps
/// its keys; the others name that workspace.
fn mark_second_occurrences(nodes: &mut [WorkspaceNode], desired: Option<&BTreeMap<String, (bool, Vec<String>)>>) {
    let wants = |ws: &str, name: &str| desired.and_then(|d| d.get(ws)).is_some_and(|(_, wanted)| wanted.iter().any(|w| w == name));
    let mut seen: HashMap<String, Vec<(usize, bool)>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        for r in n.supervised.iter().chain(n.repos.iter().flat_map(|f| &f.sessions)) {
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
            for r in n.supervised.iter_mut().chain(n.repos.iter_mut().flat_map(|f| f.sessions.iter_mut())) {
                if r.session.session_id == id {
                    r.also_in = Some(names[owner].clone());
                }
            }
        }
    }
}

pub fn health_label(health: &Health) -> String {
    match health {
        Health::Orphaned { .. } => "process gone".to_string(),
        Health::Blocked { .. } => "blocked".to_string(),
        other => other.to_string(),
    }
}

pub fn short_id(session: &Session) -> &str {
    session.id.get(..8).unwrap_or(&session.id)
}

pub fn display_name(session: &Session) -> &str {
    if session.name.is_empty() { short_id(session) } else { &session.name }
}

/// The home of the workspace holding `active` (the deepest home or folder containing it), else of the first
/// workspace, else `home`.
pub fn shell_cwd(active: Option<&Path>, workspaces: &[Workspace], home: Option<PathBuf>) -> Option<PathBuf> {
    let holder = active.and_then(|cwd| holder_of(cwd, workspaces));
    holder.or(workspaces.first()).and_then(|w| w.home()).map(Path::to_path_buf).or(home)
}

/// The workspace whose home or folder holds `cwd`, the deepest match winning.
pub fn holder_of<'a>(cwd: &Path, workspaces: &'a [Workspace]) -> Option<&'a Workspace> {
    let depth = |w: &Workspace| {
        w.home()
            .into_iter()
            .chain(w.folders.iter().map(|f| f.path.as_path()))
            .filter(|d| foreman_core::paths::within(cwd, d))
            .map(|d| path_key(d).len())
            .max()
    };
    workspaces.iter().filter_map(|w| depth(w).map(|d| (d, w))).max_by_key(|(d, _)| *d).map(|(_, w)| w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreman_claude::parse;
    use foreman_core::Folder;

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
    fn labels() {
        let tree = build(&[workspace()], &sessions(), NOW, true, &Supervised::default());
        let beta = &tree.workspaces[0].repos[1];
        assert_eq!(health_label(&beta.sessions[1].health), "process gone");
        let alpha = &tree.workspaces[0].repos[0];
        assert_eq!(health_label(&alpha.sessions[0].health), "blocked");
        assert_eq!(
            alpha.sessions[0].health,
            Health::Blocked {
                waiting_for: Some("permission prompt".into())
            }
        );
        assert_eq!(short_id(&beta.sessions[0].session), "bbbb0005");
    }

    #[test]
    fn foreman_session_waiting_on_a_peer_is_idle_a_parked_one_blocked() {
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
        let foreign = |name: &str| is_foreign(name, &ws, Some(&desired));
        assert!(foreign("fm-selftest-abc123-park"), "a selftest's session");
        assert!(foreign("fm-other-workspace"), "another home's workspace");
        assert!(!foreign("fm-demo-workspace"), "wanted here");
        assert!(!foreign("fm-x-workspace"), "wanted here, whatever its name");
        assert!(!foreign("fm-demo-alpha"), "named after a workspace of this home: retired, or wanted before the first tick");
        assert!(!is_foreign("fm-demo-alpha", &ws, None));
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
        foreman_claude::Transcript {
            session_id: id.into(),
            cwd: Some(cwd.into()),
            custom_title: Some(name.into()),
            modified_ms: NOW - age_ms,
            ..Default::default()
        }
        .to_session()
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
        assert!(p.supervised.iter().all(|r| r.role.is_none() && r.also_in.is_none()));
        assert_eq!(tree.hidden_stopped, 0, "the stopped wanted row is shown, not hidden");
        let never = build(&[workspace()], &sessions(), NOW, false, &Supervised::default());
        assert_eq!(never.workspaces[0].started, None);
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

    /// gaia: home P:\L\Gaia, folders Gaia (disabled) and the plain NodeApi; larchsys: home P:\L, folders Gaia
    /// and Skills.
    fn overlapping() -> Vec<Workspace> {
        let gaia = Workspace::new(
            "gaia",
            r"P:\L\Gaia",
            vec![
                Folder { enabled: false, ..Folder::new("Gaia", r"P:\L\Gaia") },
                Folder { kind: FolderKind::Plain, ..Folder::new("NodeApi", r"P:\L\NodeApi") },
            ],
        );
        let larchsys = Workspace::new("larchsys", r"P:\L", vec![Folder::new("Gaia", r"P:\L\Gaia"), Folder::new("Skills", r"P:\L\Skills")]);
        vec![gaia, larchsys]
    }

    fn find<'a>(w: &'a WorkspaceNode, name: &str) -> &'a SessionRow {
        w.supervised.iter().chain(w.repos.iter().flat_map(|r| &r.sessions)).find(|r| r.session.name == name).unwrap()
    }

    #[test]
    fn a_session_shows_under_every_workspace_holding_its_cwd_and_later_occurrences_name_the_first() {
        let ws = overlapping();
        let sessions = rows(&[("aaaa0101", "node bindings", r"P:\L\NodeApi\src"), ("aaaa0102", "gaia-ui", r"P:\L\Gaia"), ("aaaa0103", "loose", r"P:\L")]);
        let tree = build(&ws, &sessions, NOW, false, &Supervised::default());
        let (gaia, larchsys) = (&tree.workspaces[0], &tree.workspaces[1]);
        let folders = |w: &WorkspaceNode| w.repos.iter().map(|r| (r.name.clone(), r.listed, r.sessions.len())).collect::<Vec<_>>();
        let f = |name: &str, listed, n| (name.to_string(), listed, n);
        assert_eq!(folders(gaia), [f("Gaia", true, 1), f("NodeApi", true, 1)], "Gaia is supervised in larchsys, so shown with its sessions");
        assert_eq!(gaia.repos[0].supervised_in.as_deref(), Some("larchsys"));
        assert_eq!(gaia.repos[0].also_in, ["larchsys"]);
        assert_eq!((gaia.repos[1].kind, gaia.repos[1].supervised_in.as_deref()), (FolderKind::Plain, None));
        assert_eq!(
            folders(larchsys),
            [f("Gaia", true, 1), f("Skills", true, 0), f("L", false, 1), f("NodeApi", false, 1)],
            "the home's folders with sessions follow the listed ones"
        );
        assert_eq!(larchsys.repos[2].path, PathBuf::from(r"P:\L"), "a session in the home itself");
        assert_eq!(larchsys.repos[3].also_in, ["gaia"], "NodeApi is one of gaia's folders");
        assert_eq!(larchsys.repos[0].supervised_in, None);

        assert_eq!(find(gaia, "node bindings").also_in, None, "the first workspace keeps its keys");
        assert_eq!(find(larchsys, "node bindings").also_in.as_deref(), Some("gaia"));
        assert_eq!(find(larchsys, "node bindings").scope("larchsys"), "@larchsys");
        assert_eq!(find(gaia, "node bindings").scope("gaia"), "");
        assert_eq!(find(larchsys, "loose").also_in, None, "shown once");
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
        let mut roles: Vec<(&str, Option<&str>)> = larchsys.supervised.iter().map(|r| (r.session.name.as_str(), r.role)).collect();
        roles.sort();
        assert_eq!(
            roles,
            [("fm-larchsys-park", None), ("fm-larchsys-relay", Some(RELAY_ROLE)), ("fm-larchsys-workspace", Some(WORKSPACE_ROLE))],
            "a wanted session of no folder stays in the block"
        );
        assert_eq!(names(&larchsys.repos[0]), ["fm-larchsys-gaia", "mine"], "the folder's supervised session first");
        assert_eq!(names(&gaia.repos[0]), ["fm-larchsys-gaia", "mine"]);
        assert_eq!(find(larchsys, "fm-larchsys-gaia").also_in, None, "larchsys wants it");
        assert_eq!(find(gaia, "fm-larchsys-gaia").also_in.as_deref(), Some("larchsys"));
        assert_eq!(find(gaia, "mine").also_in, None, "first shown in gaia");
        assert_eq!(find(larchsys, "mine").also_in.as_deref(), Some("gaia"));
        assert_eq!(tree.workspaces.iter().map(WorkspaceNode::session_count).collect::<Vec<_>>(), [2, 5]);
    }
}

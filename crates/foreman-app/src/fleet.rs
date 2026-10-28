//! The left panel's model: registered workspaces → their supervised sessions and repos → the sessions
//! in the enabled ones. Disabled repos come last, without sessions.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use foreman_claude::{Health, Kind, Session, assign_with, main_repo};
use foreman_core::{Event, Workspace};

#[derive(Debug, Clone)]
pub struct WorkspaceNode {
    pub name: String,
    pub root: PathBuf,
    /// `None`: never started (no desired-state file).
    pub started: Option<bool>,
    /// Rows named like one of the workspace's wanted sessions, wherever their cwd; stopped ones too.
    pub supervised: Vec<SessionRow>,
    pub repos: Vec<RepoNode>,
}

#[derive(Debug, Clone)]
pub struct RepoNode {
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    /// The registry's `enabled`; a disabled repo lists no sessions.
    pub enabled: bool,
    pub sessions: Vec<SessionRow>,
    /// Closed sessions past [`CLOSED_CAP`], behind its `more…` row.
    pub more: usize,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session: Session,
    pub health: Health,
    pub attachable: bool,
    pub hook: Option<HookSignal>,
    /// [`is_foreign`]: shown dimmed as another Foreman's, never highlighted as blocked.
    pub foreign: bool,
    pub starred: bool,
    /// From its transcript (m5/27): an interactive session `claude agents` no longer lists.
    pub closed: bool,
}

/// Closed sessions older than this are left out, unless starred.
pub const CLOSED_WINDOW_MS: i64 = 14 * 24 * 3_600_000;
/// Per repo, the newest closed sessions shown before its `more…` row.
pub const CLOSED_CAP: usize = 5;

/// The latest hook event of one session (`fm hook`, `<FOREMAN_HOME>/events`), keyed by sessionId.
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

    /// e.g. "permission prompt 12 s ago · compacted 2 min ago".
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

/// A permission prompt the hook reported after `claude agents` was polled marks a live row blocked
/// before the next poll can show it.
pub fn with_hook(health: Health, hook: Option<&HookSignal>, polled_at: i64) -> Health {
    match (health, hook) {
        (Health::Running | Health::Idle, Some(h)) if h.is_permission_prompt() && h.last.ts > polled_at => Health::Blocked {
            waiting_for: Some("permission prompt".into()),
        },
        (health, _) => health,
    }
}

/// What the supervisor knows per workspace (from its last tick), and the hook signals.
#[derive(Default)]
pub struct Supervised<'a> {
    pub desired: Option<&'a BTreeMap<String, (bool, Vec<String>)>>,
    pub hooks: Option<&'a HashMap<String, HookSignal>>,
    /// When the `claude agents` snapshot being shown was taken.
    pub polled_at: i64,
    /// [`main_repos`] of that snapshot; `None` walks the filesystem instead.
    pub main_repos: Option<&'a HashMap<PathBuf, Option<PathBuf>>>,
    /// Closed sessions from their transcripts ([`foreman_claude::Transcript::to_session`]); a session
    /// `claude agents` lists is shown from there instead.
    pub closed: &'a [Session],
    /// Starred session ids: shown whatever `show_stopped` and however old.
    pub starred: Option<&'a BTreeSet<String>>,
    /// `<workspace>/<repo>` whose `more…` was clicked: every closed session of the window shown.
    pub expanded: Option<&'a HashSet<String>>,
}

/// [`main_repo`] of every distinct session cwd: filled once per poll, off the UI thread, because
/// [`build`] runs on every frame (m4/2b: a workspace without repos walked every cwd every frame).
pub fn main_repos(sessions: &[Session]) -> HashMap<PathBuf, Option<PathBuf>> {
    let mut out = HashMap::new();
    for s in sessions {
        out.entry(s.cwd.clone()).or_insert_with(|| main_repo(&s.cwd));
    }
    out
}

pub struct Tree {
    pub workspaces: Vec<WorkspaceNode>,
    /// Stopped sessions left out because `show_stopped` was off.
    pub hidden_stopped: usize,
}

impl WorkspaceNode {
    pub fn session_count(&self) -> usize {
        self.supervised.len() + self.repos.iter().map(|r| r.sessions.len()).sum::<usize>()
    }
}

/// `claude attach` only takes over a live background session: an interactive one belongs to the
/// terminal that started it, and a stopped or orphaned one has no process to attach to.
pub fn attachable(session: &Session, health: &Health) -> bool {
    session.kind == Kind::Background
        && matches!(health, Health::Running | Health::Blocked { .. } | Health::Idle)
}

pub fn is_alive(health: &Health) -> bool {
    matches!(health, Health::Running | Health::Blocked { .. } | Health::Idle)
}

/// [`foreman_supervisor::is_foreign`]. Its prompts are not this user's to answer.
pub fn is_foreign(name: &str, workspaces: &[Workspace], desired: Option<&BTreeMap<String, (bool, Vec<String>)>>) -> bool {
    let wanted = desired.into_iter().flat_map(|d| d.values()).flat_map(|(_, wanted)| wanted.iter().map(String::as_str));
    foreman_supervisor::is_foreign(name, workspaces.iter().map(|w| w.name.as_str()), wanted)
}

/// Blocked first (it needs a human), then running, idle, unknown, orphaned, stopped; another Foreman's
/// blocked row ranks as running.
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
    let mut hidden_stopped = 0;
    let starred = |id: &str| sup.starred.is_some_and(|s| s.contains(id));
    let row = |session: Session| {
        let hook = sup.hooks.and_then(|h| h.get(&session.session_id)).cloned();
        let health = with_hook(session.supervised_health(now_ms), hook.as_ref(), sup.polled_at);
        SessionRow {
            attachable: attachable(&session, &health),
            foreign: is_foreign(&session.name, workspaces, sup.desired),
            starred: starred(&session.session_id),
            closed: false,
            session,
            health,
            hook,
        }
    };
    let listed: HashSet<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
    // a Foreman session's transcript (a selftest's, a supervised one removed) is not the user's to resume
    let closed: Vec<Session> = sup
        .closed
        .iter()
        .filter(|s| !listed.contains(s.session_id.as_str()))
        .filter(|s| starred(&s.session_id) || (!s.name.starts_with("fm-") && s.started_at.is_some_and(|t| now_ms - t <= CLOSED_WINDOW_MS)))
        .cloned()
        .collect();
    let main_of = |cwd: &Path| match sup.main_repos {
        Some(known) => known.get(cwd).cloned().flatten(),
        None => main_repo(cwd),
    };
    let workspaces = workspaces
        .iter()
        .map(|workspace| {
            let (started, wanted) = match sup.desired.and_then(|d| d.get(&workspace.name)) {
                Some((started, wanted)) => (Some(*started), wanted.as_slice()),
                None => (None, &[][..]),
            };
            let (theirs, rest): (Vec<Session>, Vec<Session>) =
                sessions.iter().cloned().partition(|s| wanted.contains(&s.name));
            let mut supervised: Vec<SessionRow> = theirs.into_iter().map(row).collect();
            sort(&mut supervised);
            let mut by_repo = assign_with(&rest, workspace, &main_of).by_repo;
            let mut closed_by_repo = assign_with(&closed, workspace, &main_of).by_repo;
            let mut repos: Vec<RepoNode> = workspace
                .repos
                .iter()
                .filter(|r| r.enabled)
                .map(|repo| {
                    let mut rows: Vec<SessionRow> = by_repo
                        .remove(&repo.name)
                        .unwrap_or_default()
                        .into_iter()
                        .map(row)
                        .filter(|row| {
                            let keep = show_stopped || row.health != Health::Stopped || row.starred;
                            hidden_stopped += usize::from(!keep);
                            keep
                        })
                        .collect();
                    sort(&mut rows);
                    let mut gone: Vec<SessionRow> = closed_by_repo
                        .remove(&repo.name)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|s| SessionRow { closed: true, ..row(s) })
                        .collect();
                    gone.sort_by(|a, b| b.session.started_at.cmp(&a.session.started_at));
                    let all = sup.expanded.is_some_and(|e| e.contains(&format!("{}/{}", workspace.name, repo.name)));
                    let (mut shown, mut more) = (0, 0);
                    for r in gone {
                        if r.starred {
                            rows.push(r);
                        } else if !show_stopped {
                            hidden_stopped += 1;
                        } else if all || shown < CLOSED_CAP {
                            shown += 1;
                            rows.push(r);
                        } else {
                            more += 1;
                        }
                    }
                    RepoNode {
                        name: repo.name.clone(),
                        path: workspace.root.join(&repo.path),
                        branch: repo.branch.clone(),
                        enabled: true,
                        sessions: rows,
                        more,
                    }
                })
                .collect();
            repos.extend(workspace.repos.iter().filter(|r| !r.enabled).map(|repo| RepoNode {
                name: repo.name.clone(),
                path: workspace.root.join(&repo.path),
                branch: repo.branch.clone(),
                enabled: false,
                sessions: Vec::new(),
                more: 0,
            }));
            WorkspaceNode {
                name: workspace.name.clone(),
                root: workspace.root.clone(),
                started,
                supervised,
                repos,
            }
        })
        .collect();
    Tree {
        workspaces,
        hidden_stopped,
    }
}

/// What the row's health label says; an orphaned row's process is gone, whatever it last claimed.
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

/// Where a new shell starts (m5/22): the folder of the workspace holding `active` (the active tab's
/// folder; the innermost root when roots nest), else the first workspace's, else `home`.
pub fn shell_cwd(active: Option<&Path>, workspaces: &[Workspace], home: Option<PathBuf>) -> Option<PathBuf> {
    // Registry roots may use `/` and another drive-letter case than a session's cwd.
    let key = |p: &Path| -> Vec<String> { p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect() };
    let holder = active.and_then(|cwd| {
        let cwd = key(cwd);
        workspaces
            .iter()
            .filter(|w| cwd.starts_with(&key(&w.root)))
            .max_by_key(|w| w.root.components().count())
    });
    holder.or(workspaces.first()).map(|w| w.root.clone()).or(home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreman_claude::parse;
    use foreman_core::Repo;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn shell_cwd_is_the_active_tabs_workspace_then_the_first_then_home() {
        let a = Workspace::new("a", "P:/Projects/A", vec![]);
        let b = Workspace::new("b", r"P:\Projects\B", vec![]);
        let inner = Workspace::new("inner", r"P:\Projects\B\inner", vec![]);
        let all = [a.clone(), b.clone(), inner.clone()];
        let home = Some(PathBuf::from(r"C:\Users\me"));
        let cwd = |active: Option<&str>, workspaces: &[Workspace]| shell_cwd(active.map(Path::new), workspaces, home.clone());
        assert_eq!(cwd(Some(r"p:\projects\b\repo\src"), &all), Some(b.root.clone()), "another drive-letter case");
        assert_eq!(cwd(Some(r"P:\Projects\A\x"), &all), Some(a.root.clone()), "forward-slash root");
        assert_eq!(cwd(Some(r"P:\Projects\B\inner\r"), &all), Some(inner.root.clone()), "innermost root");
        assert_eq!(cwd(Some(r"P:\Projects\Bee"), &all), Some(a.root.clone()), "a sibling with a common prefix is not inside");
        assert_eq!(cwd(None, &all), Some(a.root.clone()), "no active tab: the first workspace");
        assert_eq!(cwd(Some(r"D:\elsewhere"), &[]), home, "no workspace: home");
    }

    fn workspace() -> Workspace {
        let repo = |name: &str, enabled: bool| Repo {
            name: name.into(),
            path: format!(r"C:\demo\{name}").into(),
            has_claude_md: false,
            has_agents_md: false,
            branch: Some("main".into()),
            enabled,
        };
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
        assert!(!p.repos[2].enabled && p.repos[2].sessions.is_empty(), "and lists no sessions");
        assert_eq!(names(&p.repos[0]), ["parked", "busy"]);
        assert_eq!(names(&p.repos[1]), ["term", "gone"]);
        assert_eq!(tree.hidden_stopped, 1);
        assert_eq!(p.session_count(), 4);
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
                ("done", false)
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

    // m5/10
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

    // m5/27
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
}

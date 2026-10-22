//! The left panel's model: registered platforms → their enabled repos → the sessions in them.

use std::path::PathBuf;

use foreman_claude::{Health, Kind, Session, assign};
use foreman_core::Platform;

#[derive(Debug, Clone)]
pub struct PlatformNode {
    pub name: String,
    pub root: PathBuf,
    pub repos: Vec<RepoNode>,
}

#[derive(Debug, Clone)]
pub struct RepoNode {
    pub name: String,
    pub branch: Option<String>,
    pub sessions: Vec<SessionRow>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session: Session,
    pub health: Health,
    pub attachable: bool,
}

pub struct Tree {
    pub platforms: Vec<PlatformNode>,
    /// Stopped sessions left out because `show_stopped` was off.
    pub hidden_stopped: usize,
}

impl PlatformNode {
    pub fn session_count(&self) -> usize {
        self.repos.iter().map(|r| r.sessions.len()).sum()
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

/// Blocked first (it needs a human), then running, idle, unknown, orphaned, stopped.
fn rank(health: &Health) -> u8 {
    match health {
        Health::Blocked { .. } => 0,
        Health::Running => 1,
        Health::Idle => 2,
        Health::Unknown => 3,
        Health::Orphaned { .. } => 4,
        Health::Stopped => 5,
    }
}

pub fn build(platforms: &[Platform], sessions: &[Session], now_ms: i64, show_stopped: bool) -> Tree {
    let mut hidden_stopped = 0;
    let platforms = platforms
        .iter()
        .map(|platform| {
            let mut by_repo = assign(sessions, platform).by_repo;
            let repos = platform
                .repos
                .iter()
                .filter(|r| r.enabled)
                .map(|repo| {
                    let mut rows: Vec<SessionRow> = by_repo
                        .remove(&repo.name)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|session| {
                            let health = session.health(now_ms);
                            SessionRow {
                                attachable: attachable(&session, &health),
                                session,
                                health,
                            }
                        })
                        .filter(|row| {
                            let keep = show_stopped || row.health != Health::Stopped;
                            hidden_stopped += usize::from(!keep);
                            keep
                        })
                        .collect();
                    rows.sort_by(|a, b| {
                        rank(&a.health)
                            .cmp(&rank(&b.health))
                            .then_with(|| b.session.started_at.cmp(&a.session.started_at))
                    });
                    RepoNode {
                        name: repo.name.clone(),
                        branch: repo.branch.clone(),
                        sessions: rows,
                    }
                })
                .collect();
            PlatformNode {
                name: platform.name.clone(),
                root: platform.root.clone(),
                repos,
            }
        })
        .collect();
    Tree {
        platforms,
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

#[cfg(test)]
mod tests {
    use super::*;
    use foreman_claude::parse;
    use foreman_core::Repo;

    const NOW: i64 = 1_800_000_000_000;

    fn platform() -> Platform {
        let repo = |name: &str, enabled: bool| Repo {
            name: name.into(),
            path: format!(r"C:\demo\{name}").into(),
            has_claude_md: false,
            has_agents_md: false,
            branch: Some("main".into()),
            enabled,
        };
        Platform::new(
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
    fn groups_enabled_repos_blocked_first_and_hides_stopped() {
        let tree = build(&[platform()], &sessions(), NOW, false);
        let p = &tree.platforms[0];
        let repos: Vec<_> = p.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(repos, ["Alpha", "Beta"], "disabled repo Off is not listed");
        assert_eq!(names(&p.repos[0]), ["parked", "busy"]);
        assert_eq!(names(&p.repos[1]), ["term", "gone"]);
        assert_eq!(tree.hidden_stopped, 1);
        assert_eq!(p.session_count(), 4);
    }

    #[test]
    fn show_stopped_keeps_them_last() {
        let tree = build(&[platform()], &sessions(), NOW, true);
        assert_eq!(names(&tree.platforms[0].repos[1]), ["term", "gone", "done"]);
        assert_eq!(tree.hidden_stopped, 0);
    }

    #[test]
    fn attach_only_for_live_background_sessions() {
        let tree = build(&[platform()], &sessions(), NOW, true);
        let flags: Vec<(&str, bool)> = tree.platforms[0]
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
        let tree = build(&[platform()], &sessions(), NOW, true);
        let beta = &tree.platforms[0].repos[1];
        assert_eq!(health_label(&beta.sessions[1].health), "process gone");
        let alpha = &tree.platforms[0].repos[0];
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
    fn no_platforms_is_empty() {
        let tree = build(&[], &sessions(), NOW, false);
        assert!(tree.platforms.is_empty());
    }
}

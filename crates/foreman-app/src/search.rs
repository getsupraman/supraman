use std::ops::Range;

use foreman_ui::matcher::{find_all, prefix};

use crate::fleet::{SessionRow, Tree};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    pub name: Vec<Range<usize>>,
    pub id: Option<Range<usize>>,
}

#[derive(Debug, Clone)]
pub struct SessionHit {
    pub row: SessionRow,
    pub marks: Marks,
}

#[derive(Debug, Clone)]
pub struct RepoHit {
    pub name: String,
    pub path: std::path::PathBuf,
    pub name_marks: Vec<Range<usize>>,
    pub sessions: Vec<SessionHit>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceHit {
    pub name: String,
    pub home: Option<std::path::PathBuf>,
    pub name_marks: Vec<Range<usize>>,
    pub supervised: Vec<SessionHit>,
    pub repos: Vec<RepoHit>,
}

impl WorkspaceHit {
    pub fn count(&self) -> usize {
        self.supervised.len() + self.repos.iter().map(|r| r.sessions.len()).sum::<usize>()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Results {
    pub workspaces: Vec<WorkspaceHit>,
}

impl Results {
    pub fn sessions(&self) -> usize {
        self.workspaces.iter().map(WorkspaceHit::count).sum()
    }

    pub fn places(&self) -> usize {
        self.workspaces
            .iter()
            .map(|w| usize::from(!w.name_marks.is_empty()) + w.repos.iter().filter(|r| !r.name_marks.is_empty()).count())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }

    pub fn summary(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let (sessions, places) = (self.sessions(), self.places());
        let mut out = plural(sessions, "session", "sessions");
        if sessions > 0 {
            out.push_str(&format!(" in {}", plural(self.workspaces.iter().filter(|w| w.count() > 0).count(), "workspace", "workspaces")));
        }
        if places > 0 {
            out.push_str(&format!(", {} by name", plural(places, "workspace or repo", "workspaces or repos")));
        }
        out
    }
}

/// The id mark covers the short id as the row shows it.
pub fn session_marks(row: &SessionRow, needle: &str) -> Option<Marks> {
    let s = &row.session;
    let name = find_all(&s.name, needle);
    let short = crate::fleet::short_id(s);
    let id = (prefix(&s.id, needle) || prefix(&s.session_id, needle)).then(|| 0..needle.len().min(short.len()));
    (!name.is_empty() || id.is_some()).then_some(Marks { name, id })
}

pub fn search(tree: &Tree, query: &str) -> Results {
    let needle = query.trim();
    if needle.is_empty() {
        return Results::default();
    }
    let hits = |rows: &[SessionRow]| -> Vec<SessionHit> {
        rows.iter().filter_map(|row| session_marks(row, needle).map(|marks| SessionHit { row: row.clone(), marks })).collect()
    };
    let workspaces = tree
        .workspaces
        .iter()
        .filter_map(|w| {
            let repos: Vec<RepoHit> = w
                .repos
                .iter()
                .filter_map(|r| {
                    let repo = RepoHit { name: r.name.clone(), path: r.path.clone(), name_marks: find_all(&r.name, needle), sessions: hits(&r.sessions) };
                    (!repo.name_marks.is_empty() || !repo.sessions.is_empty()).then_some(repo)
                })
                .collect();
            let hit = WorkspaceHit { name: w.name.clone(), home: w.home.clone(), name_marks: find_all(&w.name, needle), supervised: hits(&w.supervised), repos };
            (!hit.name_marks.is_empty() || !hit.supervised.is_empty() || !hit.repos.is_empty()).then_some(hit)
        })
        .collect();
    Results { workspaces }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use foreman_claude::{Health, Kind, Session};

    use super::*;
    use crate::fleet::{RepoNode, WorkspaceNode};

    fn row(name: &str, session_id: &str, closed: bool) -> SessionRow {
        let session = Session {
            id: session_id[..8].to_string(),
            session_id: session_id.into(),
            name: name.into(),
            cwd: PathBuf::new(),
            kind: Kind::Interactive,
            state: None,
            status: None,
            waiting_for: None,
            pid: None,
            started_at: None,
        };
        SessionRow { closed, ..SessionRow::new(session, Health::Stopped) }
    }

    fn repo(name: &str, sessions: Vec<SessionRow>) -> RepoNode {
        RepoNode {
            name: name.into(),
            path: PathBuf::from(name),
            kind: foreman_core::FolderKind::Git,
            branch: None,
            enabled: true,
            listed: true,
            supervised_in: None,
            also_in: vec![],
            sessions,
            more: 0,
        }
    }

    fn tree() -> Tree {
        Tree {
            workspaces: vec![
                WorkspaceNode {
                    name: "gaia".into(),
                    home: Some(PathBuf::from("G:/gaia")),
                    started: Some(true),
                    supervised: vec![row("fm-gaia-root", "aaaa1111-0000", false)],
                    repos: vec![
                        repo("Web", vec![row("CI-CD-pipeline", "5e1f7e57-0001", true), row("docs", "01d05e1f-0002", false)]),
                        repo("api", vec![]),
                    ],
                },
                WorkspaceNode {
                    name: "nomos".into(),
                    home: Some(PathBuf::from("N:/nomos")),
                    started: None,
                    supervised: vec![],
                    repos: vec![repo("pipeline-tools", vec![row("Pipeline fix", "bbbb2222-0003", false)])],
                },
            ],
            hidden_stopped: 0,
        }
    }

    fn names(r: &Results) -> Vec<String> {
        let mut out = Vec::new();
        for w in &r.workspaces {
            out.push(format!("{} ({})", w.name, w.count()));
            out.extend(w.supervised.iter().map(|s| format!("  {}", s.row.session.name)));
            for repo in &w.repos {
                out.push(format!("  ./{} ({})", repo.name, repo.sessions.len()));
                out.extend(repo.sessions.iter().map(|s| format!("    {}", s.row.session.name)));
            }
        }
        out
    }

    #[test]
    fn sessions_match_by_name_anywhere_and_by_id_from_the_start() {
        let r = row("CI-CD-pipeline", "5e1f7e57-0001", true);
        assert_eq!(session_marks(&r, "PIPE"), Some(Marks { name: vec![6..10], id: None }));
        assert_eq!(session_marks(&r, "5E1F"), Some(Marks { name: vec![], id: Some(0..4) }));
        assert_eq!(session_marks(&r, "5e1f7e57-00"), Some(Marks { name: vec![], id: Some(0..8) }), "the full session id, past the short one");
        assert_eq!(session_marks(&r, "7e57"), None, "an id matches from its start only");
    }

    #[test]
    fn results_are_grouped_by_workspace_then_repo_with_counts() {
        let r = search(&tree(), "pipe");
        assert_eq!(names(&r), ["gaia (1)", "  ./Web (1)", "    CI-CD-pipeline", "nomos (1)", "  ./pipeline-tools (1)", "    Pipeline fix"]);
        assert_eq!(r.workspaces[1].repos[0].name_marks, [0..4]);
        assert_eq!(r.summary(), "2 sessions in 2 workspaces, 1 workspace or repo by name");
    }

    #[test]
    fn a_closed_session_is_found_by_its_id_prefix_and_a_workspace_by_its_name() {
        let r = search(&tree(), " 5e1f ");
        assert_eq!(names(&r), ["gaia (1)", "  ./Web (1)", "    CI-CD-pipeline"], "01d05e1f has 5e1f inside, not at its start");
        assert!(r.workspaces[0].repos[0].sessions[0].row.closed);
        let r = search(&tree(), "GAIA");
        assert_eq!(names(&r), ["gaia (1)", "  fm-gaia-root"]);
        assert_eq!(r.workspaces[0].name_marks, [0..4]);
        assert_eq!(r.summary(), "1 session in 1 workspace, 1 workspace or repo by name");
    }

    #[test]
    fn a_repo_found_by_name_shows_without_sessions_and_nothing_found_is_empty() {
        let r = search(&tree(), "api");
        assert_eq!(names(&r), ["gaia (0)", "  ./api (0)"]);
        assert_eq!(r.summary(), "0 sessions, 1 workspace or repo by name");
        assert!(search(&tree(), "zzqx").is_empty());
        assert!(search(&tree(), "   ").is_empty());
    }
}

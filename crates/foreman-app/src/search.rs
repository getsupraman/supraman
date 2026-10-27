//! The tree's search (m5/30), as VS Code's Search panel shows results: sessions found by name or by a
//! prefix of their id, workspaces and repos by name, grouped workspace › repo with the count of sessions
//! found under each. Matching ignores case. The tree searched is the whole one: stopped, closed and
//! starred sessions, and the closed ones behind `more…` ([`crate::view::FleetView::search_tree`]).

use std::ops::Range;

use crate::fleet::{SessionRow, Tree};

/// Where a session row matched: byte ranges in its name, and the matched prefix of its short id.
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
    pub root: std::path::PathBuf,
    pub name_marks: Vec<Range<usize>>,
    /// Its supervised sessions that matched (shown under the workspace, as in the tree).
    pub supervised: Vec<SessionHit>,
    pub repos: Vec<RepoHit>,
}

impl WorkspaceHit {
    /// The sessions found under it, its badge.
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

    /// Workspaces and repos found by their own name.
    pub fn places(&self) -> usize {
        self.workspaces
            .iter()
            .map(|w| usize::from(!w.name_marks.is_empty()) + w.repos.iter().filter(|r| !r.name_marks.is_empty()).count())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }

    /// VS Code's summary line: `3 sessions in 2 workspaces`, plus the places found by name.
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

fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Every place `needle` occurs in `hay`, ignoring case, as byte ranges of `hay`, not overlapping.
pub fn find_all(hay: &str, needle: &str) -> Vec<Range<usize>> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = hay.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()].iter().zip(&needle).all(|((_, h), n)| same(*h, *n)) {
            let end = chars.get(i + needle.len()).map_or(hay.len(), |(at, _)| *at);
            out.push(chars[i].0..end);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Whether `id` starts with `needle`, ignoring case.
fn id_prefix(id: &str, needle: &str) -> bool {
    !needle.is_empty() && id.len() >= needle.len() && id.is_char_boundary(needle.len()) && id[..needle.len()].eq_ignore_ascii_case(needle)
}

/// Where `row` matches `needle`, if it does: its name anywhere, or its id (the 8-hex short id or the full
/// session id) from the start. The id mark covers the short id as the row shows it.
pub fn session_marks(row: &SessionRow, needle: &str) -> Option<Marks> {
    let s = &row.session;
    let name = find_all(&s.name, needle);
    let short = crate::fleet::short_id(s);
    let id = (id_prefix(&s.id, needle) || id_prefix(&s.session_id, needle)).then(|| 0..needle.len().min(short.len()));
    (!name.is_empty() || id.is_some()).then_some(Marks { name, id })
}

/// `tree` filtered by `query` (trimmed): a workspace stays when its name, one of its repos' names or one
/// of its sessions matches; a repo when its name or one of its sessions does. Order is the tree's.
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
            let hit = WorkspaceHit { name: w.name.clone(), root: w.root.clone(), name_marks: find_all(&w.name, needle), supervised: hits(&w.supervised), repos };
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
        SessionRow {
            session: Session {
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
            },
            health: Health::Stopped,
            attachable: false,
            hook: None,
            foreign: false,
            starred: false,
            closed,
        }
    }

    fn repo(name: &str, sessions: Vec<SessionRow>) -> RepoNode {
        RepoNode { name: name.into(), path: PathBuf::from(name), branch: None, enabled: true, sessions, more: 0 }
    }

    fn tree() -> Tree {
        Tree {
            workspaces: vec![
                WorkspaceNode {
                    name: "gaia".into(),
                    root: PathBuf::from("G:/gaia"),
                    started: Some(true),
                    supervised: vec![row("fm-gaia-root", "aaaa1111-0000", false)],
                    repos: vec![
                        repo("Web", vec![row("CI-CD-pipeline", "5e1f7e57-0001", true), row("docs", "01d05e1f-0002", false)]),
                        repo("api", vec![]),
                    ],
                },
                WorkspaceNode {
                    name: "nomos".into(),
                    root: PathBuf::from("N:/nomos"),
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
    fn find_all_ignores_case_and_returns_byte_ranges() {
        assert_eq!(find_all("CI-CD-Pipeline", "pipe"), [6..10]);
        assert_eq!(find_all("aAa", "a"), [0..1, 1..2, 2..3]);
        assert_eq!(find_all("Grüße GRÜSSE", "grü"), [0..4, 8..12]);
        assert_eq!(find_all("abc", ""), Vec::<Range<usize>>::new());
        assert_eq!(find_all("ab", "abc"), Vec::<Range<usize>>::new());
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

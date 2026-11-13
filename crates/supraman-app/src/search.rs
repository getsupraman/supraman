use std::ops::Range;

use supraman_ui::matcher::{find_all, prefix};

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
pub struct FolderHit {
    /// The folder's [`crate::fleet::FolderNode::key`].
    pub key: String,
    pub name: String,
    pub kind: supraman_core::FolderKind,
    pub name_marks: Vec<Range<usize>>,
    pub sessions: Vec<SessionHit>,
}

/// A group with its hits, or a top-level folder alone (`group` false, its one folder in `folders`).
#[derive(Debug, Clone)]
pub struct EntryHit {
    pub name: String,
    pub group: bool,
    pub name_marks: Vec<Range<usize>>,
    /// Scheduled runs' sessions, under the group rather than a folder.
    pub runs: Vec<SessionHit>,
    pub folders: Vec<FolderHit>,
}

impl EntryHit {
    pub fn count(&self) -> usize {
        self.runs.len() + self.folders.iter().map(|r| r.sessions.len()).sum::<usize>()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Results {
    pub entries: Vec<EntryHit>,
}

impl Results {
    pub fn sessions(&self) -> usize {
        self.entries.iter().map(EntryHit::count).sum()
    }

    pub fn places(&self) -> usize {
        self.entries.iter().map(|w| usize::from(!w.name_marks.is_empty()) + w.folders.iter().filter(|r| !r.name_marks.is_empty()).count()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn summary(&self) -> String {
        let (sessions, places) = (self.sessions(), self.places());
        let mut out = supraman_core::text::count(sessions, "session", "sessions");
        if places > 0 {
            out.push_str(&format!(", {} by name", supraman_core::text::count(places, "group or folder", "groups or folders")));
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
    let hits = |rows: &[SessionRow]| -> Vec<SessionHit> { rows.iter().filter_map(|row| session_marks(row, needle).map(|marks| SessionHit { row: row.clone(), marks })).collect() };
    let entries = tree
        .entries
        .iter()
        .filter_map(|w| {
            let folders: Vec<FolderHit> = w
                .folders
                .iter()
                .filter_map(|r| {
                    let folder = FolderHit { key: r.key.clone(), name: r.name.clone(), kind: r.kind, name_marks: find_all(&r.name, needle), sessions: hits(&r.sessions) };
                    (!folder.name_marks.is_empty() || !folder.sessions.is_empty()).then_some(folder)
                })
                .collect();
            let name_marks = if w.group { find_all(&w.name, needle) } else { Vec::new() };
            let runs = hits(&w.folders.iter().flat_map(|f| f.scheduled.values().flatten()).cloned().collect::<Vec<_>>());
            let hit = EntryHit { name: w.name.clone(), group: w.group, name_marks, runs, folders };
            (!hit.name_marks.is_empty() || !hit.runs.is_empty() || !hit.folders.is_empty()).then_some(hit)
        })
        .collect();
    Results { entries }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use supraman_claude::{Health, Kind, Session};

    use super::*;
    use crate::fleet::{EntryNode, FolderNode};

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

    fn folder(key: &str, sessions: Vec<SessionRow>) -> FolderNode {
        let name = key.rsplit('/').next().unwrap().to_string();
        FolderNode { key: key.into(), path: PathBuf::from(&name), name, kind: supraman_core::FolderKind::Git, branch: None, sessions, more: 0, scheduled: Default::default() }
    }

    fn tree() -> Tree {
        Tree {
            entries: vec![
                EntryNode {
                    name: "gaia".into(),
                    group: true,
                    folders: vec![
                        FolderNode {
                            scheduled: [("nightly".to_string(), vec![row("fm-gaia-1a2b3c4d-sched-nightly-20261113-0900", "aaaa1111-0000", false)])].into(),
                            ..folder("gaia/Web", vec![row("CI-CD-pipeline", "5e1f7e57-0001", true), row("docs", "01d05e1f-0002", false)])
                        },
                        folder("gaia/api", vec![]),
                    ],
                },
                EntryNode { name: "pipeline-tools".into(), group: false, folders: vec![folder("pipeline-tools", vec![row("Pipeline fix", "bbbb2222-0003", false)])] },
            ],
            hidden_stopped: 0,
        }
    }

    fn names(r: &Results) -> Vec<String> {
        let mut out = Vec::new();
        for w in &r.entries {
            if w.group {
                out.push(format!("{} ({})", w.name, w.count()));
            }
            out.extend(w.runs.iter().map(|s| format!("  {}", s.row.session.name)));
            for f in &w.folders {
                out.push(format!("  {} ({})", f.key, f.sessions.len()));
                out.extend(f.sessions.iter().map(|s| format!("    {}", s.row.session.name)));
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
    fn results_are_grouped_by_entry_then_folder_with_counts() {
        let r = search(&tree(), "pipe");
        assert_eq!(names(&r), ["gaia (1)", "  gaia/Web (1)", "    CI-CD-pipeline", "  pipeline-tools (1)", "    Pipeline fix"]);
        assert_eq!(r.entries[1].folders[0].name_marks, [0..4]);
        assert_eq!(r.summary(), "2 sessions, 1 group or folder by name");
    }

    #[test]
    fn a_closed_session_is_found_by_its_id_prefix_and_a_group_by_its_name() {
        let r = search(&tree(), " 5e1f ");
        assert_eq!(names(&r), ["gaia (1)", "  gaia/Web (1)", "    CI-CD-pipeline"], "01d05e1f has 5e1f inside, not at its start");
        assert!(r.entries[0].folders[0].sessions[0].row.closed);
        let r = search(&tree(), "GAIA");
        assert_eq!(names(&r), ["gaia (1)", "  fm-gaia-1a2b3c4d-sched-nightly-20261113-0900"], "a scheduled run under the group");
        assert_eq!(r.entries[0].name_marks, [0..4]);
        assert_eq!(r.summary(), "1 session, 1 group or folder by name");
    }

    #[test]
    fn a_folder_found_by_name_shows_without_sessions_and_nothing_found_is_empty() {
        let r = search(&tree(), "api");
        assert_eq!(names(&r), ["gaia (0)", "  gaia/api (0)"]);
        assert_eq!(r.summary(), "0 sessions, 1 group or folder by name");
        assert!(search(&tree(), "zzqx").is_empty());
        assert!(search(&tree(), "   ").is_empty());
    }
}

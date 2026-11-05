//! Everything the Git tab reads, blocking, for `cx.background_spawn`: git located, the refs, the status and the
//! history page by page, each page laid out with the state the last one left.

use std::path::Path;

use gpui::SharedString;
use supraman_core::Settings;
use supraman_gitman::{ChangeKind, Commit, EntryKind, FIRST_PAGE, Layout, Located, NEXT_PAGE, Repo, Row, Status};

/// One row of the graph, its texts ready to paint.
#[derive(Debug, Clone)]
pub struct GraphRow {
    pub row: Row,
    /// Empty on the WIP row.
    pub sha: SharedString,
    pub summary: SharedString,
    /// The body's start, on one line.
    pub body: SharedString,
    pub initials: SharedString,
    /// Epoch milliseconds of the commit; 0 on the WIP row.
    pub time_ms: i64,
    pub merge: bool,
}

impl GraphRow {
    pub fn wip(&self) -> bool {
        self.row.sha.is_none()
    }
}

/// The work in progress the WIP row counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Wip {
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
}

impl Wip {
    /// Each changed path once: added if either side adds it, else deleted if either deletes it, else modified.
    pub fn of(status: &Status) -> Wip {
        let mut wip = Wip::default();
        for e in &status.entries {
            let sides = [e.staged, e.unstaged];
            if e.kind == EntryKind::Untracked || sides.contains(&Some(ChangeKind::Added)) {
                wip.added += 1;
            } else if sides.contains(&Some(ChangeKind::Deleted)) {
                wip.deleted += 1;
            } else {
                wip.modified += 1;
            }
        }
        wip
    }

    /// `+1 ✎2 −1`, the zero counts left out.
    pub fn counts(&self) -> String {
        [(self.added, "+"), (self.modified, "✎"), (self.deleted, "−")]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, mark)| format!("{mark}{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Why the tab shows no graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// No git, or one too old or unusable: the words of the page.
    Git(String),
    Failed(String),
}

/// A page laid out: its rows, whether more follow, and the layout to lay out the next one with.
pub struct Page {
    pub rows: Vec<GraphRow>,
    pub more: bool,
    pub layout: Layout,
}

pub struct First {
    pub repo: Repo,
    pub tips: Vec<String>,
    pub wip: Option<Wip>,
    pub page: Page,
}

pub fn git_problem(located: &Located) -> Option<String> {
    match located {
        Located::Found(_) => None,
        Located::Missing { at: Some(at) } => Some(format!("No git at {}, the path set in [gitman] git.", at.display())),
        Located::Missing { at: None } => Some("No git on PATH. Install Git for Windows, or set [gitman] git to a git.exe.".into()),
        Located::TooOld { path, version, floor } => Some(format!("{} is git {version}; the Git tab needs {floor} or newer.", path.display())),
        Located::Unusable { path, reason } => Some(format!("{} does not run as git: {reason}", path.display())),
    }
}

/// git located per the settings in `home`, the refs, the status and the first page.
pub fn first(home: Option<&Path>, folder: &Path) -> Result<First, Problem> {
    let settings = home.and_then(|h| Settings::load(h).ok()).unwrap_or_default();
    let located = supraman_gitman::locate(&settings);
    if let Some(problem) = git_problem(&located) {
        return Err(Problem::Git(problem));
    }
    let Located::Found(git) = located else { unreachable!() };
    let repo = Repo::new(git, folder);
    let failed = |e: supraman_gitman::GitError| Problem::Failed(e.to_string());
    let refs = supraman_gitman::refs(&repo).map_err(failed)?;
    let status = supraman_gitman::status(&repo).map_err(failed)?;
    let dirty = !status.entries.is_empty() || status.operation.is_some();
    let layout = Layout::new(&refs, dirty);
    let tips = refs.tips();
    let page = page(&repo, &tips, 0, FIRST_PAGE, layout).map_err(Problem::Failed)?;
    let wip = page.rows.first().filter(|r| r.wip()).map(|_| Wip::of(&status));
    Ok(First { repo, tips, wip, page })
}

/// The page of `count` commits after the first `skip`, laid out after the rows `layout` has seen.
pub fn page(repo: &Repo, tips: &[String], skip: usize, count: usize, mut layout: Layout) -> Result<Page, String> {
    let history = supraman_gitman::history(repo, tips, skip, count).map_err(|e| e.to_string())?;
    let rows = layout.extend(&history.commits);
    let mut commits = history.commits.iter();
    let rows = rows.into_iter().map(|row| if row.sha.is_some() { graph_row(row, commits.next()) } else { graph_row(row, None) }).collect();
    Ok(Page { rows, more: history.more, layout })
}

pub fn next(repo: &Repo, tips: &[String], skip: usize, layout: Layout) -> Result<Page, String> {
    page(repo, tips, skip, NEXT_PAGE, layout)
}

fn graph_row(row: Row, commit: Option<&Commit>) -> GraphRow {
    let Some(c) = commit else {
        return GraphRow { row, sha: "".into(), summary: "".into(), body: "".into(), initials: "".into(), time_ms: 0, merge: false };
    };
    let (summary, body) = c.message.split_once('\n').unwrap_or((&c.message, ""));
    GraphRow {
        sha: c.sha.clone().into(),
        summary: summary.trim_end().to_string().into(),
        body: body_start(body).into(),
        initials: initials(&c.author.name).into(),
        time_ms: c.author.time * 1000,
        merge: c.parents.len() > 1,
        row,
    }
}

/// The body's first words on one line, enough for the widest message column.
fn body_start(body: &str) -> String {
    body.split_whitespace().scan(0, |len, word| {
        *len += word.len() + 1;
        (*len <= 240).then_some(word)
    })
    .collect::<Vec<_>>()
    .join(" ")
}

/// `AF` for `Ada Fixture`, `JV` for `Jan van Vliet`, `A` for `ada`; `?` for no name.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-').filter(|w| !w.is_empty()).collect();
    let first = |w: &str| w.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    match words.as_slice() {
        [] => "?".into(),
        [one] => first(one),
        [a, .., b] => first(a) + &first(b),
    }
}

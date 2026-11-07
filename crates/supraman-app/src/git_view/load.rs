//! Everything the Git tab reads, blocking, for `cx.background_spawn`: git located, the refs, the status and the
//! history page by page, each page laid out with the state the last one left.

use std::path::Path;
use std::sync::Arc;

use gpui::SharedString;
use supraman_core::Settings;
use super::ref_panel::{self, Walk};
use super::splice::{self, Loaded};
use supraman_gitman::{ChangeKind, Commit, EntryKind, FIRST_PAGE, Head, History, Layout, Located, NEXT_PAGE, Refs, Repo, Row, Stash, Status};

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
    pub author: SharedString,
    /// Epoch milliseconds of the commit; 0 on the WIP row.
    pub time_ms: i64,
    pub merge: bool,
    /// Empty on the WIP row.
    pub parents: Vec<SharedString>,
}

impl GraphRow {
    pub fn wip(&self) -> bool {
        self.row.sha.is_none()
    }

    pub fn stash(&self) -> Option<&Arc<Stash>> {
        self.row.stash.as_ref()
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

    /// The changed paths.
    pub fn total(&self) -> usize {
        self.added + self.modified + self.deleted
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

/// The commits HEAD and its branch's upstream are on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Heads {
    pub head: Option<SharedString>,
    pub upstream: Option<SharedString>,
}

impl Heads {
    pub fn of(refs: &Refs) -> Heads {
        let (head, branch) = match &refs.head {
            Head::Branch { name, sha } => (sha.clone(), Some(name)),
            Head::Detached { sha } => (Some(sha.clone()), None),
        };
        let upstream = branch
            .and_then(|name| refs.branches.iter().find(|b| &b.name == name))
            .and_then(|b| b.upstream.as_ref().filter(|u| !u.gone))
            .and_then(|u| refs.remotes.iter().find(|r| r.name == u.name))
            .map(|r| SharedString::from(r.sha.clone()));
        Heads { head: head.map(SharedString::from), upstream }
    }
}

pub struct First {
    pub repo: Repo,
    /// Every ref, the hidden ones too, as the ref panel's rows.
    pub refs: ref_panel::Built,
    pub tips: Vec<String>,
    pub heads: Heads,
    pub wip: Option<Wip>,
    pub status: Arc<Status>,
    pub page: Page,
    /// The walk read, its hidden and soloed refs.
    pub walk: Walk,
    pub kept: Option<Kept>,
    /// The walk's `rev-list` read on below the first page, unless rows were kept below it.
    pub history: Option<History>,
}

/// The commits loaded below a refresh's first page, laid out again after it.
pub struct Kept {
    /// The first one's index among the commits loaded.
    pub from: usize,
    pub rows: Vec<Row>,
    /// The layout after the last of them, and whether more follow it.
    pub layout: Layout,
    pub more: bool,
}

pub fn git_problem(located: &Located) -> Option<String> {
    match located {
        Located::Found(_) => None,
        Located::Missing { at: Some(at) } => Some(format!("No git at {}, the path set in [gitman] git.", at.display())),
        Located::Missing { at: None } => Some("No git on PATH. Install Git for Windows, or set [gitman] git to a git.exe.".into()),
        Located::TooOld { path, version, floor } => Some(format!("{} is git {version}; the Git tab needs {floor} or newer.", path.display())),
        Located::Unusable { path, reason } => Some(format!("{} does not run as git: {reason}", path.display())),
        Located::BadSetting(notice) => Some(notice.clone()),
    }
}

/// git located per the settings in `home`, the repo's dirs, the refs, the status and the first page of `walk`.
pub fn first(home: Option<&Path>, folder: &Path, walk: &Walk) -> Result<First, Problem> {
    let settings = match home.map(Settings::load_with_notices).transpose() {
        Ok(loaded) => loaded.map(|(settings, _)| settings).unwrap_or_default(),
        Err(e) => return Err(Problem::Git(format!("[gitman] git is not read: {e}"))),
    };
    let located = supraman_gitman::located(&settings);
    if let Some(problem) = git_problem(&located) {
        return Err(Problem::Git(problem));
    }
    let Located::Found(git) = located else { unreachable!() };
    let repo = Repo::new(git, folder);
    let dirs = supraman_gitman::dirs(&repo).map_err(|e| Problem::Failed(e.to_string()))?;
    read(repo.with_dirs(dirs), walk, None).map_err(Problem::Failed)
}

/// The refs, the status and the first page of `repo` walked as `walk` says, laid out afresh, and the commits
/// `loaded` below it laid out again after it when the walk below is the same.
pub fn read(repo: Repo, walk: &Walk, loaded: Option<&Loaded>) -> Result<First, String> {
    let failed = |e: supraman_gitman::GitError| e.to_string();
    let _index = supraman_gitman::write::refresh_lock(&repo);
    let refs = supraman_gitman::refs(&repo).map_err(failed)?;
    let status = supraman_gitman::status_first(&repo).map_err(failed)?;
    let walked = ref_panel::without(&refs, walk);
    let layout = Layout::new(&walked, dirty(&status));
    let tips = ref_panel::tips(&walked, walk);
    let mut walker = History::start(&repo, &tips, 0).map_err(failed)?;
    let history = walker.page(FIRST_PAGE).map_err(failed)?;
    let kept_from = loaded.filter(|_| history.more).and_then(|l| Some((l, splice::kept_from(l, &tips, &history.commits)?)));
    let page = lay_out(history, layout);
    let kept = kept_from.map(|(loaded, from)| {
        let mut layout = page.layout.clone();
        let rows = layout.extend_with(loaded.commits[from..].iter().map(|(sha, parents)| (sha.as_ref(), parents.as_slice())));
        Kept { from, rows, layout, more: loaded.more }
    });
    let wip = page.rows.first().filter(|r| r.wip()).map(|_| Wip::of(&status));
    let history = (kept.is_none() && page.more).then_some(walker);
    Ok(First { repo, heads: Heads::of(&walked), refs: ref_panel::build(&refs), tips, wip, status: Arc::new(status), page, walk: walk.clone(), kept, history })
}

fn dirty(status: &Status) -> bool {
    !status.entries.is_empty() || status.operation.is_some()
}

/// The WIP row's counts, `None` for a clean tree, and the status they count.
pub fn wip(repo: &Repo) -> Result<(Option<Wip>, Arc<Status>), String> {
    let _index = supraman_gitman::write::refresh_lock(repo);
    let status = supraman_gitman::status(repo).map_err(|e| e.to_string())?;
    Ok((dirty(&status).then(|| Wip::of(&status)), Arc::new(status)))
}

fn lay_out(history: supraman_gitman::Page, mut layout: Layout) -> Page {
    let rows = layout.extend(&history.commits);
    let mut commits = history.commits.iter();
    let rows = rows.into_iter().map(|row| if row.sha.is_some() { graph_row(row, commits.next()) } else { graph_row(row, None) }).collect();
    Page { rows, more: history.more, layout }
}

/// The page after the first `skip` commits, read on from `walker` and laid out after the rows `layout` has seen;
/// the walker to read the next one from while more follow.
pub fn next(repo: &Repo, tips: &[String], skip: usize, layout: Layout, walker: Option<History>) -> Result<(Page, Option<History>), String> {
    let failed = |e: supraman_gitman::GitError| e.to_string();
    // Rows the walker already passed (a dropped tail, rows a refresh kept) cost a new walk from `skip`.
    let mut walker = match walker.filter(|w| w.at() == skip && w.tips() == tips) {
        Some(walker) => walker,
        None => History::start(repo, tips, skip).map_err(failed)?,
    };
    let page = lay_out(walker.page(NEXT_PAGE).map_err(failed)?, layout);
    let walker = page.more.then_some(walker);
    Ok((page, walker))
}

fn graph_row(row: Row, commit: Option<&Commit>) -> GraphRow {
    let Some(c) = commit else {
        return GraphRow { row, sha: "".into(), summary: "".into(), body: "".into(), initials: "".into(), author: "".into(), time_ms: 0, merge: false, parents: Vec::new() };
    };
    let (summary, body) = c.message.split_once('\n').unwrap_or((&c.message, ""));
    let (summary, body, time) = match &row.stash {
        Some(stash) => (stash.label(), String::new(), stash.time),
        None => (summary.trim_end().to_string(), body_start(body), c.author.time),
    };
    GraphRow {
        sha: c.sha.clone().into(),
        summary: summary.into(),
        body: body.into(),
        initials: initials(&c.author.name).into(),
        author: c.author.name.clone().into(),
        time_ms: time * 1000,
        merge: c.parents.len() > 1,
        parents: c.parents.iter().map(|p| SharedString::from(p.clone())).collect(),
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

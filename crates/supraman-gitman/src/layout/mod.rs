//! The commit graph's rows: lanes, edges, ref groups and ghost refs, laid out page by page off the UI thread.

mod edges;
mod groups;
mod lanes;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use groups::{RefGroup, RefKind, RefLabel};
pub use lanes::{Edge, EdgeKind, LANE_COLOURS};

use crate::{Commit, Refs, Stash};
use edges::Edges;
use groups::RefIndex;
use lanes::Lanes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `None` on the WIP row and a worktree's.
    pub sha: Option<String>,
    pub lane: usize,
    pub colour: usize,
    edges: Edges,
    /// The highest lane any edge or the node uses in this row.
    pub max_lane: usize,
    /// In order; the row shows the first and `+(len - 1)`.
    pub refs: Arc<[RefGroup]>,
    /// The nearest descendant's branch groups, for a row with no refs of its own.
    pub ghost: Option<Arc<[RefGroup]>>,
    pub detached_head: bool,
    /// The stash whose index commit the row is.
    pub stash: Option<Arc<Stash>>,
    /// The other worktree whose work in progress the row is.
    pub worktree: Option<Arc<Path>>,
}

impl Row {
    /// The edges the row paints: ending and passing by lane, then starting.
    pub fn edges(&self) -> impl Iterator<Item = Edge> + '_ {
        self.edges.iter()
    }

    /// The `+N` beside the first group.
    pub fn more(&self) -> usize {
        self.refs.len().saturating_sub(1)
    }

    pub fn worktree(&self) -> Option<&Path> {
        self.worktree.as_deref()
    }

    /// `n` rows of no commit stacked directly above this one on its lane, each joined to the next by a dashed edge,
    /// and this row joined to the lowest: the live sessions' rows over the WIP row.
    pub fn stacked_above(&self, n: usize) -> (Vec<Row>, Row) {
        let (above, edges) = self.edges.stacked_above(n);
        let none: Arc<[RefGroup]> = Arc::from([]);
        let rows = above
            .into_iter()
            .map(|edges| Row { sha: None, lane: self.lane, colour: self.colour, edges, max_lane: self.max_lane, refs: none.clone(), ghost: None, detached_head: false, stash: None, worktree: None })
            .collect();
        (rows, Row { edges, ..self.clone() })
    }
}

/// Another worktree's WIP row, laid out directly above its HEAD commit's row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorktreeRow {
    pub head: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
struct Handed {
    /// The child's parent index, then its row.
    rank: (usize, usize),
    groups: Arc<[RefGroup]>,
}

/// The state carried from one page to the next; clone it to lay out a page off the UI thread.
#[derive(Debug, Clone)]
pub struct Layout {
    lanes: Lanes,
    refs: Arc<RefIndex>,
    handed: HashMap<String, Handed>,
    /// Each stash by its index commit, the newest of those sharing one.
    stashes: Arc<HashMap<String, Arc<Stash>>>,
    /// The worktrees' WIP rows by their HEAD commit.
    worktrees: Arc<HashMap<String, Vec<Arc<Path>>>>,
    wip: bool,
    rows: usize,
    none: Arc<[RefGroup]>,
}

impl Layout {
    /// With `wip`, the first page starts with a WIP row parented to HEAD's commit, when HEAD has one.
    pub fn new(refs: &Refs, wip: bool) -> Layout {
        let mut stashes = HashMap::new();
        for stash in refs.stashes.iter().rev() {
            stashes.insert(stash.index.clone(), Arc::new(stash.clone()));
        }
        let refs = RefIndex::new(refs);
        let wip = wip && refs.head.is_some();
        Layout { lanes: Lanes::default(), refs: Arc::new(refs), handed: HashMap::new(), stashes: Arc::new(stashes), worktrees: Arc::default(), wip, rows: 0, none: Arc::from([]) }
    }

    /// With a WIP row for each of `worktrees`, each above its HEAD's row, on its own lane, its edge dashed.
    pub fn with_worktrees(mut self, worktrees: &[WorktreeRow]) -> Layout {
        let mut by_head: HashMap<String, Vec<Arc<Path>>> = HashMap::new();
        for w in worktrees {
            by_head.entry(w.head.clone()).or_default().push(Arc::from(w.path.as_path()));
        }
        self.worktrees = Arc::new(by_head);
        self
    }

    /// The rows of the next page; pages laid out in turn give the rows of laying them out at once.
    pub fn extend(&mut self, commits: &[Commit]) -> Vec<Row> {
        self.extend_with(commits.iter().map(|c| (c.sha.as_str(), c.parents.as_slice())))
    }

    /// [`Layout::extend`] over commits as their shas and parents, such as rows read before.
    pub fn extend_with<'a, P: AsRef<str> + 'a>(&mut self, commits: impl ExactSizeIterator<Item = (&'a str, &'a [P])>) -> Vec<Row> {
        let mut rows = Vec::with_capacity(commits.len() + 1);
        if std::mem::take(&mut self.wip) {
            let head = self.refs.head.clone().into_iter().collect::<Vec<_>>();
            let placed = self.lanes.place("", &head, true, false);
            rows.push(self.row(None, placed, None));
        }
        for (sha, parents) in commits {
            for path in self.worktrees.get(sha).cloned().unwrap_or_default() {
                let placed = self.lanes.place("", &[sha], true, true);
                let mut row = self.row(None, placed, None);
                row.worktree = Some(path);
                rows.push(row);
            }
            let stash = self.stashes.get(sha).cloned();
            let placed = self.lanes.place(sha, parents, stash.is_some(), stash.is_some());
            let ghost = if stash.is_some() { None } else { self.ghost(sha, parents) };
            let mut row = self.row(Some(sha.to_string()), placed, ghost);
            row.stash = stash;
            rows.push(row);
        }
        rows
    }

    fn row(&mut self, sha: Option<String>, placed: lanes::Placed, ghost: Option<Arc<[RefGroup]>>) -> Row {
        self.rows += 1;
        let refs = sha.as_ref().and_then(|s| self.refs.groups.get(s)).cloned().unwrap_or_else(|| self.none.clone());
        let detached_head = sha.is_some() && sha == self.refs.detached;
        Row { sha, lane: placed.lane, colour: placed.lane % LANE_COLOURS, edges: placed.edges, max_lane: placed.max_lane, refs, ghost, detached_head, stash: None, worktree: None }
    }

    /// A commit with branch refs hands them to its parents; one with no refs takes the best it was handed and
    /// hands that on.
    fn ghost<P: AsRef<str>>(&mut self, sha: &str, parents: &[P]) -> Option<Arc<[RefGroup]>> {
        let handed = self.handed.remove(sha).map(|h| h.groups);
        let (ghost, pass) = match self.refs.branches.get(sha) {
            Some(own) => (None, Some(own.clone())),
            None if self.refs.groups.contains_key(sha) => (None, None),
            None => (handed.clone(), handed),
        };
        if let Some(groups) = pass {
            for (i, parent) in parents.iter().enumerate() {
                self.hand(parent.as_ref(), (i, self.rows), &groups);
            }
        }
        ghost
    }

    fn hand(&mut self, parent: &str, rank: (usize, usize), groups: &Arc<[RefGroup]>) {
        let better = self.handed.get(parent).is_none_or(|h| rank < h.rank);
        if better {
            self.handed.insert(parent.to_string(), Handed { rank, groups: groups.clone() });
        }
    }
}

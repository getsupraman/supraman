//! The commit graph's rows: lanes, edges, ref groups and ghost refs, laid out page by page off the UI thread.

mod groups;
mod lanes;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

pub use groups::{RefGroup, RefKind, RefLabel};
pub use lanes::{Edge, EdgeKind, LANE_COLOURS};

use crate::{Commit, Refs};
use groups::RefIndex;
use lanes::Lanes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// `None` on the WIP row.
    pub sha: Option<String>,
    pub lane: usize,
    pub colour: usize,
    pub edges: Vec<Edge>,
    /// The highest lane any edge or the node uses in this row.
    pub max_lane: usize,
    /// In order; the row shows the first and `+(len - 1)`.
    pub refs: Arc<[RefGroup]>,
    /// The nearest descendant's branch groups, for a row with no refs of its own.
    pub ghost: Option<Arc<[RefGroup]>>,
    pub detached_head: bool,
}

impl Row {
    /// The `+N` beside the first group.
    pub fn more(&self) -> usize {
        self.refs.len().saturating_sub(1)
    }
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
    wip: bool,
    rows: usize,
    none: Arc<[RefGroup]>,
}

impl Layout {
    /// With `wip`, the first page starts with a WIP row parented to HEAD's commit, when HEAD has one.
    pub fn new(refs: &Refs, wip: bool) -> Layout {
        let refs = RefIndex::new(refs);
        let wip = wip && refs.head.is_some();
        Layout { lanes: Lanes::default(), refs: Arc::new(refs), handed: HashMap::new(), wip, rows: 0, none: Arc::from([]) }
    }

    /// The rows of the next page; pages laid out in turn give the rows of laying them out at once.
    pub fn extend(&mut self, commits: &[Commit]) -> Vec<Row> {
        let mut rows = Vec::with_capacity(commits.len() + 1);
        if std::mem::take(&mut self.wip) {
            let head = self.refs.head.clone().into_iter().collect::<Vec<_>>();
            let placed = self.lanes.place("", &head, true);
            rows.push(self.row(None, placed, None));
        }
        for commit in commits {
            let placed = self.lanes.place(&commit.sha, &commit.parents, false);
            let ghost = self.ghost(&commit.sha, &commit.parents);
            rows.push(self.row(Some(commit.sha.clone()), placed, ghost));
        }
        rows
    }

    fn row(&mut self, sha: Option<String>, placed: lanes::Placed, ghost: Option<Arc<[RefGroup]>>) -> Row {
        self.rows += 1;
        let refs = sha.as_ref().and_then(|s| self.refs.groups.get(s)).cloned().unwrap_or_else(|| self.none.clone());
        let detached_head = sha.is_some() && sha == self.refs.detached;
        Row { sha, lane: placed.lane, colour: placed.lane % LANE_COLOURS, edges: placed.edges, max_lane: placed.max_lane, refs, ghost, detached_head }
    }

    /// A commit with branch refs hands them to its parents; one with no refs takes the best it was handed and
    /// hands that on.
    fn ghost(&mut self, sha: &str, parents: &[String]) -> Option<Arc<[RefGroup]>> {
        let handed = self.handed.remove(sha).map(|h| h.groups);
        let (ghost, pass) = match self.refs.branches.get(sha) {
            Some(own) => (None, Some(own.clone())),
            None if self.refs.groups.contains_key(sha) => (None, None),
            None => (handed.clone(), handed),
        };
        if let Some(groups) = pass {
            for (i, parent) in parents.iter().enumerate() {
                self.hand(parent, (i, self.rows), &groups);
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

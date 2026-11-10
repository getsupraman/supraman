use std::collections::HashMap;
use std::sync::Arc;

use super::edges::{Edges, open_set};

/// The theme's lane palette has this many colours; a lane paints in `lane % LANE_COLOURS`.
pub const LANE_COLOURS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// From this row's node down to a parent.
    Starting,
    PassThrough,
    /// A carried lane reaching this row's node.
    Ending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Edge {
    /// The lane the edge runs in above (ending) or below (starting) the row's middle.
    pub lane: usize,
    pub kind: EdgeKind,
    /// The node's lane the edge curves to; `lane` for a pass-through.
    pub node: usize,
    pub colour: usize,
    /// The WIP row's edge to HEAD.
    pub dashed: bool,
}

/// Who opened a lane waiting for a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum By {
    Commit,
    Stash,
    /// A stash older than a real child already waiting: the parent keeps the child's lane.
    OlderStash,
}

/// The open lanes between rows, each `Some(dashed)`, and the lanes waiting for each commit not placed yet.
#[derive(Debug, Clone, Default)]
pub(super) struct Lanes {
    carry: Vec<Option<bool>>,
    waiting: HashMap<String, Vec<(usize, By)>>,
    /// The last row's open lanes, for the next row to share.
    open: Option<Arc<[u64]>>,
}

pub(super) struct Placed {
    pub lane: usize,
    pub edges: Edges,
    pub max_lane: usize,
}

impl Lanes {
    /// Places one row in the lowest lane reserved for it, else the lowest free; its first parent continues there.
    pub fn place<P: AsRef<str>>(&mut self, sha: &str, parents: &[P], dashed: bool, stash: bool) -> Placed {
        let waiting = self.waiting.remove(sha).unwrap_or_default();
        let mut incoming: Vec<usize> = waiting.iter().map(|(l, _)| *l).collect();
        incoming.sort_unstable();
        let lane = kept(&waiting).unwrap_or_else(|| self.lowest_free(None, &incoming));
        let open = self.open_above();
        let mut ending: Vec<usize> = incoming.iter().copied().filter(|&l| self.carry.get(l).is_some_and(Option::is_some)).collect();
        ending.dedup();
        let mut max_lane = self.carry.len().saturating_sub(1).max(lane);
        let mut starting = Vec::with_capacity(parents.len());
        for &l in &incoming {
            self.carry[l] = None;
        }
        for (i, parent) in parents.iter().map(AsRef::as_ref).enumerate() {
            if parents[..i].iter().any(|p| p.as_ref() == parent) {
                continue;
            }
            let waiting = self.waiting.get(parent).and_then(|w| kept(w));
            let by = match stash {
                false => By::Commit,
                true if self.waiting.get(parent).is_some_and(|w| w.iter().any(|(_, by)| *by == By::Commit)) => By::OlderStash,
                true => By::Stash,
            };
            let target = match (i, waiting) {
                (0, _) => self.open(lane, parent, dashed, by),
                (_, Some(l)) => l,
                (_, None) => {
                    let l = self.lowest_free(Some(lane), &incoming);
                    self.open(l, parent, dashed, by)
                }
            };
            starting.push(target);
            max_lane = max_lane.max(target);
        }
        while matches!(self.carry.last(), Some(None)) {
            self.carry.pop();
        }
        Placed { lane, edges: Edges::new(open, &ending, &starting, lane, dashed), max_lane }
    }

    /// The lanes open above the row, the last row's when they are the same.
    fn open_above(&mut self) -> Arc<[u64]> {
        let set = open_set(&self.carry);
        match &self.open {
            Some(last) if **last == *set => last.clone(),
            _ => self.open.insert(Arc::from(set)).clone(),
        }
    }

    fn open(&mut self, lane: usize, parent: &str, dashed: bool, by: By) -> usize {
        if self.carry.len() <= lane {
            self.carry.resize(lane + 1, None);
        }
        self.carry[lane] = Some(dashed);
        self.waiting.entry(parent.to_string()).or_default().push((lane, by));
        lane
    }

    /// The lowest lane nothing carries, other than the node's and those ending at this row.
    fn lowest_free(&self, node: Option<usize>, ending: &[usize]) -> usize {
        (0..).find(|&l| Some(l) != node && !ending.contains(&l) && self.carry.get(l).is_none_or(|c| c.is_none())).unwrap_or(0)
    }
}

/// The lane a commit takes of those waiting for it: the lowest one not opened by an older stash, else the lowest.
fn kept(waiting: &[(usize, By)]) -> Option<usize> {
    let lowest = |older: bool| waiting.iter().filter(|(_, by)| older || *by != By::OlderStash).map(|(l, _)| *l).min();
    lowest(false).or_else(|| lowest(true))
}

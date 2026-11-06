use std::collections::HashMap;

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
}

pub(super) struct Placed {
    pub lane: usize,
    pub edges: Vec<Edge>,
    pub max_lane: usize,
}

impl Lanes {
    /// Places one row: the lowest lane reserved for it, else the lowest free; its first parent continues in
    /// its lane, a further parent joins the lane already waiting for it or opens the lowest free one.
    pub fn place(&mut self, sha: &str, parents: &[String], dashed: bool, stash: bool) -> Placed {
        let waiting = self.waiting.remove(sha).unwrap_or_default();
        let mut incoming: Vec<usize> = waiting.iter().map(|(l, _)| *l).collect();
        incoming.sort_unstable();
        let lane = kept(&waiting).unwrap_or_else(|| self.lowest_free(None, &incoming));
        let mut edges = self.through(lane, &incoming);
        for &l in &incoming {
            self.carry[l] = None;
        }
        for (i, parent) in parents.iter().enumerate() {
            if parents[..i].contains(parent) {
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
            edges.push(Edge { lane: target, kind: EdgeKind::Starting, node: lane, colour: target % LANE_COLOURS, dashed });
        }
        while matches!(self.carry.last(), Some(None)) {
            self.carry.pop();
        }
        let max_lane = edges.iter().map(|e| e.lane).chain([lane]).max().unwrap_or(lane);
        Placed { lane, edges, max_lane }
    }

    /// The edges of every lane open above the row: ending at it or passing it.
    fn through(&self, lane: usize, incoming: &[usize]) -> Vec<Edge> {
        let open = self.carry.iter().enumerate().filter_map(|(l, c)| c.map(|dashed| (l, dashed)));
        open.map(|(l, dashed)| {
            let (kind, node) = if incoming.contains(&l) { (EdgeKind::Ending, lane) } else { (EdgeKind::PassThrough, l) };
            Edge { lane: l, kind, node, colour: l % LANE_COLOURS, dashed }
        })
        .collect()
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

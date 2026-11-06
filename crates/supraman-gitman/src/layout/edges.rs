use std::sync::Arc;

use super::lanes::{Edge, EdgeKind, LANE_COLOURS};

/// A row's edges, compact: the lanes open above it, those ending at its node and those starting from it; each
/// [`Edge`] is derived as it is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Edges {
    /// Two bitsets of equal length, the lanes open above the row and which of them are dashed; shared with the
    /// row above while they stay the same.
    open: Arc<[u64]>,
    /// The lanes ending at the node, ascending, then the lanes starting from it, in parent order.
    lanes: Box<[u32]>,
    ending: u16,
    node: u32,
    /// The starting edges'.
    dashed: bool,
}

impl Edges {
    pub fn new(open: Arc<[u64]>, ending: &[usize], starting: &[usize], node: usize, dashed: bool) -> Edges {
        let lanes = ending.iter().chain(starting).map(|&l| narrow(l)).collect();
        let ending = u16::try_from(ending.len()).expect("fewer than 65536 lanes end at one commit");
        Edges { open, lanes, ending, node: narrow(node), dashed }
    }

    /// Ending and passing edges by lane, then the starting ones.
    pub fn iter(&self) -> impl Iterator<Item = Edge> + '_ {
        let (carried, dashed) = self.open.split_at(self.open.len() / 2);
        let (ending, starting) = self.lanes.split_at(usize::from(self.ending));
        let node = self.node as usize;
        let through = bits(carried).map(move |lane| {
            let (kind, node) = if ending.contains(&narrow(lane)) { (EdgeKind::Ending, node) } else { (EdgeKind::PassThrough, lane) };
            Edge { lane, kind, node, colour: lane % LANE_COLOURS, dashed: has(dashed, lane) }
        });
        let starting = starting.iter().map(move |&lane| {
            let lane = lane as usize;
            Edge { lane, kind: EdgeKind::Starting, node, colour: lane % LANE_COLOURS, dashed: self.dashed }
        });
        through.chain(starting)
    }
}

fn narrow(lane: usize) -> u32 {
    u32::try_from(lane).expect("a lane fits in u32")
}

fn has(set: &[u64], lane: usize) -> bool {
    set.get(lane / 64).is_some_and(|w| w >> (lane % 64) & 1 == 1)
}

fn bits(set: &[u64]) -> impl Iterator<Item = usize> + '_ {
    set.iter().enumerate().flat_map(|(i, &word)| {
        std::iter::successors((word != 0).then_some(word), |w| Some(w & (w - 1)).filter(|w| *w != 0)).map(move |w| i * 64 + w.trailing_zeros() as usize)
    })
}

/// The bitsets of [`Edges::open`] for lanes `carry`, each `Some(dashed)` while open.
pub(super) fn open_set(carry: &[Option<bool>]) -> Vec<u64> {
    let words = carry.len().div_ceil(64);
    let mut set = vec![0u64; 2 * words];
    for (lane, c) in carry.iter().enumerate() {
        if let Some(dashed) = c {
            set[lane / 64] |= 1 << (lane % 64);
            if *dashed {
                set[words + lane / 64] |= 1 << (lane % 64);
            }
        }
    }
    set
}

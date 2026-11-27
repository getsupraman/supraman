//! The dock's layout: a split tree of depth ≤ 2 with alternating axes over panel ids, and its placement rules as
//! pure functions. A refused placement is `None`.

pub use crate::dnd::Zone;
pub use crate::split::Axis;

/// A new split's first share.
pub const RATIO: f32 = 0.5;
/// Most panels a dock holds.
pub const MAX: usize = 4;
/// Most panels [`Layout::add`] places; a fourth only lands by a drop.
pub const ADD_MAX: usize = 3;
/// Deepest a leaf lies: quarters.
pub const DEPTH: usize = 2;

/// A dock's panels: a leaf, or a split whose `ratio` is `first`'s share of its room along `axis`.
#[derive(Clone, Debug, PartialEq)]
pub enum Layout<P> {
    Leaf(P),
    Split { axis: Axis, ratio: f32, first: Box<Layout<P>>, second: Box<Layout<P>> },
}

/// The axis an edge splits along and whether the dropped panel comes first; the centre has none.
fn edge(zone: Zone) -> Option<(Axis, bool)> {
    match zone {
        Zone::Left => Some((Axis::Columns, true)),
        Zone::Right => Some((Axis::Columns, false)),
        Zone::Top => Some((Axis::Rows, true)),
        Zone::Bottom => Some((Axis::Rows, false)),
        Zone::Centre => None,
    }
}

/// What a drop does: a new layout, or the target's own drop (its centre), which leaves the layout as it is.
#[derive(Clone, Debug, PartialEq)]
pub enum Dropped<P> {
    Layout(Layout<P>),
    Owner,
}

/// A slot's rectangle in the dock's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    fn cut(self, axis: Axis, ratio: f32) -> (Rect, Rect) {
        match axis {
            Axis::Columns => {
                let w = self.width * ratio;
                (Rect { width: w, ..self }, Rect { x: self.x + w, width: self.width - w, ..self })
            }
            Axis::Rows => {
                let h = self.height * ratio;
                (Rect { height: h, ..self }, Rect { y: self.y + h, height: self.height - h, ..self })
            }
        }
    }
}

fn across(axis: Axis) -> Axis {
    match axis {
        Axis::Columns => Axis::Rows,
        Axis::Rows => Axis::Columns,
    }
}

fn pair<P>(axis: Axis, first: Layout<P>, second: Layout<P>) -> Layout<P> {
    Layout::Split { axis, ratio: RATIO, first: Box::new(first), second: Box::new(second) }
}

impl<P: Copy + Eq> Layout<P> {
    /// The panels, each split's first side before its second.
    pub fn panels(&self) -> Vec<P> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<P>) {
        match self {
            Layout::Leaf(p) => out.push(*p),
            Layout::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Layout::Leaf(_) => 1,
            Layout::Split { first, second, .. } => first.len() + second.len(),
        }
    }

    pub fn contains(&self, p: P) -> bool {
        match self {
            Layout::Leaf(q) => *q == p,
            Layout::Split { first, second, .. } => first.contains(p) || second.contains(p),
        }
    }

    /// Depth ≤ [`DEPTH`], alternating axes, every ratio in (0, 1) and each panel once.
    pub fn is_valid(&self) -> bool {
        let panels = self.panels();
        let unique = panels.iter().enumerate().all(|(i, p)| !panels[..i].contains(p));
        unique && self.valid_at(0, None)
    }

    fn valid_at(&self, depth: usize, parent: Option<Axis>) -> bool {
        match self {
            Layout::Leaf(_) => true,
            Layout::Split { axis, ratio, first, second } => {
                depth < DEPTH
                    && parent != Some(*axis)
                    && *ratio > 0.0
                    && *ratio < 1.0
                    && first.valid_at(depth + 1, Some(*axis))
                    && second.valid_at(depth + 1, Some(*axis))
            }
        }
    }

    /// The menu's placement: one panel splits left|right (new right), then the second half splits across (new
    /// second), then refused.
    pub fn add(&self, p: P) -> Option<Self> {
        if self.contains(p) {
            return None;
        }
        match self {
            Layout::Leaf(_) => Some(pair(Axis::Columns, self.clone(), Layout::Leaf(p))),
            Layout::Split { axis, ratio, first, second } if self.len() < ADD_MAX => {
                let half = pair(across(*axis), (**second).clone(), Layout::Leaf(p));
                Some(Layout::Split { axis: *axis, ratio: *ratio, first: first.clone(), second: Box::new(half) })
            }
            Layout::Split { .. } => None,
        }
    }

    /// `p` dropped on `target`'s `zone`: an edge splits the target's slot where depth and the alternating axes
    /// allow; refused for a missing target, a full dock or a `p` already here.
    pub fn drop(&self, target: P, zone: Zone, p: P) -> Option<Dropped<P>> {
        if !self.contains(target) {
            return None;
        }
        let Some((axis, before)) = edge(zone) else { return Some(Dropped::Owner) };
        if self.contains(p) || self.len() >= MAX {
            return None;
        }
        self.split_at(target, axis, before, p, 0, None).map(Dropped::Layout)
    }

    fn split_at(&self, target: P, axis: Axis, before: bool, p: P, depth: usize, parent: Option<Axis>) -> Option<Self> {
        match self {
            Layout::Leaf(q) => {
                let fits = *q == target && depth < DEPTH && parent != Some(axis);
                fits.then(|| if before { pair(axis, Layout::Leaf(p), self.clone()) } else { pair(axis, self.clone(), Layout::Leaf(p)) })
            }
            Layout::Split { axis: own, ratio, first, second } => {
                let at = |side: &Layout<P>| side.split_at(target, axis, before, p, depth + 1, Some(*own));
                let (first, second) = match at(first) {
                    Some(f) => (Box::new(f), second.clone()),
                    None => (first.clone(), Box::new(at(second)?)),
                };
                Some(Layout::Split { axis: *own, ratio: *ratio, first, second })
            }
        }
    }

    /// `p` taken out: its sibling takes the split's whole room and keeps its own ratio, the removed split's ratio
    /// goes with it; the only panel cannot go.
    pub fn remove(&self, p: P) -> Option<Self> {
        let Layout::Split { axis, ratio, first, second } = self else { return None };
        let leaf = |side: &Layout<P>| matches!(side, Layout::Leaf(q) if *q == p);
        if leaf(first) {
            return Some((**second).clone());
        }
        if leaf(second) {
            return Some((**first).clone());
        }
        let (first, second) = match first.remove(p) {
            Some(f) => (Box::new(f), second.clone()),
            None => (first.clone(), Box::new(second.remove(p)?)),
        };
        Some(Layout::Split { axis: *axis, ratio: *ratio, first, second })
    }

    /// `p` removed, then dropped on `target`'s `zone`; `None` for a move onto itself or one that leaves every
    /// panel where it was.
    pub fn moved(&self, p: P, target: P, zone: Zone) -> Option<Dropped<P>> {
        if p == target || !self.contains(p) {
            return None;
        }
        if zone == Zone::Centre {
            return self.drop(target, zone, p);
        }
        match self.remove(p)?.drop(target, zone, p)? {
            Dropped::Layout(landed) => (!landed.same_places(self)).then_some(Dropped::Layout(landed)),
            Dropped::Owner => Some(Dropped::Owner),
        }
    }

    /// The same panels in the same slots, whatever the ratios.
    pub fn same_places(&self, other: &Self) -> bool {
        match (self, other) {
            (Layout::Leaf(a), Layout::Leaf(b)) => a == b,
            (Layout::Split { axis: a, first: af, second: asd, .. }, Layout::Split { axis: b, first: bf, second: bs, .. }) => {
                a == b && af.same_places(bf) && asd.same_places(bs)
            }
            _ => false,
        }
    }

    /// The split at `path` (each step a split's side, 0 its first) given `ratio`; `None` for no split there or a ratio
    /// outside (0, 1).
    pub fn with_ratio(&self, path: &[usize], ratio: f32) -> Option<Self> {
        let Layout::Split { axis, ratio: own, first, second } = self else { return None };
        match path.split_first() {
            None => (ratio > 0.0 && ratio < 1.0).then(|| Layout::Split { axis: *axis, ratio, first: first.clone(), second: second.clone() }),
            Some((0, rest)) => Some(Layout::Split { axis: *axis, ratio: *own, first: Box::new(first.with_ratio(rest, ratio)?), second: second.clone() }),
            Some((1, rest)) => Some(Layout::Split { axis: *axis, ratio: *own, first: first.clone(), second: Box::new(second.with_ratio(rest, ratio)?) }),
            Some(_) => None,
        }
    }

    /// Each panel's slot in a dock of this size, in [`Layout::panels`]' order.
    pub fn rects(&self, width: f32, height: f32) -> Vec<(P, Rect)> {
        let mut out = Vec::new();
        self.place(Rect { x: 0.0, y: 0.0, width, height }, &mut out);
        out
    }

    fn place(&self, room: Rect, out: &mut Vec<(P, Rect)>) {
        match self {
            Layout::Leaf(p) => out.push((*p, room)),
            Layout::Split { axis, ratio, first, second } => {
                let (a, b) = room.cut(*axis, *ratio);
                first.place(a, out);
                second.place(b, out);
            }
        }
    }
}

#[cfg(test)]
mod tests;

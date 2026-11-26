//! Where a list's keys move its selection over the rows that act: wrapping steps, the ends, and type-ahead.

/// The next item that acts after `from` (before it when `back`), wrapping; the first (last) from none.
pub fn step(acts: &[bool], from: Option<usize>, back: bool) -> Option<usize> {
    let n = acts.len();
    let start = match (from, back) {
        (Some(at), false) => at + 1,
        (Some(at), true) => at + n - 1,
        (None, false) => 0,
        (None, true) => n.wrapping_sub(1),
    };
    (0..n).map(|k| if back { (start + n - k) % n } else { (start + k) % n }).find(|&i| acts[i])
}

/// The first item that acts (the last when `back`).
pub fn end(acts: &[bool], back: bool) -> Option<usize> {
    step(acts, None, back)
}

/// Type-ahead: a repeated letter steps to the next item after `from` starting with it; longer text finds the first
/// item from `from` on starting with it; only items that act, wrapping.
pub fn typed(labels: &[(&str, bool)], from: Option<usize>, typed: &str) -> Option<usize> {
    let n = labels.len();
    let typed = typed.to_lowercase();
    let mut chars = typed.chars();
    let first = chars.next()?;
    let (prefix, start) = if chars.all(|c| c == first) { (first.to_string(), from.map_or(0, |at| at + 1)) } else { (typed.clone(), from.unwrap_or(0)) };
    (0..n).map(|k| (start + k) % n).find(|&i| labels[i].1 && labels[i].0.to_lowercase().starts_with(&prefix))
}

use super::*;

const IDS: [u8; 4] = [1, 2, 3, 4];
const W: f32 = 1000.0;
const H: f32 = 600.0;
const EPS: f32 = 1e-3;

fn leaf(p: u8) -> Layout<u8> {
    Layout::Leaf(p)
}

fn split(axis: Axis, ratio: f32, first: Layout<u8>, second: Layout<u8>) -> Layout<u8> {
    Layout::Split { axis, ratio, first: Box::new(first), second: Box::new(second) }
}

fn cols(first: Layout<u8>, second: Layout<u8>) -> Layout<u8> {
    split(Axis::Columns, RATIO, first, second)
}

fn rows(first: Layout<u8>, second: Layout<u8>) -> Layout<u8> {
    split(Axis::Rows, RATIO, first, second)
}

fn landed(drop: Option<Dropped<u8>>) -> Layout<u8> {
    match drop {
        Some(Dropped::Layout(l)) => l,
        other => panic!("expected a layout, got {other:?}"),
    }
}

/// Every layout `add`, `drop`, `remove` and `moved` reach from one panel, over four ids.
fn reachable() -> Vec<Layout<u8>> {
    let mut seen = vec![leaf(1)];
    let mut next = 0;
    while next < seen.len() {
        let layout = seen[next].clone();
        next += 1;
        let mut found = Vec::new();
        for p in IDS {
            found.extend(layout.add(p));
            found.extend(layout.remove(p));
            for target in IDS {
                for zone in Zone::ALL {
                    for d in [layout.drop(target, zone, p), layout.moved(p, target, zone)] {
                        if let Some(Dropped::Layout(l)) = d {
                            found.push(l);
                        }
                    }
                }
            }
        }
        for l in found {
            if !seen.contains(&l) {
                seen.push(l);
            }
        }
    }
    seen
}

/// Every valid layout over four ids built directly from the shapes: a leaf, a split of two sides each a leaf or a
/// split across.
fn every_valid() -> Vec<Layout<u8>> {
    fn sides(axis: Axis, ids: &[u8]) -> Vec<Layout<u8>> {
        match ids {
            [p] => vec![leaf(*p)],
            [a, b] => vec![split(axis, RATIO, leaf(*a), leaf(*b))],
            _ => vec![],
        }
    }
    fn orders(ids: &[u8]) -> Vec<Vec<u8>> {
        if ids.is_empty() {
            return vec![vec![]];
        }
        let mut out = Vec::new();
        for (i, p) in ids.iter().enumerate() {
            let rest: Vec<u8> = ids.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, q)| *q).collect();
            for mut o in orders(&rest) {
                o.insert(0, *p);
                out.push(o);
            }
        }
        out
    }
    let mut out: Vec<Layout<u8>> = IDS.iter().map(|p| leaf(*p)).collect();
    let mut chosen = Vec::new();
    for n in 2..=4usize {
        for mask in 0u8..16 {
            let set: Vec<u8> = IDS.iter().copied().filter(|p| mask & (1 << (p - 1)) != 0).collect();
            if set.len() == n {
                chosen.push(set);
            }
        }
    }
    for set in chosen {
        for order in orders(&set) {
            for axis in [Axis::Columns, Axis::Rows] {
                for cut in 1..order.len() {
                    for a in sides(across(axis), &order[..cut]) {
                        for b in sides(across(axis), &order[cut..]) {
                            out.push(split(axis, RATIO, a.clone(), b));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Each split's ratio set apart from the others and from 0.5, so a ratio kept on the wrong split shows in the rects.
fn varied(layout: &Layout<u8>, at: &mut usize) -> Layout<u8> {
    const RATIOS: [f32; 3] = [0.3, 0.7, 0.8];
    match layout {
        Layout::Leaf(_) => layout.clone(),
        Layout::Split { axis, first, second, .. } => {
            let ratio = RATIOS[*at % RATIOS.len()];
            *at += 1;
            let first = varied(first, at);
            split(*axis, ratio, first, varied(second, at))
        }
    }
}

fn contains(outer: &Rect, inner: &Rect) -> bool {
    inner.x >= outer.x - EPS
        && inner.y >= outer.y - EPS
        && inner.x + inner.width <= outer.x + outer.width + EPS
        && inner.y + inner.height <= outer.y + outer.height + EPS
}

fn overlap(a: &Rect, b: &Rect) -> f32 {
    let w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    w.max(0.0) * h.max(0.0)
}

/// The rects lie in the dock, cover it and do not overlap.
fn tiles(layout: &Layout<u8>) {
    let rects = layout.rects(W, H);
    let dock = Rect { x: 0.0, y: 0.0, width: W, height: H };
    let area: f32 = rects.iter().map(|(_, r)| r.width * r.height).sum();
    assert!((area - W * H).abs() < 1.0, "{layout:?} covers {area} of {}", W * H);
    for (i, (p, a)) in rects.iter().enumerate() {
        assert!(a.width > 0.0 && a.height > 0.0 && contains(&dock, a), "{p}'s {a:?} in {layout:?}");
        for (q, b) in &rects[i + 1..] {
            assert!(overlap(a, b) < 1.0, "{p} and {q} overlap in {layout:?}");
        }
    }
}

#[test]
fn dock_layout_reachable_is_every_valid_layout_of_one_to_four() {
    let found = reachable();
    let valid = every_valid();
    let count = |set: &[Layout<u8>], n| set.iter().filter(|l| l.len() == n).count();
    assert_eq!([1, 2, 3, 4].map(|n| count(&found, n)), [4, 24, 96, 48]);
    assert_eq!(found.len(), 172);
    assert_eq!(valid.len(), found.len());
    for l in &found {
        assert!(l.is_valid(), "{l:?}");
        assert!(valid.contains(l), "{l:?} is reachable but not a valid shape");
        tiles(l);
    }
}

#[test]
fn dock_layout_add_places_right_then_below_then_refuses() {
    let two = leaf(1).add(2).unwrap();
    assert_eq!(two, cols(leaf(1), leaf(2)));
    let three = two.add(3).unwrap();
    assert_eq!(three, cols(leaf(1), rows(leaf(2), leaf(3))));
    assert_eq!(three.add(4), None, "the menu places three at most");
    assert_eq!(two.add(2), None, "an id is in a layout once");
    assert_eq!(leaf(1).add(1), None);
    assert_eq!(rows(leaf(1), leaf(2)).add(3), Some(rows(leaf(1), cols(leaf(2), leaf(3)))));
    for l in reachable() {
        for p in IDS {
            if let Some(added) = l.add(p) {
                assert!(added.is_valid() && added.len() == l.len() + 1 && added.len() <= ADD_MAX, "{l:?} + {p}");
            }
        }
    }
}

#[test]
fn dock_layout_drop_splits_the_target_along_its_edge() {
    assert_eq!(landed(leaf(1).drop(1, Zone::Left, 2)), cols(leaf(2), leaf(1)));
    assert_eq!(landed(leaf(1).drop(1, Zone::Right, 2)), cols(leaf(1), leaf(2)));
    assert_eq!(landed(leaf(1).drop(1, Zone::Top, 2)), rows(leaf(2), leaf(1)));
    assert_eq!(landed(leaf(1).drop(1, Zone::Bottom, 2)), rows(leaf(1), leaf(2)));
    let three = cols(leaf(1), rows(leaf(2), leaf(3)));
    assert_eq!(landed(three.drop(1, Zone::Bottom, 4)), cols(rows(leaf(1), leaf(4)), rows(leaf(2), leaf(3))));
    assert_eq!(three.drop(2, Zone::Centre, 4), Some(Dropped::Owner), "the centre is the owner's");
}

#[test]
fn dock_layout_drop_refuses_depth_capacity_the_same_id_and_a_missing_target() {
    let two = cols(leaf(1), leaf(2));
    let three = cols(leaf(1), rows(leaf(2), leaf(3)));
    let four = cols(rows(leaf(1), leaf(4)), rows(leaf(2), leaf(3)));
    assert_eq!(three.drop(2, Zone::Left, 4), None, "depth: a quarter does not split again");
    assert_eq!(three.drop(2, Zone::Top, 4), None);
    assert_eq!(two.drop(2, Zone::Right, 3), None, "the axes alternate: a half does not split along its own split");
    for target in IDS {
        for zone in [Zone::Left, Zone::Right, Zone::Top, Zone::Bottom] {
            assert_eq!(four.drop(target, zone, 5), None, "capacity: four panels");
        }
    }
    assert_eq!(two.drop(1, Zone::Bottom, 2), None, "the same id");
    assert_eq!(two.drop(1, Zone::Bottom, 1), None);
    assert_eq!(two.drop(3, Zone::Bottom, 4), None, "a missing target");
    assert_eq!(two.drop(3, Zone::Centre, 4), None, "a missing target's centre");
    for l in reachable() {
        for (target, zone, p) in IDS.iter().flat_map(|t| Zone::ALL.map(move |z| (*t, z))).flat_map(|(t, z)| IDS.map(move |p| (t, z, p))) {
            match l.drop(target, zone, p) {
                Some(Dropped::Layout(d)) => assert!(d.is_valid() && d.len() == l.len() + 1 && d.contains(p), "{l:?}"),
                Some(Dropped::Owner) => assert!(zone == Zone::Centre && l.contains(target)),
                None => assert!(!l.contains(target) || zone != Zone::Centre),
            }
        }
    }
}

#[test]
fn dock_layout_remove_gives_the_room_to_the_sibling() {
    assert_eq!(leaf(1).remove(1), None, "the only panel stays");
    assert_eq!(cols(leaf(1), leaf(2)).remove(3), None);
    let three = split(Axis::Columns, 0.3, leaf(1), split(Axis::Rows, 0.8, leaf(2), leaf(3)));
    assert_eq!(three.remove(1), Some(split(Axis::Rows, 0.8, leaf(2), leaf(3))));
    assert_eq!(three.remove(3), Some(split(Axis::Columns, 0.3, leaf(1), leaf(2))));
}

#[test]
fn dock_layout_remove_of_any_leaf_leaves_a_valid_layout_that_tiles_the_dock() {
    let mut removals = 0;
    for l in reachable() {
        let l = varied(&l, &mut 0);
        let before = l.rects(W, H);
        for p in l.panels() {
            let Some(r) = l.remove(p) else {
                assert_eq!(l.len(), 1, "{l:?} keeps {p}");
                continue;
            };
            removals += 1;
            assert!(r.is_valid(), "{l:?} - {p}");
            let mut left = l.panels();
            left.retain(|q| *q != p);
            assert_eq!(r.panels(), left);
            tiles(&r);
            for (q, now) in r.rects(W, H) {
                let was = before.iter().find(|(o, _)| *o == q).map(|(_, r)| *r).unwrap();
                assert!(contains(&now, &was), "{q} shrank from {was:?} to {now:?} as {p} left {l:?}");
            }
        }
    }
    assert_eq!(removals, 24 * 2 + 96 * 3 + 48 * 4);
}

#[test]
fn dock_layout_moved_to_its_own_slot_is_none() {
    let two = cols(leaf(1), leaf(2));
    for zone in Zone::ALL {
        assert_eq!(two.moved(1, 1, zone), None, "onto itself");
    }
    assert_eq!(two.moved(1, 2, Zone::Left), None, "already left of 2");
    assert_eq!(two.moved(2, 1, Zone::Right), None);
    assert_eq!(landed(two.moved(1, 2, Zone::Right)), cols(leaf(2), leaf(1)));
    assert_eq!(landed(two.moved(1, 2, Zone::Top)), rows(leaf(1), leaf(2)));
    assert_eq!(two.moved(1, 2, Zone::Centre), Some(Dropped::Owner));
    assert_eq!(two.moved(3, 2, Zone::Left), None, "a panel not here");
    for l in reachable() {
        let l = varied(&l, &mut 0);
        for p in l.panels() {
            for zone in Zone::ALL {
                assert_eq!(l.moved(p, p, zone), None, "{l:?}");
            }
            for target in IDS {
                for zone in [Zone::Left, Zone::Right, Zone::Top, Zone::Bottom] {
                    if let Some(Dropped::Layout(m)) = l.moved(p, target, zone) {
                        assert!(m.is_valid() && !m.same_places(&l), "{l:?}: {p} to {target} {zone:?}");
                        let (mut a, mut b) = (m.panels(), l.panels());
                        a.sort();
                        b.sort();
                        assert_eq!(a, b);
                    }
                }
            }
        }
    }
}

#[test]
fn dock_layout_rects_and_panels_follow_the_splits() {
    let three = split(Axis::Columns, 0.3, leaf(1), split(Axis::Rows, 0.8, leaf(2), leaf(3)));
    assert_eq!(three.panels(), vec![1, 2, 3]);
    let rects = three.rects(W, H);
    let at = |p: u8| rects.iter().find(|(q, _)| *q == p).unwrap().1;
    assert_eq!(at(1), Rect { x: 0.0, y: 0.0, width: 300.0, height: 600.0 });
    assert_eq!(at(2), Rect { x: 300.0, y: 0.0, width: 700.0, height: 480.0 });
    assert_eq!(at(3), Rect { x: 300.0, y: 480.0, width: 700.0, height: 120.0 });
    assert!(!split(Axis::Columns, 1.0, leaf(1), leaf(2)).is_valid());
    assert!(!cols(leaf(1), cols(leaf(2), leaf(3))).is_valid(), "the axes alternate");
    assert!(!cols(leaf(1), leaf(1)).is_valid(), "an id once");
    assert!(!cols(leaf(1), rows(leaf(2), cols(leaf(3), leaf(4)))).is_valid(), "depth");
}

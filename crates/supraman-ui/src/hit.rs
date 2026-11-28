//! The selftests' click targets: every clickable element records where it was painted under a key.

use std::{
    cell::{Ref, RefCell, RefMut},
    collections::HashMap,
    rc::Rc,
};

use gpui::{Bounds, IntoElement, Pixels, Styled as _, canvas, px};

/// Where a clickable thing was painted, and the part of the window it can be seen in: its clip, i.e.
/// the visible area of the scroll container around it (the whole window when there is none).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub bounds: Bounds<Pixels>,
    pub visible: Bounds<Pixels>,
}

/// How far an element's snapped bounds may pass its clip and still count as inside: one device pixel at
/// a fractional scale factor.
pub const SNAP: Pixels = px(1.);

impl Hit {
    pub fn fully_visible(&self) -> bool {
        let (b, v) = (self.bounds, self.visible);
        b.left() >= v.left() - SNAP && b.top() >= v.top() - SNAP && b.right() <= v.right() + SNAP && b.bottom() <= v.bottom() + SNAP
    }
}

/// [`Hit`]s keyed by the same string as their element id, as of the last painted frame. The selftest
/// clicks through these. A handle carries a scope: every key a view records or looks up through it, and every
/// key it names in another window-wide registry ([`HitTargets::key`]), is prefixed with it, so two views painted
/// in one window never share a key; the empty scope's keys are the keys as given.
#[derive(Clone, Default)]
pub struct HitTargets {
    map: Rc<RefCell<HashMap<String, Hit>>>,
    scope: Rc<RefCell<Rc<str>>>,
}

impl HitTargets {
    /// A handle on the same window's hits whose keys lie under `name` within this one's scope.
    pub fn scoped(&self, name: &str) -> Self {
        Self { map: self.map.clone(), scope: Rc::new(RefCell::new(format!("{}{name}/", self.scope()).into())) }
    }

    /// The prefix this handle puts before every key.
    pub fn scope(&self) -> Rc<str> {
        self.scope.borrow().clone()
    }

    /// A handle on the same hits and scope whose clones, apart from this one's, move together by [`HitTargets::rescope`].
    pub fn own(&self) -> Self {
        Self { map: self.map.clone(), scope: Rc::new(RefCell::new(self.scope())) }
    }

    /// This handle and every clone of it take `to`'s scope: a view painted in another place of its window.
    pub fn rescope(&self, to: &HitTargets) {
        if *self.scope.borrow() != *to.scope.borrow() {
            *self.scope.borrow_mut() = to.scope();
        }
    }

    /// `key` as the window's registries hold it: hits, focus panes, split ids, popup anchors, cut labels, drags.
    pub fn key(&self, key: &str) -> String {
        format!("{}{key}", self.scope.borrow())
    }

    pub fn insert(&self, key: &str, hit: Hit) {
        self.map.borrow_mut().insert(self.key(key), hit);
    }

    pub fn remove(&self, key: &str) {
        self.map.borrow_mut().remove(&self.key(key));
    }

    pub fn get(&self, key: &str) -> Option<Hit> {
        self.map.borrow().get(&self.key(key)).copied()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.borrow().contains_key(&self.key(key))
    }

    /// Every view's hits in the window, under their full keys.
    pub fn borrow(&self) -> Ref<'_, HashMap<String, Hit>> {
        self.map.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, HashMap<String, Hit>> {
        self.map.borrow_mut()
    }
}

/// An invisible overlay that records its parent's painted bounds and clip under `key`.
pub fn hit_target(hits: &HitTargets, key: String) -> impl IntoElement {
    let hits = hits.clone();
    canvas(
        move |bounds, window, _| {
            let visible = window.content_mask().bounds;
            hits.insert(&key, Hit { bounds, visible });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use gpui::{point, size};

    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn an_own_handle_and_its_clones_rescope_together_and_leave_the_handle_they_came_from() {
        let root = HitTargets::default();
        let own = root.own();
        let child = own.clone();
        own.rescope(&root.scoped("p2"));
        assert_eq!((child.key("x"), own.key("x"), root.key("x")), ("p2/x".to_string(), "p2/x".to_string(), "x".to_string()));
        child.rescope(&root);
        assert_eq!(own.key("x"), "x");
    }

    #[test]
    fn a_hit_is_visible_only_when_wholly_inside_its_clip() {
        let tree = rect(0., 560., 480., 260.);
        assert!(Hit { bounds: rect(406., 700., 41., 19.), visible: tree }.fully_visible());
        assert!(Hit { bounds: rect(406., 560., 41., 260.), visible: tree }.fully_visible(), "edges touch");
        assert!(!Hit { bounds: rect(406., 804., 41., 19.33), visible: tree }.fully_visible());
        assert!(!Hit { bounds: rect(406., 846., 41., 19.), visible: tree }.fully_visible(), "below the fold");
        assert!(!Hit { bounds: rect(406., 550., 41., 19.), visible: tree }.fully_visible(), "under the section above");
        assert!(!Hit { bounds: rect(470., 700., 41., 19.), visible: tree }.fully_visible(), "past the right edge");
        let group = rect(1123., 383., 112., 22.);
        assert!(Hit { bounds: rect(1122.67, 382.67, 56., 22.67), visible: group }.fully_visible(), "snapped a device pixel past its clip");
    }
}

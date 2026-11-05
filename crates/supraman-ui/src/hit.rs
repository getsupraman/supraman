//! The selftests' click targets: every clickable element records where it was painted under a key.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

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
/// clicks through these.
pub type HitTargets = Rc<RefCell<HashMap<String, Hit>>>;

/// An invisible overlay that records its parent's painted bounds and clip under `key`.
pub fn hit_target(hits: &HitTargets, key: String) -> impl IntoElement {
    let hits = hits.clone();
    canvas(
        move |bounds, window, _| {
            let visible = window.content_mask().bounds;
            hits.borrow_mut().insert(key, Hit { bounds, visible });
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

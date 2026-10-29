//! The selftests' click targets: every clickable element records where it was painted under a key.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::{Bounds, IntoElement, Pixels, Styled as _, canvas};

/// Where a clickable thing was painted, and the part of the window it can be seen in: its clip, i.e.
/// the visible area of the scroll container around it (the whole window when there is none).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub bounds: Bounds<Pixels>,
    pub visible: Bounds<Pixels>,
}

impl Hit {
    pub fn fully_visible(&self) -> bool {
        let (b, v) = (self.bounds, self.visible);
        b.left() >= v.left() && b.top() >= v.top() && b.right() <= v.right() && b.bottom() <= v.bottom()
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
    use gpui::{point, px, size};

    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn a_hit_is_visible_only_when_wholly_inside_its_clip() {
        let tree = rect(0., 560., 480., 260.);
        assert!(Hit { bounds: rect(406., 700., 41., 19.), visible: tree }.fully_visible());
        assert!(Hit { bounds: rect(406., 560., 41., 260.), visible: tree }.fully_visible(), "edges touch");
        // m3/3: painted 804..823 under an 820 px window, centre inside, bottom outside
        assert!(!Hit { bounds: rect(406., 804., 41., 19.33), visible: tree }.fully_visible());
        assert!(!Hit { bounds: rect(406., 846., 41., 19.), visible: tree }.fully_visible(), "below the fold");
        assert!(!Hit { bounds: rect(406., 550., 41., 19.), visible: tree }.fully_visible(), "under the section above");
        assert!(!Hit { bounds: rect(470., 700., 41., 19.), visible: tree }.fully_visible(), "past the right edge");
    }
}

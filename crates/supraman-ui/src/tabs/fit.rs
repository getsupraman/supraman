//! The strip's widths and scroll, in plain numbers: a scroll is the distance the tabs are moved left, 0 to `content - viewport`.

use gpui::{Pixels, px};

/// The narrowest a tab gets: its icon, a few title characters and its ×. Past it the strip scrolls.
pub const TAB_MIN: Pixels = px(120.);
/// The widest a tab gets while the strip has room.
pub const TAB_MAX: Pixels = px(240.);
/// Below this an inactive tab shows its × only while hovered.
pub const CLOSE_ON_HOVER_BELOW: Pixels = px(150.);
/// How near an edge a dragged tab starts scrolling the strip.
pub const EDGE: Pixels = px(40.);
/// The most a drag scrolls per move, at the very edge.
pub const EDGE_STEP: Pixels = px(24.);

/// Each tab's width: the room shared evenly, between [`TAB_MIN`] and [`TAB_MAX`]; [`TAB_MAX`] while the room is unknown.
pub fn tab_width(room: Pixels, tabs: usize) -> Pixels {
    if tabs == 0 || room <= px(0.) {
        return TAB_MAX;
    }
    (room / tabs as f32).clamp(TAB_MIN, TAB_MAX)
}

pub fn max_scroll(content: Pixels, viewport: Pixels) -> Pixels {
    (content - viewport).max(px(0.))
}

/// The scroll that shows `start..end` whole, moving as little as it can from `scroll`.
pub fn revealed(scroll: Pixels, viewport: Pixels, content: Pixels, start: Pixels, end: Pixels) -> Pixels {
    let to = if start < scroll {
        start
    } else if end > scroll + viewport {
        end - viewport
    } else {
        scroll
    };
    to.clamp(px(0.), max_scroll(content, viewport))
}

/// The tab at `ix`'s span along the strip.
pub fn span(ix: usize, width: Pixels) -> (Pixels, Pixels) {
    (width * ix as f32, width * (ix + 1) as f32)
}

/// Whether tabs are cut off at the left and at the right edge.
pub fn cut(scroll: Pixels, viewport: Pixels, content: Pixels) -> (bool, bool) {
    let max = max_scroll(content, viewport);
    (scroll > px(0.5), max - scroll > px(0.5))
}

/// How far a drag at `x` (from the viewport's left) scrolls: towards the edge it is within [`EDGE`] of, more the nearer it is.
pub fn edge_scroll(x: Pixels, viewport: Pixels) -> Pixels {
    let depth = |into: Pixels| EDGE_STEP * ((EDGE - into.max(px(0.))) / EDGE).clamp(0., 1.);
    if x < EDGE {
        -depth(x)
    } else if x > viewport - EDGE {
        depth(viewport - x)
    } else {
        px(0.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_share_the_room_evenly_until_the_minimum_then_overflow() {
        assert_eq!(tab_width(px(1200.), 4), TAB_MAX, "few tabs stop at the maximum");
        assert_eq!(tab_width(px(1200.), 6), px(200.), "more share the room evenly");
        assert_eq!(tab_width(px(1200.), 10), TAB_MIN, "exactly the minimum");
        assert_eq!(tab_width(px(1200.), 30), TAB_MIN, "never below the minimum: the strip scrolls");
        assert_eq!(tab_width(px(0.), 3), TAB_MAX, "the room not yet measured");
        assert_eq!(tab_width(px(800.), 0), TAB_MAX);
        let room = px(1000.);
        for n in 1..=40 {
            let w = tab_width(room, n);
            assert!(w * n as f32 <= room + px(0.01) || w == TAB_MIN, "{n} tabs of {w:?} cut off before the minimum");
        }
    }

    #[test]
    fn a_revealed_tab_is_whole_in_view_with_the_least_scroll() {
        let (viewport, content) = (px(500.), px(3600.));
        assert_eq!(revealed(px(0.), viewport, content, px(120.), px(240.)), px(0.), "already in view");
        assert_eq!(revealed(px(0.), viewport, content, px(3480.), px(3600.)), px(3100.), "the last: its right edge at the viewport's");
        assert_eq!(revealed(px(3100.), viewport, content, px(0.), px(120.)), px(0.), "the first: its left edge at the viewport's");
        assert_eq!(revealed(px(1000.), viewport, content, px(960.), px(1080.)), px(960.), "cut at the left");
        assert_eq!(revealed(px(1000.), viewport, content, px(1440.), px(1560.)), px(1060.), "cut at the right");
        assert_eq!(revealed(px(0.), px(800.), px(600.), px(480.), px(600.)), px(0.), "no scroll while everything fits");
        assert_eq!(span(29, TAB_MIN), (px(3480.), px(3600.)));
    }

    #[test]
    fn a_fade_shows_on_each_cut_edge() {
        assert_eq!(cut(px(0.), px(500.), px(3600.)), (false, true));
        assert_eq!(cut(px(1000.), px(500.), px(3600.)), (true, true));
        assert_eq!(cut(px(3100.), px(500.), px(3600.)), (true, false));
        assert_eq!(cut(px(0.), px(800.), px(600.)), (false, false));
    }

    #[test]
    fn a_drag_near_an_edge_scrolls_towards_it() {
        let viewport = px(500.);
        assert_eq!(edge_scroll(px(250.), viewport), px(0.));
        assert_eq!(edge_scroll(px(0.), viewport), -EDGE_STEP);
        assert_eq!(edge_scroll(px(-30.), viewport), -EDGE_STEP, "past the edge: the full step");
        assert_eq!(edge_scroll(px(20.), viewport), -EDGE_STEP / 2.);
        assert_eq!(edge_scroll(px(500.), viewport), EDGE_STEP);
        assert_eq!(edge_scroll(px(480.), viewport), EDGE_STEP / 2.);
        assert_eq!(edge_scroll(px(460.), viewport), px(0.));
    }
}

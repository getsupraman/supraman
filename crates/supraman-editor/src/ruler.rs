//! The split diff's change ruler: removed runs in its left lane, added runs in its right, a box over the
//! rows in view; a click or drag in it scrolls there.

use std::ops::Range;

use gpui::{App, Bounds, Hsla, Pixels, Window, fill, point, px, size};

use crate::Hunk;

/// The ruler's width.
pub const WIDTH: f32 = 14.;
/// The least height a mark is painted at, so a one-row change in a long file still shows.
pub const MIN_MARK: f32 = 2.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Removed,
    Added,
}

/// A run of changed rows in a lane, in the split's aligned display rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub lane: Lane,
    pub rows: Range<u32>,
}

/// The display row each hunk starts on, the same on both sides once the spacers align them.
pub fn aligned_starts(hunks: &[Hunk]) -> Vec<u32> {
    let mut padded = 0;
    hunks
        .iter()
        .map(|h| {
            let start = h.old_rows.start + padded;
            padded += (h.new_rows.len() as u32).saturating_sub(h.old_rows.len() as u32);
            start
        })
        .collect()
}

pub fn marks(hunks: &[Hunk]) -> Vec<Mark> {
    let mut out = Vec::new();
    for (h, start) in hunks.iter().zip(aligned_starts(hunks)) {
        let (o, n) = (h.old_rows.len() as u32, h.new_rows.len() as u32);
        if o > 0 {
            out.push(Mark { lane: Lane::Removed, rows: start..start + o });
        }
        if n > 0 {
            out.push(Mark { lane: Lane::Added, rows: start..start + n });
        }
    }
    out
}

/// `rows` as the top and height of a ruler `height` px tall over `total` rows, at least [`MIN_MARK`] high.
pub fn span(rows: Range<f64>, total: u32, height: f32) -> (f32, f32) {
    let per_row = height / total.max(1) as f32;
    let top = (rows.start as f32 * per_row).clamp(0., height);
    let bottom = (rows.end as f32 * per_row).clamp(top, height);
    let tall = (bottom - top).max(MIN_MARK);
    (top.min(height - tall).max(0.), tall)
}

/// The scroll row that centres the view on `fraction` of the ruler, held inside the content.
pub fn scroll_row_at(fraction: f32, visible: f64, total: u32) -> f64 {
    let most = (total as f64 - visible).max(0.);
    (fraction.clamp(0., 1.) as f64 * total as f64 - visible / 2.).clamp(0., most)
}

/// What the ruler paints: its marks, the rows in view, and its colours.
pub struct Ruler {
    pub marks: Vec<Mark>,
    pub total: u32,
    pub view: Range<f64>,
    pub removed: Hsla,
    pub added: Hsla,
    pub view_fill: Hsla,
}

impl Ruler {
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window, _: &mut App) {
        let (h, w) = (f32::from(bounds.size.height), f32::from(bounds.size.width));
        let lane = (w - 2.) / 2.;
        let (top, tall) = span(self.view.clone(), self.total, h);
        window.paint_quad(fill(Bounds::new(bounds.origin + point(px(0.), px(top)), size(bounds.size.width, px(tall))), self.view_fill));
        for m in &self.marks {
            let (top, tall) = span(m.rows.start as f64..m.rows.end as f64, self.total, h);
            let (x, color) = match m.lane {
                Lane::Removed => (1., self.removed),
                Lane::Added => (1. + lane, self.added),
            };
            window.paint_quad(fill(Bounds::new(bounds.origin + point(px(x), px(top)), size(px(lane), px(tall))), color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(old: Range<u32>, new: Range<u32>) -> Hunk {
        Hunk { old_rows: old, new_rows: new, old_words: Vec::new(), new_words: Vec::new() }
    }

    #[test]
    fn marks_sit_on_the_aligned_rows_in_their_lanes() {
        let hunks = [hunk(1..2, 1..2), hunk(3..5, 3..3), hunk(6..6, 4..7)];
        assert_eq!(aligned_starts(&hunks), [1, 3, 6]);
        let m = |lane, rows| Mark { lane, rows };
        assert_eq!(marks(&hunks), [m(Lane::Removed, 1..2), m(Lane::Added, 1..2), m(Lane::Removed, 3..5), m(Lane::Added, 6..9)]);
    }

    #[test]
    fn spans_scale_rows_to_the_ruler_and_keep_a_least_height() {
        assert_eq!(span(10.0..20.0, 100, 200.), (20., 20.));
        assert_eq!(span(50.0..50.5, 1000, 100.), (5., MIN_MARK));
        assert_eq!(span(999.0..1000.0, 1000, 100.), (100. - MIN_MARK, MIN_MARK));
    }

    #[test]
    fn a_click_centres_the_view_inside_the_content() {
        assert_eq!(scroll_row_at(0.5, 20., 200), 90.);
        assert_eq!(scroll_row_at(0.0, 20., 200), 0.);
        assert_eq!(scroll_row_at(1.0, 20., 200), 180.);
        assert_eq!(scroll_row_at(0.5, 300., 200), 0.);
    }
}

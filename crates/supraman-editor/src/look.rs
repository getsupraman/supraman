//! The diff's colours from the theme, and the striped filler that stands for the other side's lines.

use gpui::{App, Bounds, Hsla, IntoElement, PathBuilder, Pixels, Window, canvas, div, point, prelude::*, px};
use theme::{ActiveTheme as _, Theme};

/// A changed line's background: the status colour at this alpha.
pub const LINE_ALPHA: f32 = 0.25;
/// A changed word's background, painted over its line's: on a dark theme light text holds 4.5:1 on it,
/// on a light one it still stands out of a pale line.
pub const WORD_ALPHA_DARK: f32 = 0.2;
pub const WORD_ALPHA_LIGHT: f32 = 0.4;
/// The filler's stripes: the theme's text at this alpha.
pub const STRIPE_ALPHA: f32 = 0.16;
/// The stripes' horizontal period: two 1 px bands per 8 px tile.
pub const STRIPE_STEP: f32 = 4.;

/// One side's backgrounds: its lines', and its words' laid over them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub line: Hsla,
    pub word: Hsla,
}

/// The removed side's tint, the added side's, and the filler's stripes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffColors {
    pub removed: Tint,
    pub added: Tint,
    pub stripes: Hsla,
}

pub fn diff_colors(theme: &Theme) -> DiffColors {
    let status = theme.status();
    let word = if theme.appearance.is_light() { WORD_ALPHA_LIGHT } else { WORD_ALPHA_DARK };
    let tint = |c: Hsla| Tint { line: c.opacity(LINE_ALPHA), word: c.opacity(word) };
    DiffColors { removed: tint(status.deleted), added: tint(status.created), stripes: theme.colors().text.opacity(STRIPE_ALPHA) }
}

/// The stripes of a `width` × `height` block as line segments rising to the right at 45°, one every
/// [`STRIPE_STEP`] px along the bottom edge, clipped by the caller to the block.
pub fn stripes(width: f32, height: f32) -> Vec<((f32, f32), (f32, f32))> {
    let count = ((width + height) / STRIPE_STEP).ceil() as usize;
    (0..=count).map(|i| -height + i as f32 * STRIPE_STEP).map(|x| ((x, height), (x + height, 0.))).collect()
}

fn paint_stripes(bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let color = diff_colors(cx.theme()).stripes;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let mut path = PathBuilder::stroke(px(1.));
    for ((x0, y0), (x1, y1)) in stripes(w, h) {
        path.move_to(bounds.origin + point(px(x0), px(y0)));
        path.line_to(bounds.origin + point(px(x1), px(y1)));
    }
    if let Ok(path) = path.build() {
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| window.paint_path(path, color));
    }
}

/// A filler `height` tall, striped.
pub fn filler(height: Pixels) -> impl IntoElement {
    div().w_full().h(height).child(canvas(|_, _, _| (), |bounds, _, window, cx| paint_stripes(bounds, window, cx)).size_full())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stripes_rise_at_45_degrees_every_four_pixels_across_the_block() {
        let s = stripes(16., 8.);
        assert!(s.iter().all(|((x0, y0), (x1, y1))| x1 - x0 == y0 - y1 && *y0 == 8. && *y1 == 0.));
        assert!(s.windows(2).all(|w| w[1].0.0 - w[0].0.0 == STRIPE_STEP));
        assert!(s.first().unwrap().1.0 <= 0. && s.last().unwrap().0.0 >= 16. - STRIPE_STEP, "the stripes cover the whole block: {s:?}");
    }
}

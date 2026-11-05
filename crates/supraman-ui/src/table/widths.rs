//! Column widths that always fill the table: each column has its current width, a min, a preferred width (where
//! the user last dragged it) and maybe a max. A drag refits only the columns right of its divider; a change of
//! the table's width takes from the rightmost columns first and gives back to those below their preferred
//! width, left to right, the rest to the last column. Only when every column is at its min does the sum pass
//! the table's width.

use std::ops::Range;

use gpui::{Pixels, px};

use super::Column;

fn sum(widths: &[Pixels]) -> Pixels {
    widths.iter().fold(px(0.), |s, w| s + *w)
}

/// The last column has no max: it takes what the others leave.
fn bounded(columns: &[Column], i: usize, width: Pixels) -> Pixels {
    let width = width.max(columns[i].min);
    match columns[i].max {
        Some(max) if i + 1 < columns.len() => width.min(max.max(columns[i].min)),
        _ => width,
    }
}

/// Each preferred width held within its column's min and max; a missing one is the column's default.
pub fn clamp(columns: &[Column], preferred: &[Pixels]) -> Vec<Pixels> {
    columns.iter().enumerate().map(|(i, c)| bounded(columns, i, preferred.get(i).copied().unwrap_or(c.default))).collect()
}

/// The widths in a table `total` wide, from the last frame's `current` ones (none: the preferred ones).
pub fn fit(columns: &[Column], current: &[Pixels], preferred: &[Pixels], total: Pixels) -> Vec<Pixels> {
    let mut widths = if current.len() == columns.len() {
        current.iter().enumerate().map(|(i, w)| bounded(columns, i, *w)).collect()
    } else {
        clamp(columns, preferred)
    };
    let gap = total - sum(&widths);
    if gap < px(0.) {
        take(columns, &mut widths, 0..columns.len(), -gap);
    } else {
        give(&mut widths, &clamp(columns, preferred), 0, gap);
    }
    widths
}

/// The widths after the divider right of column `divider` is dragged to `x` (from the table's left edge): that
/// column follows, the columns left of it stay. Growing takes from the columns right of it, the rightmost first,
/// down to their mins, and grows no further than they give; shrinking gives back as [`fit`] does.
pub fn drag(columns: &[Column], current: &[Pixels], preferred: &[Pixels], divider: usize, x: Pixels) -> Vec<Pixels> {
    let mut widths = current.to_vec();
    let n = columns.len();
    if divider + 1 >= n || widths.len() != n {
        return widths;
    }
    let wanted = bounded(columns, divider, x - sum(&widths[..divider]));
    let delta = wanted - widths[divider];
    if delta > px(0.) {
        let got = take(columns, &mut widths, divider + 1..n, delta);
        widths[divider] += got;
    } else if delta < px(0.) {
        widths[divider] = wanted;
        give(&mut widths, &clamp(columns, preferred), divider + 1, -delta);
    }
    widths
}

/// Takes up to `want` from the columns in `range`, the rightmost first, each down to its min; what it took.
fn take(columns: &[Column], widths: &mut [Pixels], range: Range<usize>, want: Pixels) -> Pixels {
    let mut taken = px(0.);
    for i in range.rev() {
        let part = (widths[i] - columns[i].min).max(px(0.)).min(want - taken);
        widths[i] -= part;
        taken += part;
    }
    taken
}

/// Gives `spare` to the columns from `from` on that are below their `preferred` width, left to right, up to it;
/// the rest to the last column.
fn give(widths: &mut [Pixels], preferred: &[Pixels], from: usize, mut spare: Pixels) {
    for i in from..widths.len() {
        let part = (preferred[i] - widths[i]).max(px(0.)).min(spare);
        widths[i] += part;
        spare -= part;
    }
    if let Some(last) = widths.last_mut() {
        *last += spare;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<Column> {
        vec![
            Column::new("ref", "Branch / Tag", px(32.), px(130.)),
            Column::new("graph", "Graph", px(54.), px(150.)).max(px(200.)),
            Column::new("message", "Commit Message", px(50.), px(300.)),
            Column::new("date", "Commit Date / Time", px(50.), px(170.)),
            Column::new("sha", "Sha", px(50.), px(130.)),
        ]
    }

    fn px_all(w: &[f32]) -> Vec<Pixels> {
        w.iter().map(|w| px(*w)).collect()
    }

    fn mins(columns: &[Column]) -> Pixels {
        columns.iter().fold(px(0.), |s, c| s + c.min)
    }

    #[test]
    fn a_fresh_table_takes_the_preferred_widths_and_gives_the_rest_to_the_last_column() {
        let c = columns();
        let preferred = px_all(&[130., 150., 300., 170., 130.]);
        assert_eq!(fit(&c, &[], &preferred, px(1000.)), px_all(&[130., 150., 300., 170., 250.]));
        assert_eq!(fit(&c, &[], &[], px(880.)), preferred, "no preferred widths: the defaults");
    }

    #[test]
    fn growing_takes_from_the_rightmost_column_first_and_is_capped_by_what_the_right_side_gives() {
        let c = columns();
        let preferred = px_all(&[130., 150., 300., 170., 130.]);
        let current = fit(&c, &[], &preferred, px(880.));
        let w = drag(&c, &current, &preferred, 2, px(130. + 150. + 380.));
        assert_eq!(w, px_all(&[130., 150., 380., 170., 50.]), "the sha gives its 80 px down to its min");
        let w = drag(&c, &current, &preferred, 2, px(130. + 150. + 450.));
        assert_eq!(w, px_all(&[130., 150., 450., 100., 50.]), "then the date");
        let w = drag(&c, &current, &preferred, 2, px(5000.));
        assert_eq!(w, px_all(&[130., 150., 500., 50., 50.]), "no further than both mins allow");
        let w = drag(&c, &current, &preferred, 0, px(5000.));
        assert_eq!(w, px_all(&[676., 54., 50., 50., 50.]), "a first column takes the whole right side");
    }

    #[test]
    fn shrinking_gives_back_to_columns_below_their_preferred_width_then_to_the_last() {
        let c = columns();
        let preferred = px_all(&[130., 150., 300., 170., 130.]);
        let grown = px_all(&[130., 150., 450., 100., 50.]);
        let w = drag(&c, &grown, &preferred, 2, px(130. + 150. + 360.));
        assert_eq!(w, px_all(&[130., 150., 360., 170., 70.]), "the date back to its preferred first, left to right");
        let w = drag(&c, &grown, &preferred, 2, px(130. + 150. + 200.));
        assert_eq!(w, px_all(&[130., 150., 200., 170., 230.]), "the rest to the last column, past its preferred");
        let w = drag(&c, &grown, &preferred, 0, px(80.));
        assert_eq!(w, px_all(&[80., 150., 450., 150., 50.]), "the graph is at its preferred; the date takes the 50 px");
    }

    #[test]
    fn columns_left_of_the_divider_never_change_and_the_graph_stops_at_its_max() {
        let c = columns();
        let preferred = px_all(&[130., 300., 300., 170., 130.]);
        let current = px_all(&[130., 100., 300., 170., 180.]);
        let w = drag(&c, &current, &preferred, 0, px(60.));
        assert_eq!(w, px_all(&[60., 170., 300., 170., 180.]), "the graph grows back only up to its max of 200");
        let w = drag(&c, &current, &preferred, 1, px(130. + 400.));
        assert_eq!(w[..2], px_all(&[130., 200.]), "dragged past its max, the graph stops there");
        let w = drag(&c, &current, &preferred, 3, px(800.));
        assert_eq!(w[..3], current[..3], "left of the divider: unchanged");
    }

    #[test]
    fn the_last_column_cannot_be_dragged_narrower_than_the_others_leave_it() {
        let c = columns();
        let current = px_all(&[130., 150., 300., 170., 130.]);
        assert_eq!(drag(&c, &current, &current, 4, px(10.)), current, "the last column has no divider of its own");
        let w = drag(&c, &current, &current, 3, px(130. + 150. + 300. + 100.));
        assert_eq!(w[4], px(200.), "a narrower date gives its 70 px to the last column");
    }

    #[test]
    fn a_narrower_table_takes_from_the_right_and_a_wider_one_gives_back_left_to_right() {
        let c = columns();
        let preferred = px_all(&[130., 150., 300., 170., 130.]);
        let current = fit(&c, &[], &preferred, px(880.));
        let narrow = fit(&c, &current, &preferred, px(700.));
        assert_eq!(narrow, px_all(&[130., 150., 300., 70., 50.]));
        let narrower = fit(&c, &narrow, &preferred, px(400.));
        assert_eq!(narrower, px_all(&[130., 120., 50., 50., 50.]), "then the message, then the graph");
        let wider = fit(&c, &narrower, &preferred, px(800.));
        assert_eq!(wider, px_all(&[130., 150., 300., 170., 50.]), "below preferred grows back left to right");
        let widest = fit(&c, &wider, &preferred, px(1000.));
        assert_eq!(widest, px_all(&[130., 150., 300., 170., 250.]), "the rest to the last column");
        let clipped = fit(&c, &narrower, &preferred, px(100.));
        assert_eq!(clipped, c.iter().map(|c| c.min).collect::<Vec<_>>(), "at the mins the table clips");
    }

    #[test]
    fn preferred_widths_are_clamped_to_min_and_max_on_load() {
        let c = columns();
        assert_eq!(clamp(&c, &px_all(&[1., 900., 10., 170.])), px_all(&[32., 200., 50., 170., 130.]));
        let mut last_graph = columns();
        last_graph.truncate(2);
        assert_eq!(clamp(&last_graph, &px_all(&[130., 900.])), px_all(&[130., 900.]), "the last column has no max");
    }

    #[test]
    fn the_widths_always_fill_the_table_over_random_drags_and_resizes() {
        let c = columns();
        let mut seed = 0x2545_f491_u64;
        let mut next = |below: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        };
        let mut preferred = px_all(&[130., 150., 300., 170., 130.]);
        let mut total = px(900.);
        let mut widths = fit(&c, &[], &preferred, total);
        for step in 0..5_000 {
            if next(5) == 0 {
                total = px(next(1400) as f32 + f32::from(mins(&c)));
                widths = fit(&c, &widths, &preferred, total);
            } else {
                let divider = next(4) as usize;
                let before = widths.clone();
                widths = drag(&c, &widths, &preferred, divider, px(next(1600) as f32));
                assert_eq!(widths[..divider], before[..divider], "step {step}: left of divider {divider} changed");
                preferred[divider] = widths[divider];
            }
            let s = widths.iter().fold(px(0.), |s, w| s + *w);
            assert!((s - total).abs() < px(0.01), "step {step}: {widths:?} sums to {s:?}, not {total:?}");
            assert!(c.iter().zip(&widths).all(|(c, w)| *w >= c.min), "step {step}: under a min {widths:?}");
            assert!(widths[1] <= px(200.), "step {step}: the graph past its max {widths:?}");
        }
    }
}

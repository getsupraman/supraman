//! A state's mark in a list or a strip: a dot, a strip of one square per item, and a thin progress line.
//! The owner maps its states to [`Mark`]s on the theme's status colours.

use gpui::{Div, Hsla, div, prelude::*, px};

/// A strip over this many items is a plain bar.
pub const SQUARES_MAX: usize = 40;
const SQUARE: f32 = 11.;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    Filled(Hsla),
    /// A hollow ring: waiting on something else.
    Ring(Hsla),
}

impl Mark {
    pub fn color(self) -> Hsla {
        match self {
            Mark::Filled(c) | Mark::Ring(c) => c,
        }
    }
}

fn shape(mark: Mark, side: f32) -> Div {
    let base = div().flex_none().size(px(side));
    match mark {
        Mark::Filled(c) => base.bg(c),
        Mark::Ring(c) => base.border_color(c).border(px(1.5)),
    }
}

pub fn dot(mark: Mark, side: f32) -> Div {
    shape(mark, side).rounded_full()
}

/// One rounded square per mark, in order, wrapping; over [`SQUARES_MAX`] marks a bar of runs of equal marks.
pub fn squares(marks: &[Mark]) -> Div {
    if marks.len() <= SQUARES_MAX {
        return div().flex().flex_wrap().gap(px(3.)).children(marks.iter().map(|m| shape(*m, SQUARE).rounded_sm()));
    }
    let mut runs: Vec<(Mark, usize)> = Vec::new();
    for m in marks {
        match runs.last_mut() {
            Some((last, n)) if last == m => *n += 1,
            _ => runs.push((*m, 1)),
        }
    }
    let segments = runs.into_iter().map(|(mark, n)| {
        let fill = match mark {
            Mark::Filled(c) => c,
            Mark::Ring(c) => c.opacity(0.35),
        };
        div().h_full().flex_grow(n as f32).flex_basis(px(0.)).bg(fill)
    });
    div().flex().w_full().h(px(8.)).rounded_sm().overflow_hidden().gap(px(1.)).children(segments.collect::<Vec<_>>())
}

/// A 2 px line: each `(count, colour)` part's share of `total` from the left, the rest `track`.
pub fn progress_line(parts: &[(u32, Hsla)], total: u32, track: Hsla) -> Div {
    let total = total.max(parts.iter().map(|(n, _)| *n).sum()).max(1) as f32;
    let shown: u32 = parts.iter().map(|(n, _)| *n).sum();
    let rest = total - shown as f32;
    let line = div().flex().w_full().h(px(2.)).rounded_full().overflow_hidden().bg(track);
    let line = line.children(parts.iter().filter(|(n, _)| *n > 0).map(|(n, c)| div().h_full().flex_grow(*n as f32).flex_basis(px(0.)).bg(*c)));
    line.when(rest > 0., |d| d.child(div().h_full().flex_grow(rest).flex_basis(px(0.))))
}

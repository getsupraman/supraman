//! The Graph column: a row's lanes and edges painted as paths, its node as a circle with the author's initials.
//! Geometry per docs/research/gitkraken-12.6.1.md §3.

use gpui::{AnyElement, Bounds, Div, FontWeight, Hsla, SharedString, PathBuilder, Pixels, Window, canvas, div, point, prelude::*, px};
use supraman_gitman::{Edge, EdgeKind};
use supraman_ui::fonts;

use super::colours::Palette;
use super::load::GraphRow;

pub const ROW: f32 = 28.;
const LANE: f32 = 22.;
const PAD: f32 = 6.;
const RADIUS: f32 = 11.;
const NODE: f32 = 20.;
const MERGE: f32 = 10.;
const STROKE: f32 = 1.5;
/// The table pads each cell by this much; the graph paints from the column's edge.
pub const CELL_PAD: f32 = 8.;

/// The middle of `lane` from the column's left edge.
pub fn lane_x(lane: usize) -> f32 {
    PAD + lane as f32 * LANE + LANE / 2.
}

pub fn cell(row: &GraphRow, connector: bool, palette: &Palette) -> AnyElement {
    let edges: Vec<(Edge, Hsla)> = row.row.edges.iter().map(|e| (*e, palette.lane(e.colour))).collect();
    let (lane, colour) = (row.row.lane, row.row.colour);
    let link = connector.then(|| palette.connector(colour));
    let paint = canvas(|_, _, _| {}, move |bounds, _, window, _| {
        if let Some(link) = link {
            line(window, bounds, link, false, |b, mid| {
                b.move_to(point(bounds.left(), mid));
                b.line_to(point(bounds.left() + px(lane_x(lane)), mid));
            });
        }
        for (edge, colour) in &edges {
            paint_edge(window, bounds, edge, *colour);
        }
    })
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-CELL_PAD))
        .right(px(-CELL_PAD))
        .child(paint)
        .child(node(row, palette))
        .into_any_element()
}

fn line(window: &mut Window, bounds: Bounds<Pixels>, colour: Hsla, dashed: bool, draw: impl FnOnce(&mut PathBuilder, Pixels)) {
    let mut b = PathBuilder::stroke(px(STROKE));
    if dashed {
        b = b.dash_array(&[px(3.), px(3.)]);
    }
    draw(&mut b, bounds.top() + bounds.size.height / 2.);
    if let Ok(path) = b.build() {
        window.paint_path(path, colour);
    }
}

/// Starting: from the node across to its lane and down; ending: down its lane and across to the node; a
/// quarter curve where it turns.
fn paint_edge(window: &mut Window, bounds: Bounds<Pixels>, e: &Edge, colour: Hsla) {
    let x = |lane: usize| bounds.left() + px(lane_x(lane));
    let (top, bottom, r) = (bounds.top(), bounds.bottom(), px(RADIUS));
    line(window, bounds, colour, e.dashed, |b, mid| match e.kind {
        EdgeKind::PassThrough => {
            b.move_to(point(x(e.lane), top));
            b.line_to(point(x(e.lane), bottom));
        }
        EdgeKind::Starting if e.lane == e.node => {
            b.move_to(point(x(e.lane), mid));
            b.line_to(point(x(e.lane), bottom));
        }
        EdgeKind::Ending if e.lane == e.node => {
            b.move_to(point(x(e.lane), top));
            b.line_to(point(x(e.lane), mid));
        }
        EdgeKind::Starting => {
            let toward = if e.lane > e.node { 1. } else { -1. };
            b.move_to(point(x(e.node), mid));
            b.line_to(point(x(e.lane) - r * toward, mid));
            b.curve_to(point(x(e.lane), mid + r), point(x(e.lane), mid));
            b.line_to(point(x(e.lane), bottom));
        }
        EdgeKind::Ending => {
            let toward = if e.node > e.lane { 1. } else { -1. };
            b.move_to(point(x(e.lane), top));
            b.line_to(point(x(e.lane), mid - r));
            b.curve_to(point(x(e.lane) + r * toward, mid), point(x(e.lane), mid));
            b.line_to(point(x(e.node), mid));
        }
    });
}

/// A commit's circle with its author's initials, ringed in its lane; a merge's smaller dot; the WIP row's
/// dashed hollow ring.
fn node(row: &GraphRow, palette: &Palette) -> AnyElement {
    let colour = row.row.colour;
    let lane = palette.lane(colour);
    let size = if row.merge { MERGE } else { NODE };
    let at = div()
        .absolute()
        .left(px(lane_x(row.row.lane) - size / 2.))
        .top(px((ROW - size) / 2.))
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center();
    if row.wip() {
        return at.border_2().border_dashed().border_color(lane).bg(palette.background).into_any_element();
    }
    if row.merge {
        return at.bg(lane).into_any_element();
    }
    at.child(avatar(row.initials.clone(), colour, NODE, palette))
        .into_any_element()
}

/// An author's `side` px circle of `initials`, ringed in lane `colour`: the graph's node and the commit panel's avatars.
pub fn avatar(initials: SharedString, colour: usize, side: f32, palette: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(side))
        .rounded_full()
        .border_2()
        .border_color(palette.lane(colour))
        .bg(palette.avatar(colour))
        .text_color(palette.initials(colour))
        .text_size(px(side * 0.425))
        .font_family(fonts::UI)
        .font_weight(FontWeight::SEMIBOLD)
        .line_height(px(side / 2.))
        .child(initials)
}

//! The Graph column's lanes, edges and nodes; geometry per docs/research/gitkraken-12.6.1.md §3.

use gpui::{AnyElement, Bounds, Div, FontWeight, Hsla, SharedString, PathBuilder, Pixels, Window, canvas, div, fill, point, prelude::*, px};
use supraman_gitman::{Edge, EdgeKind};
use supraman_ui::fonts;
use supraman_ui::hit::{Hit, HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;

use super::colours::Palette;
use super::load::GraphRow;

pub(crate) const ROW: f32 = 28.;
const LANE: f32 = 22.;
/// The column's room left of the first lane and right of the last.
const PAD: f32 = 8.;
/// The space between two rows' bands; every column's band and fill is inset half of it from the row's top and bottom.
pub(crate) const GAP: f32 = 6.;
/// Every column's line box: the band's height, so each column's text centres on the same line.
pub(crate) const LINE: f32 = ROW - GAP;
const RADIUS: f32 = 11.;
const NODE: f32 = 20.;
const MERGE: f32 = 10.;
const STROKE: f32 = 1.5;
/// The table pads each cell by this much; the graph paints from the column's edge.
pub(crate) const CELL_PAD: f32 = 8.;
/// The lane-coloured bar that closes a row's band at the message column's left edge.
pub(crate) const BAR: f32 = 3.;
/// The narrowest the graph column goes: a node, 3 px either side and the gutter.
pub(crate) const MIN_WIDTH: f32 = NODE + 2. * 3. + 28.;
/// The widest the graph column grows by itself to hold its lanes.
pub(crate) const MAX_FIT: f32 = 400.;

/// The middle of `lane` from the column's left edge.
pub(crate) fn lane_x(lane: usize) -> f32 {
    PAD + lane as f32 * LANE + LANE / 2.
}

/// The column width that holds `lanes` lanes with the padding on both sides.
pub(crate) fn width_for(lanes: usize) -> f32 {
    2. * PAD + lanes as f32 * LANE
}

pub(crate) fn lanes(rows: &[GraphRow]) -> usize {
    rows.iter().map(|r| r.row.max_lane).max().map_or(0, |l| l + 1)
}

/// A row's band in a graph cell: from its node's lane centre to the column's right edge, `GAP / 2` short top and bottom.
pub(crate) fn band(lane: usize, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    let left = (bounds.left() + px(lane_x(lane))).min(bounds.right());
    let top = bounds.top() + px(GAP / 2.);
    Bounds::from_corners(point(left, top), point(bounds.right(), (bounds.bottom() - px(GAP / 2.)).max(top)))
}

/// The column's whole width, its padding too, painted under `git-graph-<ix>`.
pub(crate) fn cell(ix: usize, row: &GraphRow, connector: bool, palette: &Palette, hits: &HitTargets) -> AnyElement {
    let edges: Vec<(Edge, Hsla)> = row.row.edges().map(|e| (e, palette.lane(e.colour))).collect();
    let (lane, colour) = (row.row.lane, row.row.colour);
    let link = connector.then(|| palette.connector(colour));
    let shade = palette.band(colour);
    let hits_link = hits.clone();
    let paint = canvas(|_, _, _| {}, move |bounds, _, window, _| {
        window.paint_quad(fill(band(lane, bounds), shade));
        if let Some(link) = link {
            link_line(window, bounds.left(), bounds.left() + px(lane_x(lane)), bounds, link, &hits_link, format!("git-graph-link-{ix}"));
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
        .child(hit_target(hits, format!("git-graph-{ix}")))
        .child(node(row, palette))
        .into_any_element()
}

/// [`STROKE`] in whole device pixels: every graph line's width and every connector's.
fn device_stroke(scale: f32) -> f32 {
    (STROKE * scale).round().max(1.)
}

pub(crate) fn stroke_width(scale: f32) -> Pixels {
    px(device_stroke(scale) / scale)
}

/// `at` moved to where a [`stroke_width`] line centred on it covers whole device pixels.
pub(crate) fn snap(at: Pixels, scale: f32) -> Pixels {
    let device = f32::from(at) * scale;
    let centre = if device_stroke(scale) % 2. == 1. { device.floor() + 0.5 } else { device.round() };
    px(centre / scale)
}

/// The horizontal line from `left` to `right` at `bounds`' snapped middle, recorded under `key`: a pill's connector and its graph half.
pub(crate) fn link_line(window: &mut Window, left: Pixels, right: Pixels, bounds: Bounds<Pixels>, colour: Hsla, hits: &HitTargets, key: String) {
    let scale = window.scale_factor();
    let (mid, half) = (snap(bounds.center().y, scale), stroke_width(scale) / 2.);
    let line = Bounds::from_corners(point(left, mid - half), point(right.max(left), mid + half));
    window.paint_quad(fill(line, colour));
    hits.borrow_mut().insert(key, Hit { bounds: line, visible: window.content_mask().bounds });
}

fn line(window: &mut Window, bounds: Bounds<Pixels>, colour: Hsla, dashed: bool, draw: impl FnOnce(&mut PathBuilder, Pixels)) {
    let scale = window.scale_factor();
    let mut b = PathBuilder::stroke(stroke_width(scale));
    if dashed {
        b = b.dash_array(&[px(3.), px(3.)]);
    }
    draw(&mut b, snap(bounds.center().y, scale));
    if let Ok(path) = b.build() {
        window.paint_path(path, colour);
    }
}

fn paint_edge(window: &mut Window, bounds: Bounds<Pixels>, e: &Edge, colour: Hsla) {
    let scale = window.scale_factor();
    let x = |lane: usize| snap(bounds.left() + px(lane_x(lane)), scale);
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

/// A commit's initials ringed in its lane, a merge's dot, a WIP's dashed ring, a stash's dashed square, a session's bot.
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
    if row.agent.is_some() {
        return at.border_1().border_dashed().border_color(lane).bg(palette.background).child(Icon::Bot.svg(13., lane)).into_any_element();
    }
    if row.wip() || row.worktree().is_some() {
        return at.border_2().border_dashed().border_color(lane).bg(palette.background).into_any_element();
    }
    if row.stash().is_some() {
        return at
            .id(SharedString::from(format!("git-stash-{}", row.sha)))
            .border_1()
            .border_dashed()
            .rounded_sm()
            .border_color(lane)
            .bg(palette.background)
            .child(Icon::Stash.svg(13., lane))
            .tooltip(Tooltip::text("A stash: work set aside"))
            .into_any_element();
    }
    if row.merge {
        return at.bg(lane).into_any_element();
    }
    at.child(avatar(row.initials.clone(), colour, NODE, palette))
        .into_any_element()
}

/// An author's `side` px circle of `initials`, ringed in lane `colour`: the graph's node and the commit panel's avatars.
pub(crate) fn avatar(initials: SharedString, colour: usize, side: f32, palette: &Palette) -> Div {
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

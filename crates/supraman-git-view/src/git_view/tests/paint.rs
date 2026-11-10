//! Where the graph's rows paint: the band's edges, one inset and line box across the columns, a menu over a pill list.

use gpui::{Bounds, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, TestAppContext, point};
use supraman_gitman::fixture::Fixture;

use crate::git_view::test_support::{Opts, open, repaint};
use crate::git_view::*;

fn at(hits: &HitTargets, key: &str) -> Bounds<Pixels> {
    hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds
}

fn near(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < px(0.5)
}

#[gpui::test]
fn graph_band_runs_from_the_node_to_the_next_column_and_fills_span_every_cell(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let (selected, lane) = view.update(cx, |v, cx| {
        let (ix, row) = v.pages.read(cx).rows.iter().enumerate().find(|(_, r)| r.sha.as_ref() == f.sha("second")).expect("second's row");
        let lane = row.row.lane;
        v.select_sha(f.sha("second"), cx);
        (ix, lane)
    });
    let hits = repaint(&view, cx);
    let hovered = selected + 1;
    cx.simulate_mouse_move(at(&hits, &format!("git-row-{hovered}")).center(), None, Modifiers::none());
    let hits = repaint(&view, cx);
    let (refs, graph, message, sha) = (at(&hits, "git-col-refs"), at(&hits, "git-col-graph"), at(&hits, "git-col-message"), at(&hits, "git-col-sha"));
    let layer = at(&hits, &format!("git-graph-{selected}"));
    assert!(near(layer.left(), graph.left()) && near(layer.right(), message.left()), "the graph paints its whole column: {layer:?} {graph:?} {message:?}");
    let band = graph::band(lane, layer);
    assert!(near(band.left(), graph.left() + px(graph::lane_x(lane))), "the band starts at the node's centre: {band:?}");
    assert!(near(band.right(), message.left()), "and meets the message column: {band:?} {message:?}");
    for ix in [selected, hovered] {
        let fill = at(&hits, &format!("git-fill-{ix}"));
        assert!(near(fill.left(), refs.left()) && near(fill.right(), sha.right()), "row {ix}'s fill spans every cell: {fill:?}");
    }
}

#[gpui::test]
fn every_column_shares_one_inset_and_one_line_box(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let (ix, lane) = view.read_with(cx, |v, cx| {
        let (ix, row) = v.pages.read(cx).rows.iter().enumerate().find(|(_, r)| r.sha.as_ref() == f.sha("ahead")).expect("ahead's row");
        (ix, row.row.lane)
    });
    let hits = repaint(&view, cx);
    let row = at(&hits, &format!("git-row-{ix}"));
    let fill = at(&hits, &format!("git-fill-{ix}"));
    let inset = px(graph::GAP / 2.);
    assert!(near(fill.top(), row.top() + inset) && near(fill.bottom(), row.bottom() - inset), "the fill is inset: {fill:?} {row:?}");
    let band = graph::band(lane, at(&hits, &format!("git-graph-{ix}")));
    assert!(near(band.top(), fill.top()) && near(band.bottom(), fill.bottom()), "the band shares it: {band:?} {fill:?}");
    assert!(near(fill.size.height, px(graph::LINE)), "the line box is the band");
    for key in ["text", "date", "sha"] {
        let line = at(&hits, &format!("git-{key}-{ix}"));
        assert!(near(line.top(), fill.top()) && near(line.bottom(), fill.bottom()), "{key}'s line box is the band: {line:?} {fill:?}");
    }
}

#[gpui::test]
fn two_rows_bands_are_inset_3_px_and_6_px_apart(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let ix = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.sha.as_ref() == f.sha("ahead")).expect("ahead's row"));
    let hits = repaint(&view, cx);
    let (row, fill, next) = (at(&hits, &format!("git-row-{ix}")), at(&hits, &format!("git-fill-{ix}")), at(&hits, &format!("git-fill-{}", ix + 1)));
    assert!(near(fill.top() - row.top(), px(3.)) && near(row.bottom() - fill.bottom(), px(3.)), "GitKraken's inset: {fill:?} {row:?}");
    assert!(near(next.top() - fill.bottom(), px(6.)), "the gap between two bands: {fill:?} {next:?}");
}

#[gpui::test]
fn a_menu_paints_over_an_open_pill_group(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let ix = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.sha.as_ref() == f.sha("second")).expect("second's row"));
    let hits = repaint(&view, cx);
    let cell = at(&hits, &format!("git-refs-{ix}"));
    cx.simulate_mouse_move(point(cell.right() - px(3.), cell.center().y), None, Modifiers::none());
    let hits = repaint(&view, cx);
    let v1 = at(&hits, &format!("git-reflist-{ix}-1"));
    view.update(cx, |v, cx| v.pill_menu(ix, 0, v1.origin + point(px(2.), px(2.)), cx));
    let hits = repaint(&view, cx);
    let (menu, v1) = (at(&hits, "menu"), at(&hits, &format!("git-reflist-{ix}-1")));
    let overlap = menu.intersect(&v1);
    assert!(overlap.size.width > px(4.) && overlap.size.height > px(4.), "the menu lies over the open list: {menu:?} {v1:?}");
    let (position, button, modifiers) = (overlap.center(), MouseButton::Right, Modifiers::none());
    cx.simulate_event(MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position, button, modifiers, click_count: 1 });
    cx.run_until_parked();
    let hits = repaint(&view, cx);
    assert_eq!(hits.borrow().get("menu").map(|h| h.bounds), Some(menu), "the menu took the click; the list's pill under it never saw it");
}

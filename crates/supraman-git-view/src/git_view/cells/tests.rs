//! The ref pills on the graph: their band, their hover fill, the `+N` chip, one line of letters, and narrow columns.

use gpui::{Bounds, Entity, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext, div, prelude::*, px, rems};
use supraman_gitman::fixture::Fixture;
use supraman_ui::clipped::cut_at;
use supraman_ui::hit::HitTargets;

use super::{figure_text, pill_fill};
use crate::git_view::background::refresh_wips;
use crate::git_view::colours::Palette;
use crate::git_view::graph::{GAP, LINE};
use crate::git_view::test_support::{self, Opts, redraw, repaint, row_of};
use crate::git_view::GitView;

/// The fixture's tab with a change in its linked worktree `wt`, so that worktree has a row.
fn open<'a>(f: &Fixture, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().background().size(1400., 900.), cx);
    let id = view.entity_id();
    cx.update(|_, cx| refresh_wips(id, cx));
    cx.run_until_parked();
    redraw(&view, cx);
    (view, cx)
}

fn at(hits: &HitTargets, key: &str) -> Bounds<Pixels> {
    hits.borrow().get(key).map(|h| h.bounds).unwrap_or_else(|| panic!("{key} is painted"))
}

fn worktree_row(view: &Entity<GitView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.worktree().is_some())).expect("the worktree's row")
}

#[gpui::test]
fn a_pill_fills_the_band_its_letters_and_icons_level_and_its_more_chip_a_gap_away(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f, cx);
    let ix = row_of(&view, f.sha("second"), cx);
    let hits = repaint(&view, cx);
    let (pill, fill, more) = (at(&hits, &format!("git-pill-{ix}")), at(&hits, &format!("git-fill-{ix}")), at(&hits, &format!("git-more-{ix}")));
    assert_eq!((pill.top(), pill.size.height), (fill.top(), fill.size.height), "the band's height, as the selection fill: {pill:?} {fill:?}");
    assert_eq!(pill.size.height, px(LINE));
    assert_eq!(more.left() - pill.right(), px(GAP), "+N stands the stripes' gap from its pill: {pill:?} {more:?}");
    assert_eq!(more.size.height, px(LINE), "+N a chip of its own");

    let below = cx.update(|window, _| {
        let mut style = window.text_style();
        style.font_size = rems(0.75).into();
        supraman_ui::ink::offset(&style, window)
    });
    let text = at(&hits, &format!("git-ref-{ix}-text")).center().y + below;
    let icon = at(&hits, &format!("git-ref-{ix}-icon-0"));
    assert_eq!(icon.size.height, px(super::PILL_ICON));
    assert!(below > px(0.) && supraman_ui::ink::level(text, icon.center().y) && supraman_ui::ink::level(text, pill.center().y), "letters, icons and the pill on one line: {text:?} {icon:?} {pill:?}");

    let w = worktree_row(&view, cx);
    let (label, band) = (at(&hits, &format!("git-worktree-ref-{w}")), at(&hits, &format!("git-fill-{w}")));
    assert_eq!((label.top(), label.size.height), (band.top(), band.size.height), "the worktree row's pill too");
    let tree = at(&hits, &format!("git-worktree-label-{w}-icon-1"));
    assert!(supraman_ui::ink::level(at(&hits, &format!("git-worktree-label-{w}-text")).center().y + below, tree.center().y), "its tree icon, last, on its letters' line");
    assert!(tree.right() > at(&hits, &format!("git-worktree-label-{w}-icon-0")).right(), "local, then the tree");
}

#[gpui::test]
fn a_hovered_pill_fills_stronger_in_every_lane(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (_view, cx) = test_support::open(&f.dir, Opts::default(), cx);
    let palette = cx.update(|_, cx| Palette::of(theme::ActiveTheme::theme(cx)));
    for lane in 0..supraman_ui::themes::lanes::LANES {
        let (plain, hovered) = (pill_fill(lane, false, &palette), pill_fill(lane, true, &palette));
        assert!(plain != hovered && hovered.a > plain.a, "lane {lane}: {plain:?} {hovered:?}");
    }
}

/// The Branch / Tag column dragged to `width`, as a hand does; the hits of the frame after.
fn narrow(view: &Entity<GitView>, width: f32, cx: &mut VisualTestContext) -> HitTargets {
    let hits = repaint(view, cx);
    let divider = at(&hits, "git-divider-refs").center();
    let to = divider - gpui::point(at(&hits, "git-col-refs").size.width - px(width), px(0.));
    cx.simulate_mouse_down(divider, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(divider - gpui::point(px(10.), px(0.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    repaint(view, cx)
}

/// Each icon of the pill named `name` is painted whole inside `pill`, or not at all.
fn icons_whole(hits: &HitTargets, name: &str, pill: Bounds<Pixels>) {
    for (key, hit) in hits.borrow().iter().filter(|(k, _)| k.starts_with(&format!("{name}-icon-"))) {
        let clip = hit.bounds.intersect(&hit.visible).size;
        let hidden = clip.width <= px(0.) || clip.height <= px(0.);
        assert!(hidden || (hit.fully_visible() && hit.bounds.left() >= pill.left() && hit.bounds.right() <= pill.right()), "{key} painted whole inside its pill or not at all: {hit:?} {pill:?}");
    }
}

#[gpui::test]
fn at_a_narrow_branch_column_every_pill_is_cut_inside_it_and_drops_whole_icons(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f, cx);
    let rows = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().enumerate().filter(|(_, r)| !r.row.refs.is_empty()).map(|(i, _)| i).collect::<Vec<_>>());
    let w = worktree_row(&view, cx);
    let pills: Vec<(String, String)> = rows.iter().map(|ix| (format!("git-pill-{ix}"), format!("git-ref-{ix}"))).chain([(format!("git-worktree-ref-{w}"), format!("git-worktree-label-{w}"))]).collect();
    assert!(rows.len() >= 3, "{rows:?}");
    for width in (64..=160).step_by(3) {
        let hits = narrow(&view, width as f32, cx);
        for (pill, name) in &pills {
            icons_whole(&hits, name, at(&hits, pill));
        }
    }

    let hits = narrow(&view, 80., cx);
    let column = at(&hits, "git-col-refs");
    assert!(column.size.width < px(100.), "{column:?}");
    let head = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.row.refs.first().is_some_and(|g| g.head()))).expect("HEAD's row");
    let shown = |key: &str| hits.borrow().get(key).is_some_and(|h| h.fully_visible());
    assert!(!shown(&format!("git-ref-{head}-icon-0")) && shown(&format!("git-ref-{head}-text")), "HEAD's pill, its ✓ and cut name filling it, has no room for an icon");
    let tree = (shown(&format!("git-worktree-label-{w}-icon-0")), shown(&format!("git-worktree-label-{w}-icon-1")));
    assert!(tree.1 || !tree.0, "the worktree's icon is the last to go: {tree:?}");
    for (pill, name) in &pills {
        let shown = at(&hits, pill);
        assert!(shown.left() >= column.left() && shown.right() <= column.right(), "{pill} inside its column: {shown:?} {column:?}");
        let cut = cut_at(name).unwrap_or_else(|| panic!("{name} is cut: {shown:?} {column:?}"));
        assert!(cut.right() <= shown.right() && cut.size.width >= px(super::NAME_MIN), "{name}'s first letter and … inside its pill: {cut:?} {shown:?}");
    }
}

#[gpui::test]
fn the_date_and_sha_paint_a_step_under_the_body_muted_in_figures(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (_view, cx) = test_support::open(&f.dir, Opts::default(), cx);
    let palette = cx.update(|_, cx| Palette::of(theme::ActiveTheme::theme(cx)));
    let mut cell = figure_text(div(), &palette);
    let text = &cell.style().text;
    assert_eq!(text.font_size, Some(rems(0.75).into()), "about 0.85 of the body's text_sm");
    assert_eq!(text.color, Some(palette.muted));
    assert_eq!(text.font_family.as_ref().map(|f| f.to_string()), Some(supraman_ui::fonts::UI.to_string()));
    assert!(text.font_features.is_some(), "tabular figures");
}

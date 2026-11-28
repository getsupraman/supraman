use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext, px};

use super::*;
use crate::work_view::tests::{Opened, bounds, open_sized, resize, settle};

const LIST_KEYS: [&str; 6] = ["work-row-", "work-dot-", "work-filter", "work-crumb", "work-squares", "work-prompt-run-"];

/// Every run list key painted now, within `width` and wholly inside its clip (one the bottom cuts, across).
fn assert_list_shown(view: &gpui::Entity<WorkView>, width: f32, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let hits = hits.borrow();
    let keys: Vec<String> = hits.keys().filter(|k| LIST_KEYS.iter().any(|p| k.starts_with(p))).cloned().collect();
    assert!(keys.iter().any(|k| k.starts_with("work-row-")) && keys.iter().any(|k| k.starts_with("work-filter")), "the run list: {keys:?}");
    for key in keys {
        let hit = hits[&key];
        let across = hit.bounds.left() >= hit.visible.left() - px(1.) && hit.bounds.right() <= hit.visible.right().min(px(width)) + px(1.);
        let down = hit.bounds.bottom() > hit.visible.bottom() || hit.fully_visible();
        assert!(across && down, "{key} shown whole at {width}: {hit:?}");
    }
}

#[gpui::test]
fn narrow_at_640_the_run_list_shows_whole_with_the_detail_under_it(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open_sized((640., 700.), cx);
    assert_list_shown(&view, 640., cx);
    let (filter, detail) = (bounds(&view, "work-filter-open", cx), bounds(&view, "work-detail-column", cx));
    assert!(detail.top() > filter.bottom() && detail.left() <= filter.left() && detail.right() >= px(639.), "the detail {detail:?} under the list {filter:?}");
    bounds(&view, "work-nav-open", cx);
    assert!(view.read_with(cx, |v, _| !v.layout.nav_shut() && v.hits.borrow().get("work-nav-overview").is_none()), "the strip; the user's open navigator is still theirs");
}

#[gpui::test]
fn narrow_at_the_least_size_the_run_list_shows_whole_across(cx: &mut TestAppContext) {
    let (w, h) = (f32::from(min_size().width), f32::from(min_size().height));
    let Opened { view, cx, .. } = open_sized((w, h), cx);
    assert_list_shown(&view, w, cx);
}

fn layout(view: &gpui::Entity<WorkView>, cx: &mut VisualTestContext) -> (Pixels, Pixels, bool, Bounds<Pixels>, Bounds<Pixels>) {
    let kept = view.read_with(cx, |v, _| (v.layout.nav(), v.layout.detail(), v.layout.nav_shut()));
    (kept.0, kept.1, kept.2, bounds(view, "work-nav-overview", cx), bounds(view, "work-detail-column", cx))
}

#[gpui::test]
fn narrow_then_wide_again_gives_back_the_kept_widths(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open_sized((1500., 900.), cx);
    let before = layout(&view, cx);
    for width in [780., 640., 360.] {
        resize((width, 900.), cx);
        settle(&view, cx);
        assert_eq!(view.read_with(cx, |v, _| (v.layout.nav(), v.layout.detail(), v.layout.nav_shut())), (before.0, before.1, before.2), "kept at {width}");
    }
    resize((1500., 900.), cx);
    settle(&view, cx);
    assert_eq!(layout(&view, cx), before);
}

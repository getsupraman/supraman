//! Two Git tabs painted in one window, each under its own key scope, share no key.

use std::collections::BTreeSet;

use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
use supraman_gitman::fixture::Fixture;
use supraman_ui::hit::HitTargets;

use crate::git_view::branch::menu::RefAct;
use crate::git_view::test_support::{self, Opts, click, keys, open_pair, pane_focused, redraw, repaint, right_click, row_of};
use crate::git_view::*;

const SCOPES: [&str; 2] = ["a", "b"];

fn keys_under(hits: &HitTargets, scope: &str) -> BTreeSet<String> {
    hits.borrow().keys().filter_map(|k| k.strip_prefix(&format!("{scope}/")).map(str::to_string)).collect()
}

fn panel_width(hits: &HitTargets, scope: &str) -> f32 {
    f32::from(hits.borrow().get(&format!("{scope}/{PANEL_PANE}")).unwrap_or_else(|| panic!("{scope}'s commit panel paints")).bounds.size.width)
}

fn order(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, cx| supraman_ui::focus::order(window, cx).into_iter().map(|k| k.to_string()).collect())
}

#[gpui::test]
fn two_views_in_one_window_keep_their_keys_panes_and_dividers_apart(cx: &mut TestAppContext) {
    let (first, second) = (Fixture::copy(), Fixture::copy());
    let ([a, _], cx) = open_pair([&first.dir, &second.dir], SCOPES, cx);

    let hits = repaint(&a, cx);
    let unscoped: Vec<String> = hits.borrow().keys().filter(|k| !SCOPES.iter().any(|s| k.starts_with(&format!("{s}/")))).cloned().collect();
    assert!(unscoped.is_empty(), "every key a view paints lies under its scope: {unscoped:?}");
    let (under_a, under_b) = (keys_under(&hits, "a"), keys_under(&hits, "b"));
    for key in [REFS_PANE, GRAPH_PANE, PANEL_PANE, "git-row-0", "git-col-message"] {
        assert!(under_a.contains(key) && under_b.contains(key), "both views paint {key}");
    }
    assert_eq!(under_a, under_b, "two tabs of one repo paint the same keys, each under its own scope");

    let second_row = row_of(&a, first.sha("second"), cx);
    click(&a, &format!("a/git-row-{second_row}"), cx);
    let panes = [FOCUS_REFS, FOCUS_GRAPH, FOCUS_PANEL];
    let expected: Vec<String> = SCOPES.iter().flat_map(|s| panes.iter().map(move |p| format!("{s}/{p}"))).collect();
    assert_eq!(order(cx), expected, "F6's order holds both views' panes");
    let mut visited = vec![pane_focused(cx).expect("a pane holds the keys")];
    for _ in 1..expected.len() {
        keys(&a, "f6", cx);
        visited.push(pane_focused(cx).expect("a pane holds the keys"));
    }
    assert_eq!(BTreeSet::from_iter(visited.iter().cloned()), BTreeSet::from_iter(expected.iter().cloned()), "F6 visits every pane of both views: {visited:?}");

    let hits = repaint(&a, cx);
    let (was_a, was_b) = (panel_width(&hits, "a"), panel_width(&hits, "b"));
    let panel = hits.borrow().get(&format!("a/{PANEL_PANE}")).copied().expect("a's commit panel paints").bounds;
    let start = point(panel.left() - px(1.), panel.center().y);
    let end = start - point(px(100.), px(0.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(start - point(px(50.), px(0.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    let hits = repaint(&a, cx);
    let (now_a, now_b) = (panel_width(&hits, "a"), panel_width(&hits, "b"));
    assert!((now_a - was_a - 100.).abs() < 2., "a's divider follows the pointer 100 px left: {was_a} → {now_a}");
    assert_eq!(now_b, was_b, "b's commit panel keeps its width while a's divider is dragged");
}

#[gpui::test]
fn two_views_scope_their_menus_anchor_and_their_prompts_pane(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = test_support::open(&f.dir, Opts::default().scope("p"), cx);
    let row = row_of(&view, f.sha("second"), cx);
    right_click(&view, &format!("p/git-row-{row}"), cx);
    assert!(supraman_ui::popup::painted_open(&format!("p/git-row-{row}")) > 0, "the row its menu belongs to paints open under the view's scope");
    assert_eq!(supraman_ui::popup::painted_open(&format!("git-row-{row}")), 0, "nothing paints open under the bare key");

    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.act(RefAct::DeleteBranch("feature".into()), cx)));
    cx.run_until_parked();
    redraw(&view, cx);
    assert_eq!(pane_focused(cx).as_deref(), Some("p/prompt"), "the tab's prompt is F6's pane under the view's scope");
}

//! Two Work tabs painted in one window, each under its own key scope, share no key.

use std::collections::BTreeSet;

use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};

use super::*;

const SCOPES: [&str; 2] = ["a", "b"];
const WIDTH: f32 = 1200.;

struct Pair {
    views: [Entity<WorkView>; 2],
    _restore: gpui::Subscription,
}

impl Render for Pair {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let half = |view: &Entity<WorkView>| div().flex_none().w(px(WIDTH)).h_full().child(view.clone());
        focus::root(div().flex().w(px(WIDTH * 2.)).h(px(900.))).child(half(&self.views[0])).child(half(&self.views[1]))
    }
}

fn repaint(pair: &[Entity<WorkView>; 2], cx: &mut VisualTestContext) -> HitTargets {
    for _ in 0..3 {
        cx.run_until_parked();
        pair[0].update(cx, |v, _| v.hits.borrow_mut().clear());
        for view in pair {
            view.update(cx, |_, cx| cx.notify());
        }
        cx.run_until_parked();
        cx.update(|_, _| ());
    }
    pair[0].read_with(cx, |v, _| v.hits.clone())
}

fn keys_under(hits: &HitTargets, scope: &str) -> BTreeSet<String> {
    hits.borrow().keys().filter_map(|k| k.strip_prefix(&format!("{scope}/")).map(str::to_string)).collect()
}

fn detail_width(hits: &HitTargets, scope: &str) -> f32 {
    f32::from(hits.borrow().get(&format!("{scope}/work-detail-column")).unwrap_or_else(|| panic!("{scope}'s detail paints")).bounds.size.width)
}

#[gpui::test]
fn two_views_of_two_targets_keep_their_keys_panes_and_dividers_apart(cx: &mut TestAppContext) {
    let (_fake, client) = start(cx);
    let group = Target::Group("w".into());
    let alpha = Target::Folder("alpha".into());
    let (holder, cx) = cx.add_window_view(|window, cx| {
        let root = HitTargets::default();
        let (hub, inbox) = hub_and_inbox(client, vec![(group.clone(), folders()), (alpha.clone(), vec!["alpha".into()])], cx);
        let views = [view_of(group, folders(), root.scoped(SCOPES[0]), &hub, &inbox, cx), view_of(alpha, vec!["alpha".into()], root.scoped(SCOPES[1]), &hub, &inbox, cx)];
        Pair { views, _restore: focus::restore_on_loss(window, cx) }
    });
    cx.update(|window, _| window.activate_window());
    let pair = holder.read_with(cx, |p, _| p.views.clone());
    for (view, scope) in pair.iter().zip(SCOPES) {
        assert_eq!(view.read_with(cx, |v, _| v.page.clone()), Page::Run(RA.into()), "{scope} opens on pa's active run");
    }
    let hits = repaint(&pair, cx);
    let unscoped: Vec<String> = hits.borrow().keys().filter(|k| !SCOPES.iter().any(|s| k.starts_with(&format!("{s}/")))).cloned().collect();
    assert!(unscoped.is_empty(), "every key a view paints lies under its scope: {unscoped:?}");
    let (under_a, under_b) = (keys_under(&hits, "a"), keys_under(&hits, "b"));
    for key in ["work-nav-overview", "work-nav-run-ra", "work-detail-column", "work-crumb-project"] {
        assert!(under_a.contains(key) && under_b.contains(key), "both views paint {key}");
    }
    assert!(under_a.contains("work-nav-run-rb") && !under_b.contains("work-nav-run-rb"), "only the group's tab lists beta's run");

    let panes = [FOCUS_NAV, FOCUS_MAIN, FOCUS_DETAIL];
    let expected: Vec<String> = SCOPES.iter().flat_map(|s| panes.iter().map(move |p| format!("{s}/{p}"))).collect();
    let order: Vec<String> = cx.update(|window, cx| focus::order(window, cx).into_iter().map(|k| k.to_string()).collect());
    assert_eq!(order, expected, "F6's order holds both views' panes");
    let row = hits.borrow().get("b/work-row-ra/3").copied().expect("b's row paints").bounds;
    cx.simulate_click(row.center(), Modifiers::none());
    repaint(&pair, cx);
    assert_eq!(holding(cx).as_deref(), Some("b/work-main"), "a click on b's row gives b's list the keys");
    cx.simulate_keystrokes("enter");
    repaint(&pair, cx);
    assert_eq!(holding(cx).as_deref(), Some("b/work-detail"), "Enter in b's list gives b's detail the keys");
    cx.simulate_keystrokes("left");
    repaint(&pair, cx);
    assert_eq!(holding(cx).as_deref(), Some("b/work-main"), "Left in b's detail gives b's list the keys back");

    let (was_a, was_b) = (detail_width(&hits, "a"), detail_width(&hits, "b"));
    let detail = hits.borrow().get("a/work-detail-column").copied().expect("a's detail paints").bounds;
    let start = point(detail.left() - px(1.), detail.center().y);
    let end = start - point(px(100.), px(0.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(start - point(px(50.), px(0.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    let hits = repaint(&pair, cx);
    let (now_a, now_b) = (detail_width(&hits, "a"), detail_width(&hits, "b"));
    assert!((now_a - was_a - 100.).abs() < 2., "a's divider follows the pointer 100 px left: {was_a} → {now_a}");
    assert_eq!(now_b, was_b, "b's detail keeps its width while a's divider is dragged");
}

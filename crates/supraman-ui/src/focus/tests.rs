use gpui::{Context, Entity, FocusHandle, TestAppContext, VisualTestContext, Window, div, prelude::*, px};

use super::*;
use crate::focus_order::stepped;
use crate::hit::HitTargets;

#[test]
fn stepping_goes_forward_and_back_wrapping_and_from_none_to_the_first_or_last() {
    assert_eq!(stepped(Some(0), 3, false), Some(1));
    assert_eq!(stepped(Some(2), 3, false), Some(0), "wraps forward");
    assert_eq!(stepped(Some(0), 3, true), Some(2), "wraps back");
    assert_eq!(stepped(Some(1), 3, true), Some(0));
    assert_eq!(stepped(None, 3, false), Some(0), "none focused: the first");
    assert_eq!(stepped(None, 3, true), Some(2), "none focused: the last");
    assert_eq!(stepped(None, 0, false), None);
}

#[test]
fn a_later_pane_follows_the_rest_in_paint_order() {
    assert_eq!(ordered(&[("refs", false), ("diff", true), ("panel", false)], |p| p.1).iter().map(|p| p.0).collect::<Vec<_>>(), ["refs", "panel", "diff"]);
}

struct Window3 {
    panes: [FocusHandle; 3],
    inner: FocusHandle,
    /// The window's own, as the app's root holds the keys when no pane does.
    base: FocusHandle,
    middle_shown: bool,
    popup: Option<FocusHandle>,
    hits: HitTargets,
    _restore: Option<gpui::Subscription>,
}

const KEYS: [&str; 3] = ["left", "middle", "right"];

impl Render for Window3 {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pane = |i: usize, this: &Self| {
            let el = div().id(KEYS[i]).relative().w(px(100.)).h_full().track_focus(&this.panes[i]);
            let el = if i == 2 { el.child(div().id("inner").size_4().track_focus(&this.inner)) } else { el };
            el.child(Pane::new(KEYS[i], &this.panes[i]).paint(&this.hits))
        };
        let mut row = root(div().flex().size_full().track_focus(&self.base)).child(pane(0, self));
        if self.middle_shown {
            row = row.child(pane(1, self));
        }
        row.child(pane(2, self)).children(self.popup.as_ref().map(|p| div().id("popup").size_4().track_focus(p)))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Window3>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|window, cx| {
        let restore = restore_on_loss(window, cx);
        let base = cx.focus_handle();
        base.focus(window, cx);
        Window3 { panes: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()], inner: cx.focus_handle(), base, middle_shown: true, popup: None, hits: HitTargets::default(), _restore: Some(restore) }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (view, cx)
}

fn holding(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, cx| focused(window, cx).map(|k| k.to_string()))
}

fn marks(view: &Entity<Window3>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let mut marked: Vec<String> = KEYS.iter().map(|k| mark_key(k)).filter(|k| hits.borrow().contains_key(k)).collect();
    marked.sort();
    marked
}

#[gpui::test]
fn f6_walks_the_shown_panes_and_wraps_and_shift_f6_walks_back(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    assert_eq!(cx.update(|window, cx| order(window, cx)), ["left", "middle", "right"]);
    cx.simulate_keystrokes("f6");
    assert_eq!(holding(cx).as_deref(), Some("left"), "none focused: F6 the first");
    let mut walked = Vec::new();
    for _ in 0..3 {
        cx.simulate_keystrokes("f6");
        walked.push(holding(cx));
    }
    assert_eq!(walked, [Some("middle".into()), Some("right".into()), Some("left".into())]);
    cx.simulate_keystrokes("shift-f6");
    assert_eq!(holding(cx).as_deref(), Some("right"), "Shift+F6 wraps back");
    view.update(cx, |v, cx| {
        v.middle_shown = false;
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("f6 f6");
    assert_eq!(holding(cx).as_deref(), Some("right"), "the hidden middle is skipped: right, left, right");
}

#[gpui::test]
fn shift_f6_with_none_focused_goes_to_the_last(cx: &mut TestAppContext) {
    let (_, cx) = open(cx);
    cx.simulate_keystrokes("shift-f6");
    assert_eq!(holding(cx).as_deref(), Some("right"));
}

#[gpui::test]
fn only_the_pane_holding_the_focus_wears_the_mark(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    assert!(marks(&view, cx).is_empty(), "nothing focused, nothing marked");
    cx.simulate_keystrokes("f6 f6");
    assert_eq!(marks(&view, cx), [mark_key("middle")]);
    let inner = view.read_with(cx, |v, _| v.inner.clone());
    cx.update(|window, cx| window.focus(&inner, cx));
    assert_eq!(marks(&view, cx), [mark_key("right")], "focus inside a pane marks it");
}

#[gpui::test]
fn a_closed_popup_gives_the_focus_back_to_what_the_pane_held(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let inner = view.read_with(cx, |v, _| v.inner.clone());
    cx.update(|window, cx| window.focus(&inner, cx));
    cx.run_until_parked();
    let popup = view.update(cx, |v, cx| {
        let popup = cx.focus_handle();
        v.popup = Some(popup.clone());
        cx.notify();
        popup
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.focus(&popup, cx));
    cx.run_until_parked();
    assert_eq!(holding(cx), None, "the popup holds it");
    view.update(cx, |v, cx| {
        v.popup = None;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.update(|window, _| inner.is_focused(window)), "back in the right pane, on what it held");
    cx.simulate_keystrokes("f6");
    cx.simulate_keystrokes("shift-f6");
    assert!(cx.update(|window, _| inner.is_focused(window)), "F6 back into a pane focuses what it held");
}

/// A window's sidebar pane and its tabs A and B shown in turn in one place, each a group of its entry and a field inside.
struct Tabs {
    sidebar: FocusHandle,
    area: FocusHandle,
    entries: [FocusHandle; 2],
    fields: [FocusHandle; 2],
    shown: usize,
    popup: Option<FocusHandle>,
    hits: HitTargets,
    /// Each tab's content a pane under one key, as every Git tab's graph is `git-graph`.
    shared_pane: bool,
    /// What the window would show when a group takes the keys, as the app's active tab.
    active: std::rc::Rc<std::cell::Cell<usize>>,
    _restore: gpui::Subscription,
}

impl Render for Tabs {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tab = self.shown;
        let field = div().id("field").size_4().track_focus(&self.fields[tab]);
        let content = div().id(KEYS[tab]).relative().size_full().track_focus(&self.entries[tab]).child(field).children(self.shared_pane.then(|| Pane::new("content", &self.entries[tab]).paint(&self.hits)));
        let area = div().id("area").relative().w(px(200.)).h_full().track_focus(&self.area).child(content).child(self.group(tab).paint());
        let sidebar = div().id("sidebar").relative().w(px(100.)).h_full().track_focus(&self.sidebar).child(Pane::new("sidebar", &self.sidebar).paint(&self.hits));
        root(div().flex().size_full()).child(sidebar).child(area).children(self.popup.as_ref().map(|p| div().id("popup").size_4().track_focus(p)))
    }
}

impl Tabs {
    fn group(&self, tab: usize) -> Group {
        let active = self.active.clone();
        Group::new(KEYS[tab], &self.area, &self.entries[tab]).while_shown(move |_| active.get() == tab)
    }
}

fn open_tabs(cx: &mut TestAppContext) -> (Entity<Tabs>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|window, cx| {
        let area = cx.focus_handle();
        area.focus(window, cx);
        let (entries, fields) = ([cx.focus_handle(), cx.focus_handle()], [cx.focus_handle(), cx.focus_handle()]);
        Tabs { sidebar: cx.focus_handle(), area, entries, fields, shown: 0, popup: None, hits: HitTargets::default(), shared_pane: false, active: Default::default(), _restore: restore_on_loss(window, cx) }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (view, cx)
}

fn show_tab(view: &Entity<Tabs>, tab: usize, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.shown = tab;
        v.active.set(tab);
        cx.notify();
    });
    cx.run_until_parked();
}

fn focused_is(handle: &FocusHandle, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, _| handle.is_focused(window))
}

#[gpui::test]
fn a_tab_shown_takes_the_keys_what_it_held_when_last_shown_else_its_entry(cx: &mut TestAppContext) {
    let (view, cx) = open_tabs(cx);
    let (entries, fields) = view.read_with(cx, |v, _| (v.entries.clone(), v.fields.clone()));
    assert!(focused_is(&entries[0], cx), "the first tab shown takes the keys from the window's own element: its entry");
    cx.update(|window, cx| window.focus(&fields[0], cx));
    cx.run_until_parked();
    show_tab(&view, 1, cx);
    assert!(focused_is(&entries[1], cx), "a tab never shown takes them on its entry");
    show_tab(&view, 0, cx);
    assert!(focused_is(&fields[0], cx), "back to A: what A held");
    show_tab(&view, 1, cx);
    assert!(focused_is(&entries[1], cx), "B held its entry");
}

#[gpui::test]
fn a_tab_shown_takes_the_keys_from_a_pane_but_not_from_a_popup(cx: &mut TestAppContext) {
    let (view, cx) = open_tabs(cx);
    let (sidebar, entries) = view.read_with(cx, |v, _| (v.sidebar.clone(), v.entries.clone()));
    cx.update(|window, cx| window.focus(&sidebar, cx));
    cx.run_until_parked();
    show_tab(&view, 1, cx);
    assert!(focused_is(&entries[1], cx), "the tab takes the keys from the sidebar that showed it");
    let popup = view.update(cx, |v, cx| {
        let popup = cx.focus_handle();
        v.popup = Some(popup.clone());
        cx.notify();
        popup
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.focus(&popup, cx));
    cx.run_until_parked();
    show_tab(&view, 0, cx);
    assert!(focused_is(&popup, cx), "an open popup keeps them");
}

#[gpui::test]
fn a_tab_no_longer_shown_when_it_would_take_the_keys_leaves_them(cx: &mut TestAppContext) {
    let (view, cx) = open_tabs(cx);
    let (area, entries) = view.read_with(cx, |v, _| (v.area.clone(), v.entries.clone()));
    view.update(cx, |v, cx| {
        v.shown = 1;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!focused_is(&entries[1], cx) && focused_is(&area, cx), "B painted while A is the active one does not take the keys");
}

#[gpui::test]
fn a_tab_that_focused_its_own_field_as_it_opened_keeps_it(cx: &mut TestAppContext) {
    let (view, cx) = open_tabs(cx);
    let fields = view.read_with(cx, |v, _| v.fields.clone());
    view.update_in(cx, |v, window, cx| {
        v.shown = 1;
        v.active.set(1);
        window.focus(&v.fields[1], cx);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(focused_is(&fields[1], cx), "B's own field keeps the keys, not B's entry");
}

#[gpui::test]
fn tabs_sharing_a_pane_key_each_get_back_what_they_held(cx: &mut TestAppContext) {
    let (view, cx) = open_tabs(cx);
    view.update(cx, |v, _| v.shared_pane = true);
    let fields = view.read_with(cx, |v, _| v.fields.clone());
    show_tab(&view, 1, cx);
    cx.update(|window, cx| window.focus(&fields[1], cx));
    cx.run_until_parked();
    show_tab(&view, 0, cx);
    cx.update(|window, cx| window.focus(&fields[0], cx));
    cx.run_until_parked();
    show_tab(&view, 1, cx);
    assert!(focused_is(&fields[1], cx), "B's field, not the shared pane's restore onto B's entry");
    show_tab(&view, 0, cx);
    assert!(focused_is(&fields[0], cx));
}

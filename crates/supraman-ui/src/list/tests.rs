use gpui::{Context, Entity, FocusHandle, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px};

use super::*;
use crate::controls::button::RowKeys;
use crate::hit::HitTargets;
use crate::popup::Popup;
use crate::tokens::Layer;

fn keystroke(k: &str) -> Keystroke {
    Keystroke::parse(k).expect("a keystroke")
}

#[test]
fn every_list_key_maps_to_one_step() {
    let cases = [
        ("up", Key::Step(Step::By(-1))),
        ("down", Key::Step(Step::By(1))),
        ("pageup", Key::Step(Step::Pages(-1))),
        ("pagedown", Key::Step(Step::Pages(1))),
        ("home", Key::Step(Step::First)),
        ("end", Key::Step(Step::Last)),
        ("ctrl-home", Key::Step(Step::First)),
        ("ctrl-end", Key::Step(Step::Last)),
        ("enter", Key::Activate),
        ("b", Key::Typed("b".into())),
        ("7", Key::Typed("7".into())),
    ];
    for (k, want) in cases {
        assert_eq!(key_of(&keystroke(k)), Some(want), "{k}");
    }
    for k in ["ctrl-b", "alt-down", "tab", "space", "left"] {
        assert_eq!(key_of(&keystroke(k)), None, "{k} is not a list's");
    }
}

#[test]
fn steps_stop_at_the_ends_and_pages_move_by_the_rows_in_view() {
    let rows = Rows::new(20).page(5);
    let mut s = Selection::default();
    assert_eq!(s.go(Step::By(1), &rows), Some(0), "the first step selects the first row");
    assert_eq!(s.go(Step::Pages(1), &rows), Some(5));
    assert_eq!(s.go(Step::Pages(1), &rows), Some(10));
    assert_eq!(s.go(Step::Last, &rows), Some(19));
    assert_eq!(s.go(Step::By(1), &rows), None, "Down on the last row stays");
    assert_eq!(s.go(Step::Pages(1), &rows), None);
    assert_eq!(s.go(Step::Pages(-1), &rows), Some(14));
    assert_eq!(s.go(Step::First, &rows), Some(0));
    assert_eq!(s.go(Step::By(-1), &rows), None, "Up on the first row stays");
}

#[test]
fn a_wrapping_list_steps_round_its_ends_over_the_rows_that_act() {
    let acts = [true, false, true, true];
    let rows = Rows::new(4).wrapping().acting(&acts);
    let mut s = Selection::default();
    assert_eq!(s.go(Step::By(1), &rows), Some(0));
    assert_eq!(s.go(Step::By(1), &rows), Some(2), "the row that cannot act is passed");
    assert_eq!(s.go(Step::By(1), &rows), Some(3));
    assert_eq!(s.go(Step::By(1), &rows), Some(0), "Down from the last wraps");
    assert_eq!(s.go(Step::By(-1), &rows), Some(3));
    let plain = Rows::new(4).acting(&acts);
    s.select(Some(0));
    assert_eq!(s.go(Step::By(1), &plain), Some(2));
    assert_eq!(s.go(Step::First, &plain), Some(0));
}

#[test]
fn typed_letters_find_the_next_row_they_start_and_add_up_while_quick() {
    let labels = [("alpha", true), ("beta", true), ("bravo", true), ("charlie", true)];
    let mut s = Selection::default();
    let t0 = Instant::now();
    assert_eq!(s.type_ahead("b", &labels, t0), Some(1));
    assert_eq!(s.type_ahead("b", &labels, t0), Some(2), "the same letter again steps on");
    assert_eq!(s.type_ahead("c", &labels, t0 + TYPED_FOR * 2), Some(3), "after a pause a letter starts anew");
    s.select(None);
    assert_eq!(s.type_ahead("b", &labels, t0 + TYPED_FOR * 4), Some(1));
    assert_eq!(s.type_ahead("r", &labels, t0 + TYPED_FOR * 4), Some(2), "quick letters add up: br");
}

/// A list on the core: rows `row-<n>` painted by [`row`], its keys moving its [`Selection`] by [`key_of`].
struct Listed {
    hits: HitTargets,
    focus: FocusHandle,
    selection: Selection,
    names: Vec<&'static str>,
    popup: Option<Popup>,
}

const NAMES: [&str; 12] = ["alpha", "beta", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet", "kilo"];
const PAGE: usize = 4;

impl Render for Listed {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(popup) = &self.popup {
            popup.shown(window, cx);
        }
        let focused = self.focus.is_focused(window);
        let rows = self.names.iter().enumerate().map(|(i, name)| {
            let chrome = Chrome::new().selected(self.selection.selected() == Some(i)).focused(focused);
            row(&self.hits, RowKeys::one(format!("row-{i}").into()), chrome, cx).h(px(20.)).child(*name)
        });
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                let Some(key) = key_of(&e.keystroke) else { return };
                let rows = Rows::new(this.names.len()).page(PAGE);
                let names = this.names.clone();
                this.selection.key(&key, &rows, || Some(names.iter().map(|n| (n.to_string(), true)).collect()));
                cx.notify();
            }))
            .w(px(200.))
            .flex()
            .flex_col()
            .children(rows)
    }
}

fn listed(cx: &mut TestAppContext) -> (Entity<Listed>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Listed { hits: HitTargets::default(), focus, selection: Selection::default(), names: NAMES.to_vec(), popup: None }
    });
    cx.run_until_parked();
    (view, cx)
}

fn selected(view: &Entity<Listed>, cx: &mut VisualTestContext) -> Option<usize> {
    view.read_with(cx, |v, _| v.selection.selected())
}

/// The fills painted under the middle of `hit`.
fn fills_at(view: &Entity<Listed>, hit: &str, cx: &mut VisualTestContext) -> Vec<gpui::Background> {
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let bounds = view.read_with(cx, |v, _| v.hits.borrow().get(hit).map(|h| h.bounds)).unwrap_or_else(|| panic!("{hit} paints"));
    cx.update(|window, _| {
        let middle = bounds.center().scale(window.scale_factor());
        window.painted_quads().into_iter().filter(|q| q.bounds.contains(&middle) && q.content_mask.bounds.contains(&middle)).map(|q| q.background).collect()
    })
}

#[gpui::test]
fn the_list_keys_move_the_selection_by_a_row_a_page_and_to_the_ends(cx: &mut TestAppContext) {
    let (view, cx) = listed(cx);
    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, cx), Some(0));
    cx.simulate_keystrokes("pagedown");
    assert_eq!(selected(&view, cx), Some(PAGE), "PageDown moves a page of rows");
    cx.simulate_keystrokes("pagedown down");
    assert_eq!(selected(&view, cx), Some(2 * PAGE + 1));
    cx.simulate_keystrokes("end");
    assert_eq!(selected(&view, cx), Some(NAMES.len() - 1));
    cx.simulate_keystrokes("pageup");
    assert_eq!(selected(&view, cx), Some(NAMES.len() - 1 - PAGE));
    cx.simulate_keystrokes("home");
    assert_eq!(selected(&view, cx), Some(0));
}

#[gpui::test]
fn typed_letters_select_the_row_whose_text_they_start(cx: &mut TestAppContext) {
    let (view, cx) = listed(cx);
    cx.simulate_keystrokes("g");
    assert_eq!(selected(&view, cx), Some(7), "g: golf");
    cx.simulate_keystrokes("b");
    assert_eq!(selected(&view, cx), Some(1), "a different letter at once finds the next row it starts: beta");
    cx.simulate_keystrokes("b");
    assert_eq!(selected(&view, cx), Some(2), "b again: bravo");
}

#[gpui::test]
fn the_selection_is_the_active_role_while_the_list_has_the_keys_and_the_inactive_one_after(cx: &mut TestAppContext) {
    let (view, cx) = listed(cx);
    cx.simulate_keystrokes("down down");
    let roles = cx.update(|_, cx| crate::tokens::roles(cx));
    let (active, inactive) = (gpui::Background::from(roles.selection), gpui::Background::from(roles.selection_inactive));
    let shown = fills_at(&view, "row-1", cx);
    assert!(shown.contains(&active) && !shown.contains(&inactive), "focused, the active selection: {shown:?}");
    cx.update(|window, cx| window.blur(cx));
    let shown = fills_at(&view, "row-1", cx);
    assert!(shown.contains(&inactive) && !shown.contains(&active), "without the keys, the inactive one: {shown:?}");
    assert!(!fills_at(&view, "row-2", cx).contains(&inactive), "an unselected row has no selection fill");
}

#[gpui::test]
fn the_row_a_popup_is_anchored_to_paints_the_open_fill_until_it_closes(cx: &mut TestAppContext) {
    let (view, cx) = listed(cx);
    let open = gpui::Background::from(cx.update(|_, cx| crate::tokens::roles(cx).open));
    assert!(!fills_at(&view, "row-3", cx).contains(&open));
    view.update(cx, |v, _| v.popup = Some(Popup::open(Some("row-3".into()), Layer::Menu)));
    let before = crate::popup::painted_open("row-3");
    assert!(fills_at(&view, "row-3", cx).contains(&open), "the anchored row paints open");
    assert!(crate::popup::painted_open("row-3") > before);
    assert!(!fills_at(&view, "row-4", cx).contains(&open), "only the anchored row");
    view.update(cx, |v, _| v.popup = None);
    assert!(!fills_at(&view, "row-3", cx).contains(&open), "closed, the fill goes");
}

use std::{cell::Cell, rc::Rc};

use gpui::{Context, Entity, Modifiers, MouseButton, Render, TestAppContext, VisualTestContext, Window, canvas, div, point, px};

use super::*;
use crate::hit::HitTargets;

struct Host {
    hits: HitTargets,
    menu: Option<ContextMenu<u8>>,
    /// Another owner's menu.
    other: Option<ContextMenu<u8>>,
    picked: Vec<u8>,
    dismissed: usize,
    /// Whether `row-a` painted open in the last frame.
    row_open: Rc<Cell<bool>>,
}

impl Host {
    fn open(&mut self, items: Vec<MenuItem<u8>>, cx: &mut Context<Self>) {
        self.menu = Some(ContextMenu::new("Menu", point(px(20.), px(40.)), items).anchored("row-a"));
        cx.notify();
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let seen = self.row_open.clone();
        let row = div().w(px(100.)).h(px(20.)).child(canvas(move |_, window, cx| seen.set(crate::popup::is_open("row-a", window, cx)), |_, _, _, _| {}).size_full());
        let menu = self.menu.as_ref().map(|m| {
            m.render(
                &self.hits,
                |this: &mut Host, a, _, cx| {
                    this.picked.push(a);
                    this.menu = None;
                    cx.notify();
                },
                |this: &mut Host, cx| {
                    this.dismissed += 1;
                    this.menu = None;
                    cx.notify();
                },
                cx,
            )
        });
        let other = self.other.as_ref().map(|m| m.render(&self.hits, |this: &mut Host, _, _, cx| { this.other = None; cx.notify() }, |this: &mut Host, cx| { this.other = None; cx.notify() }, cx));
        crate::focus::root(div().size_full()).child(row).child(div().id("outside").absolute().left(px(400.)).top(px(400.)).size(px(50.))).children(menu).children(other)
    }
}

fn host(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Host { hits: HitTargets::default(), menu: None, other: None, picked: Vec::new(), dismissed: 0, row_open: Rc::default() });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (view, cx)
}

fn settle(view: &Entity<Host>, cx: &mut VisualTestContext) {
    for _ in 0..3 {
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
}

fn five() -> Vec<MenuItem<u8>> {
    vec![item("a", "Alpha", 1), item("b", "Beta", 2).dimmed("not now"), item("c", "Gamma", 3), item("d", "Delta", 4), item("e", "Beta two", 5).dimmed("no")]
}

fn highlighted(view: &Entity<Host>, cx: &mut VisualTestContext) -> (Option<usize>, Option<usize>) {
    view.read_with(cx, |v, _| v.menu.as_ref().map_or((None, None), |m| m.highlighted()))
}

fn keys(view: &Entity<Host>, keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    settle(view, cx);
}

#[gpui::test]
fn the_anchor_is_open_while_its_menu_is_and_not_after_esc_a_pick_or_a_click_outside(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    let open = |view: &Entity<Host>, cx: &mut VisualTestContext| -> bool { view.read_with(cx, |v, _| v.row_open.get()) };
    assert!(!open(&view, cx));
    for close in ["escape", "pick", "outside"] {
        view.update(cx, |v, cx| v.open(five(), cx));
        settle(&view, cx);
        assert!(open(&view, cx), "{close}: the row paints open under its menu");
        match close {
            "escape" => keys(&view, "escape", cx),
            "pick" => keys(&view, "down enter", cx),
            _ => {
                cx.simulate_mouse_down(point(px(420.), px(420.)), MouseButton::Left, Modifiers::none());
                settle(&view, cx);
            }
        }
        assert!(view.read_with(cx, |v, _| v.menu.is_none()), "{close} closes it");
        assert!(!open(&view, cx), "{close}: the row is no longer open");
    }
    assert_eq!(view.read_with(cx, |v, _| (v.picked.clone(), v.dismissed)), (vec![1], 2));
}

#[gpui::test]
fn a_second_menu_of_the_layer_closes_the_first(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    view.update(cx, |v, cx| v.open(five(), cx));
    settle(&view, cx);
    view.update(cx, |v, cx| {
        v.other = Some(ContextMenu::new("Other", point(px(60.), px(40.)), five()).anchored("row-b"));
        cx.notify();
    });
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| (v.menu.is_none(), v.dismissed, v.other.is_some())), (true, 1, true), "the registry holds one popup per layer");
    assert!(!view.read_with(cx, |v, _| v.row_open.get()));
}

#[gpui::test]
fn the_arrows_skip_dimmed_items_and_wrap_and_home_and_end_go_to_the_ends(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    view.update(cx, |v, cx| v.open(five(), cx));
    settle(&view, cx);
    let mut seen = Vec::new();
    for _ in 0..4 {
        keys(&view, "down", cx);
        seen.push(highlighted(&view, cx).0.unwrap());
    }
    assert_eq!(seen, [0, 2, 3, 0], "Beta and Beta two are dimmed");
    keys(&view, "up", cx);
    assert_eq!(highlighted(&view, cx).0, Some(3), "Up from the first wraps to the last that acts");
    keys(&view, "home", cx);
    assert_eq!(highlighted(&view, cx).0, Some(0));
    keys(&view, "end", cx);
    assert_eq!(highlighted(&view, cx).0, Some(3));
    keys(&view, "enter", cx);
    assert_eq!(view.read_with(cx, |v, _| v.picked.clone()), [4], "Enter picks the highlighted item");
}

#[gpui::test]
fn letters_jump_to_the_next_item_starting_with_them_and_space_picks(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    view.update(cx, |v, cx| v.open(vec![item("a", "Stage", 1), item("b", "Stash", 2), item("c", "Discard", 3), item("d", "Stop", 4)], cx));
    settle(&view, cx);
    keys(&view, "s", cx);
    assert_eq!(highlighted(&view, cx).0, Some(0));
    keys(&view, "s", cx);
    assert_eq!(highlighted(&view, cx).0, Some(1), "the same letter again, the next item with it");
    keys(&view, "d", cx);
    assert_eq!(highlighted(&view, cx).0, Some(2));
    keys(&view, "space", cx);
    assert_eq!(view.read_with(cx, |v, _| v.picked.clone()), [3]);
}

fn with_sub() -> Vec<MenuItem<u8>> {
    let ignore = MenuItem::submenu("ignore", "Ignore", vec![item("i1", "This file", 7), item("i2", "All *.rs", 8)]).unwrap();
    vec![item("a", "Stage", 1), ignore, item("c", "Copy path", 3).shortcut("Ctrl+C").icon(crate::icon::Icon::Copy)]
}

#[gpui::test]
fn a_submenu_opens_on_right_closes_on_left_and_its_item_picks(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    view.update(cx, |v, cx| v.open(with_sub(), cx));
    settle(&view, cx);
    keys(&view, "down down right", cx);
    assert_eq!(view.read_with(cx, |v, _| v.menu.as_ref().unwrap().submenu_open()), Some(1));
    assert_eq!(highlighted(&view, cx), (Some(1), Some(0)), "Right highlights the submenu's first item");
    assert!(view.read_with(cx, |v, _| v.hits.borrow().contains_key("menu-i2")), "its items paint");
    keys(&view, "left", cx);
    assert_eq!(view.read_with(cx, |v, _| v.menu.as_ref().unwrap().submenu_open()), None);
    assert!(view.read_with(cx, |v, _| v.menu.is_some()), "Left closes only the submenu");
    keys(&view, "right down enter", cx);
    assert_eq!(view.read_with(cx, |v, _| v.picked.clone()), [8]);
}

#[gpui::test]
fn the_pointer_resting_on_a_submenus_item_opens_it_and_highlights_one_item(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    view.update(cx, |v, cx| v.open(with_sub(), cx));
    settle(&view, cx);
    keys(&view, "down", cx);
    let at = view.read_with(cx, |v, _| v.hits.borrow()["menu-ignore"].bounds.center());
    cx.simulate_mouse_move(at, None, Modifiers::none());
    settle(&view, cx);
    assert_eq!(highlighted(&view, cx).0, Some(1), "the pointer's highlight is the keys' highlight");
    assert_eq!(view.read_with(cx, |v, _| v.menu.as_ref().unwrap().submenu_open()), None, "not before the delay");
    cx.executor().advance_clock(SUBMENU_DELAY * 2);
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.menu.as_ref().unwrap().submenu_open()), Some(1));
    let child = view.read_with(cx, |v, _| v.hits.borrow()["menu-i1"].bounds.center());
    cx.simulate_click(child, Modifiers::none());
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.picked.clone()), [7], "a click in the submenu picks, not a click outside");
}

#[gpui::test]
fn a_menu_taller_than_the_window_scrolls_to_the_highlighted_item(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    let many: Vec<MenuItem<u8>> = (0..80u8).map(|i| item(format!("i{i}"), format!("Item {i}"), i)).collect();
    view.update(cx, |v, cx| v.open(many, cx));
    settle(&view, cx);
    let window = cx.update(|window, _| window.viewport_size().height);
    let menu = view.read_with(cx, |v, _| v.hits.borrow()["menu"].bounds);
    assert!(menu.size.height <= window, "the menu fits the window: {menu:?} in {window:?}");
    keys(&view, "end", cx);
    let last = view.read_with(cx, |v, _| v.hits.borrow()["menu-i79"].clone());
    assert!(last.fully_visible(), "the last item scrolled into view: {last:?}");
}

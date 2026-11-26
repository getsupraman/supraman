use std::rc::Rc;

use gpui::{AnyElement, Context, Entity, Focusable as _, Modifiers, MouseButton, Render, Subscription, TestAppContext, VisualTestContext, Window, div, point, prelude::*, px};

use crate::dialog::{self, Dialog, Panel};
use crate::focus::{request, restore_on_loss, root};
use crate::focus_order::FocusOrder;
use crate::form::{self, Answers, Form};
use crate::hit::HitTargets;
use crate::icon::Icon;
use crate::menu::{ContextMenu, item};
use crate::palette::{Entry, Palette, PaletteEvent, Paint};
use crate::text_input::TextInput;

#[derive(Clone)]
struct E(&'static str);

impl Entry for E {
    fn key(&self) -> gpui::SharedString {
        self.0.into()
    }
    fn text(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Menu,
    Dialog,
    Palette,
    Form,
}

struct Host {
    hits: HitTargets,
    /// The field's pane, so the window's net would hand the field back had nothing else said otherwise.
    pane: gpui::FocusHandle,
    field: Entity<TextInput>,
    menu: Option<ContextMenu<u8>>,
    dialog: Option<Dialog>,
    palette: Option<(Entity<Palette<E>>, Subscription)>,
    form: Option<(Entity<TextInput>, Subscription)>,
    _restore: Subscription,
}

const ANSWERS: Answers<Host> = Answers { ok: Host::close_form, close: Host::close_form };

impl Host {
    fn open(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        match kind {
            Kind::Menu => self.menu = Some(ContextMenu::new("Menu", point(px(20.), px(60.)), vec![item("pick", "Pick", 1u8)])),
            Kind::Dialog => self.dialog = Some(Dialog::default()),
            Kind::Palette => {
                let paint: Paint<E> = Rc::new(|e: &E, _, _, _| div().child(e.0).into_any_element());
                let palette = cx.new(|cx| Palette::new("p", self.hits.clone(), vec![E("one"), E("two")], paint, window, cx));
                palette.update(cx, |p, cx| p.focus(window, cx));
                let sub = cx.subscribe(&palette, |this: &mut Host, _, _: &PaletteEvent<E>, cx| {
                    this.palette = None;
                    cx.notify();
                });
                self.palette = Some((palette, sub));
            }
            Kind::Form => {
                let (input, sub) = form::field("Name", "", ANSWERS, |_, cx| cx.notify(), window, cx);
                request(&input.focus_handle(cx), window, cx);
                self.form = Some((input, sub));
            }
        }
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    fn popup(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(menu) = &self.menu {
            let close = |this: &mut Host, cx: &mut Context<Host>| {
                this.menu = None;
                cx.notify();
            };
            return Some(menu.render(&self.hits, move |this: &mut Host, _, _, cx| close(this, cx), close, cx));
        }
        if let Some(dialog) = &self.dialog {
            let shut = |this: &mut Host, _: &mut Window, cx: &mut Context<Host>| {
                this.dialog = None;
                cx.notify();
            };
            let ok = dialog::Button::new("d-ok", "OK", crate::button::Tone::Neutral, shut);
            let panel = Panel { key: "d".into(), title: None, body: div().into_any_element(), fields: Vec::new(), buttons: vec![ok], first: None, cancel: Rc::new(shut) };
            return Some(dialog.render(&self.hits, panel, cx));
        }
        if let Some((palette, _)) = &self.palette {
            return Some(palette.clone().into_any_element());
        }
        let (input, _) = self.form.as_ref()?;
        let form = Form { key: "f".into(), icon: Icon::Plus, title: "Form".into(), ok: "OK".into(), why: None, running: false, answers: ANSWERS };
        let order = FocusOrder::new([input.focus_handle(cx)]);
        Some(div().w(px(300.)).h(px(200.)).child(form.render(&self.hits, vec![("Name".into(), form::input(&self.hits, "f-name", input))], order, None, cx)).into_any_element())
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let popup = self.popup(cx);
        let pane = div().relative().w(px(200.)).track_focus(&self.pane).child(self.field.clone()).child(crate::focus::Pane::new("left", &self.pane).paint(&self.hits));
        root(div().size_full()).child(pane).children(popup)
    }
}

fn host(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::focus_order::bind_keys(cx);
        crate::text_input::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let field = cx.new(|cx| TextInput::new("Field", None, window, cx));
        Host { hits: HitTargets::default(), pane: cx.focus_handle(), field, menu: None, dialog: None, palette: None, form: None, _restore: restore_on_loss(window, cx) }
    });
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

fn field_focused(view: &Entity<Host>, cx: &mut VisualTestContext) -> bool {
    let field = view.read_with(cx, |v, cx| v.field.focus_handle(cx));
    cx.update(|window, _| field.is_focused(window))
}

fn nothing_focused(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.focused(cx).is_none())
}

fn shown(view: &Entity<Host>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.menu.is_some() || v.dialog.is_some() || v.palette.is_some() || v.form.is_some())
}

fn open(view: &Entity<Host>, kind: Kind, cx: &mut VisualTestContext) {
    view.update_in(cx, |v, window, cx| v.open(kind, window, cx));
    settle(view, cx);
    assert!(shown(view, cx), "{kind:?} opens");
    assert!(!field_focused(view, cx) && !nothing_focused(cx), "{kind:?} holds the keys");
}

/// A pick: the menu's item clicked, the dialog's OK pressed, the palette's row opened, the form's Enter.
fn pick(view: &Entity<Host>, kind: Kind, cx: &mut VisualTestContext) {
    match kind {
        Kind::Menu => {
            let at = view.read_with(cx, |v, _| v.hits.borrow().get("menu-pick").map(|h| h.bounds.center())).expect("the item paints");
            cx.simulate_click(at, Modifiers::none());
        }
        Kind::Dialog => cx.simulate_keystrokes("enter"),
        Kind::Palette | Kind::Form => cx.simulate_keystrokes("enter"),
    }
    settle(view, cx);
}

fn focus_field(view: &Entity<Host>, cx: &mut VisualTestContext) {
    let field = view.read_with(cx, |v, cx| v.field.focus_handle(cx));
    cx.update(|window, cx| field.focus(window, cx));
    settle(view, cx);
    assert!(field_focused(view, cx));
}

const KINDS: [Kind; 4] = [Kind::Menu, Kind::Dialog, Kind::Palette, Kind::Form];

#[gpui::test]
fn each_popup_closed_by_esc_gives_the_keys_back_to_the_field_it_opened_from(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    for kind in KINDS {
        focus_field(&view, cx);
        open(&view, kind, cx);
        cx.simulate_keystrokes("escape");
        settle(&view, cx);
        assert!(!shown(&view, cx), "{kind:?}: Esc closes it");
        assert!(field_focused(&view, cx), "{kind:?}: the field has the keys back after Esc");
    }
}

#[gpui::test]
fn each_popup_closed_by_a_pick_gives_the_keys_back_to_the_field_it_opened_from(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    for kind in KINDS {
        focus_field(&view, cx);
        open(&view, kind, cx);
        pick(&view, kind, cx);
        assert!(!shown(&view, cx), "{kind:?}: the pick closes it");
        assert!(field_focused(&view, cx), "{kind:?}: the field has the keys back after a pick");
    }
}

#[gpui::test]
fn a_popup_opened_while_nothing_held_the_keys_gives_them_to_nothing(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    for kind in KINDS {
        cx.update(|window, cx| window.blur(cx));
        settle(&view, cx);
        assert!(nothing_focused(cx));
        open(&view, kind, cx);
        cx.simulate_keystrokes("escape");
        settle(&view, cx);
        assert!(!shown(&view, cx), "{kind:?}: Esc closes it");
        assert!(nothing_focused(cx), "{kind:?}: nothing held the keys, so nothing has them");
    }
}

#[gpui::test]
fn a_menu_closed_before_a_second_frame_leaves_no_keys_on_its_handle(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    focus_field(&view, cx);
    view.update_in(cx, |v, window, cx| v.open(Kind::Menu, window, cx));
    cx.run_until_parked();
    let at = view.read_with(cx, |v, _| v.hits.borrow().get("field-outside").map(|h| h.bounds.center())).unwrap_or(point(px(400.), px(400.)));
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    settle(&view, cx);
    assert!(!shown(&view, cx), "a press outside closes it");
    assert!(field_focused(&view, cx), "the keys are back on the field, not on the closed menu's handle");
}

#[gpui::test]
fn a_request_for_a_handle_no_frame_paints_does_nothing(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    focus_field(&view, cx);
    let stray = cx.update(|window, cx| {
        let stray = cx.focus_handle();
        request(&stray, window, cx);
        stray
    });
    settle(&view, cx);
    assert!(field_focused(&view, cx), "the field keeps the keys");
    assert!(!cx.update(|window, _| stray.is_focused(window)));
}

#[gpui::test]
fn a_menu_opened_as_the_keys_were_dropped_gives_them_to_nothing_though_its_pane_held_a_field(cx: &mut TestAppContext) {
    let (view, cx) = host(cx);
    focus_field(&view, cx);
    view.update_in(cx, |v, window, cx| {
        window.blur(cx);
        v.open(Kind::Menu, window, cx);
    });
    settle(&view, cx);
    assert!(shown(&view, cx) && !field_focused(&view, cx), "the menu holds the keys");
    cx.simulate_keystrokes("escape");
    settle(&view, cx);
    assert!(!shown(&view, cx));
    assert!(nothing_focused(cx), "nothing held the keys as it opened, so the field's pane does not get them back");
}

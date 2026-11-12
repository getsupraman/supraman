//! A right-click menu of items carrying the owner's action type. One menu at a time; a click outside it, Esc
//! or choosing an item closes it.

use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
};

use gpui::{AnyElement, App, ClickEvent, Context, Div, FocusHandle, KeyDownEvent, MouseDownEvent, Pixels, Point, SharedString, Window, anchored, canvas, deferred, div, prelude::*, px};
use theme::{ActiveTheme as _, ThemeColors};

use crate::hit::{HitTargets, hit_target};

/// The deferred priority of every layer that floats over its view and is not a menu (a pill group's list).
pub const FLOAT: usize = 1;
/// A side form laid over its view's content, above [`FLOAT`], and what floats inside it (its picker).
pub const FORM: usize = FLOAT + 1;
/// A card opened beside its view's edge over a [`FORM`] there (the ref panel's overlay card).
pub const CARD: usize = FORM + 1;
/// A toast, a palette or a prompt band over a view that may hold a [`FORM`].
pub const OVER_FORM: usize = CARD + 1;
/// Every menu's deferred priority: above every other deferred layer.
pub const PRIORITY: usize = OVER_FORM + 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem<A> {
    /// Unique within the menu; painted under `menu-<key>`.
    pub key: SharedString,
    pub label: String,
    pub action: A,
    /// Starts a section: a rule, then this caption (none when empty), above the item.
    pub section: Option<String>,
    /// Why it cannot act now: listed dimmed with this as its tooltip, its click a no-op.
    pub dimmed: Option<String>,
    /// A tick box before the label, ticked or not.
    pub ticked: Option<bool>,
}

pub fn item<A>(key: impl Into<SharedString>, label: impl Into<String>, action: A) -> MenuItem<A> {
    MenuItem { key: key.into(), label: label.into(), action, section: None, dimmed: None, ticked: None }
}

impl<A> MenuItem<A> {
    pub fn under(self, caption: impl Into<String>) -> Self {
        Self { section: Some(caption.into()), ..self }
    }

    /// Starts a group: a rule above the item.
    pub fn rule(self) -> Self {
        self.under("")
    }

    pub fn dimmed(self, why: impl Into<String>) -> Self {
        Self { dimmed: Some(why.into()), ..self }
    }

    /// A tick box before the label ([`crate::button::tick`]).
    pub fn ticked(self, on: bool) -> Self {
        Self { ticked: Some(on), ..self }
    }
}

/// While shown it holds the focus, given back by [`crate::focus`] when it closes; Esc closes it.
pub struct ContextMenu<A> {
    pub title: String,
    pub position: Point<Pixels>,
    pub items: Vec<MenuItem<A>>,
    focus: OnceCell<FocusHandle>,
    asked: Cell<bool>,
}

impl<A> ContextMenu<A> {
    pub fn new(title: impl Into<String>, position: Point<Pixels>, items: Vec<MenuItem<A>>) -> Self {
        Self { title: title.into(), position, items, focus: OnceCell::new(), asked: Cell::new(false) }
    }

    /// Focuses the menu once, after the frame that first paints it.
    fn take_focus(&self, cx: &mut App) -> impl IntoElement + use<A> {
        let focus = self.focus.get_or_init(|| cx.focus_handle()).clone();
        let ask = !self.asked.replace(true);
        canvas(move |_, window, cx| if ask { window.defer(cx, move |window, cx| window.focus(&focus, cx)) }, |_, _, _, _| {}).absolute().size_0()
    }
}

impl<A: Clone + 'static> ContextMenu<A> {
    /// Over everything ([`PRIORITY`]), kept in the window, under `menu`; `on_pick` and `on_dismiss` (a click outside, Esc) must close it.
    pub fn render<V: 'static>(
        &self,
        hits: &HitTargets,
        on_pick: impl Fn(&mut V, A, &mut Window, &mut Context<V>) + Clone + 'static,
        on_dismiss: impl Fn(&mut V, &mut Context<V>) + 'static,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let taken = self.take_focus(cx);
        let focus = self.focus.get().cloned().expect("take_focus made it");
        let on_dismiss = Rc::new(on_dismiss);
        let on_escape = on_dismiss.clone();
        let mut list = div()
            .id("context-menu")
            .track_focus(&focus)
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    on_escape(this, cx);
                }
            }))
            .relative()
            .flex()
            .flex_col()
            .min_w(px(180.))
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_md()
            .text_sm()
            .occlude()
            .on_mouse_down_out(cx.listener(move |this, _: &MouseDownEvent, _, cx| on_dismiss(this, cx)))
            .child(hit_target(hits, "menu".into()))
            .child(taken)
            .child(caption(&self.title, &colors));
        for entry in &self.items {
            let key = format!("menu-{}", entry.key);
            if let Some(section) = &entry.section {
                list = list.child(div().my_1().h(px(1.)).bg(colors.border_variant)).when(!section.is_empty(), |list| list.child(caption(section, &colors)));
            }
            let tick = entry.ticked.map(|on| crate::button::tick(on, colors.text_accent, cx));
            let row = div().id(SharedString::from(key.clone())).relative().flex().items_center().gap_2().px_3().py(px(3.)).children(tick);
            let row = row.child(SharedString::from(entry.label.clone())).child(hit_target(hits, key));
            list = list.child(match &entry.dimmed {
                Some(why) => row.text_color(colors.text_disabled).tooltip(crate::tooltip::Tooltip::text(why.clone())).on_click(|_, _, cx| cx.stop_propagation()),
                None => {
                    let (action, on_pick) = (entry.action.clone(), on_pick.clone());
                    row.cursor_pointer().hover(|s| s.bg(colors.element_hover)).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        on_pick(this, action.clone(), window, cx);
                    }))
                }
            });
        }
        deferred(anchored().position(self.position).snap_to_window_with_margin(px(8.)).child(list)).with_priority(PRIORITY).into_any_element()
    }
}

fn caption(text: &str, colors: &ThemeColors) -> Div {
    div().px_3().pb_1().text_size(px(10.)).text_color(colors.text_placeholder).overflow_hidden().whitespace_nowrap().child(SharedString::from(text.to_string()))
}

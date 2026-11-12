//! A window's panes: each registers its focus handle as it paints, F6 and Shift+F6 step through them, the one
//! holding the focus wears a 1 px edge in the theme's focus colour, and focus lost with a popup goes back to
//! the pane that held it.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::{
    App, BorderStyle, Bounds, Context, FocusHandle, Global, InteractiveElement, IntoElement, KeyBinding, ParentElement, Pixels, SharedString, Styled as _, Subscription,
    Window, WindowId, canvas, outline,
};
use theme::ActiveTheme as _;

use crate::focus_order;
use crate::hit::{Hit, HitTargets};

gpui::actions!(focus, [NextPane, PreviousPane]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("f6", NextPane, None), KeyBinding::new("shift-f6", PreviousPane, None)]);
}

/// The hit key of a pane's painted mark.
pub fn mark_key(pane: &str) -> String {
    format!("focus-mark-{pane}")
}

/// A pane: `region` contains every focusable thing in it; F6 focuses what it last held there, else `entry`.
#[derive(Clone)]
pub struct Pane {
    key: SharedString,
    region: FocusHandle,
    entry: FocusHandle,
    later: bool,
}

impl Pane {
    pub fn new(key: impl Into<SharedString>, region: &FocusHandle) -> Self {
        Self { key: key.into(), region: region.clone(), entry: region.clone(), later: false }
    }

    /// What F6 focuses in it before anything in it has held the focus.
    pub fn entry(self, entry: &FocusHandle) -> Self {
        Self { entry: entry.clone(), ..self }
    }

    /// Ordered after the window's other panes though painted before some (the diff viewer in the graph's place).
    pub fn later(self) -> Self {
        Self { later: true, ..self }
    }

    /// The last child of the pane's `relative` element: registers the pane as it prepaints and paints its mark.
    pub fn paint(self, hits: &HitTargets) -> impl IntoElement {
        let (pane, hits) = (self.clone(), hits.clone());
        canvas(
            move |_, window, cx| state(window, cx).borrow_mut().register(self),
            move |bounds, _, window, cx| paint_mark(&pane, bounds, &hits, window, cx),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

fn paint_mark(pane: &Pane, bounds: Bounds<Pixels>, hits: &HitTargets, window: &mut Window, cx: &mut App) {
    let key = mark_key(&pane.key);
    if !pane.region.contains_focused(window, cx) {
        hits.borrow_mut().remove(&key);
        return;
    }
    if let Some(leaf) = window.focused(cx) {
        state(window, cx).borrow_mut().held(&pane.key, leaf);
    }
    if !window.is_window_active() {
        hits.borrow_mut().remove(&key);
        return;
    }
    window.paint_quad(outline(bounds, cx.theme().colors().border_focused, BorderStyle::Solid));
    hits.borrow_mut().insert(key, Hit { bounds, visible: window.content_mask().bounds });
}

/// The window's root element: its panes registered afresh each frame from here, F6 and Shift+F6 handled.
pub fn root<E: InteractiveElement + ParentElement>(element: E) -> E {
    let start = canvas(|_, window, cx| state(window, cx).borrow_mut().shown.clear(), |_, _, _, _| {}).absolute().size_0();
    element.child(start).on_action(|_: &NextPane, window, cx| step(false, window, cx)).on_action(|_: &PreviousPane, window, cx| step(true, window, cx))
}

/// The root view's subscription: focus lost (a popup closed with it) goes back to the pane that held it.
pub fn restore_on_loss<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> Subscription {
    cx.on_focus_lost(window, |_, window, cx| {
        restore(window, cx);
    })
}

/// Focuses what the pane that last held the focus held, while that pane is shown, else gpui's nearest focusable
/// ancestor of what lost it; whether it focused anything.
pub fn restore(window: &mut Window, cx: &mut App) -> bool {
    let target = {
        let state = state(window, cx);
        let state = state.borrow();
        state.held.as_ref().and_then(|key| state.shown.iter().find(|p| &p.key == key)).map(|pane| state.target(pane, window))
    };
    target.or_else(|| window.focus_lost_restore_target(cx)).map(|t| window.focus(&t, cx)).is_some()
}

/// The shown panes' keys in F6's order.
pub fn order(window: &mut Window, cx: &mut App) -> Vec<SharedString> {
    state(window, cx).borrow().ordered().into_iter().map(|p| p.key).collect()
}

/// The pane holding the focus.
pub fn focused(window: &mut Window, cx: &mut App) -> Option<SharedString> {
    let panes = state(window, cx).borrow().ordered();
    holding(&panes, window, cx).map(|at| panes[at].key.clone())
}

fn holding(panes: &[Pane], window: &Window, cx: &App) -> Option<usize> {
    panes.iter().rposition(|p| p.region.contains_focused(window, cx))
}

fn step(back: bool, window: &mut Window, cx: &mut App) {
    let target = {
        let state = state(window, cx);
        let state = state.borrow();
        let panes = state.ordered();
        focus_order::stepped(holding(&panes, window, cx), panes.len(), back).map(|to| state.target(&panes[to], window))
    };
    if let Some(target) = target {
        window.focus(&target, cx);
    }
}

#[derive(Default)]
struct State {
    /// This frame's panes in paint order.
    shown: Vec<Pane>,
    /// The pane that last held the focus.
    held: Option<SharedString>,
    /// What each pane last held.
    leaves: HashMap<SharedString, FocusHandle>,
}

impl State {
    fn register(&mut self, pane: Pane) {
        self.shown.retain(|p| p.key != pane.key);
        self.shown.push(pane);
    }

    fn held(&mut self, key: &SharedString, leaf: FocusHandle) {
        self.held = Some(key.clone());
        self.leaves.insert(key.clone(), leaf);
    }

    fn ordered(&self) -> Vec<Pane> {
        ordered(&self.shown, |p| p.later)
    }

    fn target(&self, pane: &Pane, window: &Window) -> FocusHandle {
        self.leaves.get(&pane.key).filter(|leaf| pane.region.contains(leaf, window)).unwrap_or(&pane.entry).clone()
    }
}

/// `shown` with the later ones moved after the rest, each part in paint order.
fn ordered<T: Clone>(shown: &[T], later: impl Fn(&T) -> bool) -> Vec<T> {
    let (mut first, rest): (Vec<T>, Vec<T>) = shown.iter().cloned().partition(|p| !later(p));
    first.extend(rest);
    first
}

#[derive(Default)]
struct Windows(HashMap<WindowId, Rc<RefCell<State>>>);

impl Global for Windows {}

fn state(window: &Window, cx: &mut App) -> Rc<RefCell<State>> {
    let id = window.window_handle().window_id();
    cx.default_global::<Windows>().0.entry(id).or_default().clone()
}

#[cfg(test)]
mod tests;

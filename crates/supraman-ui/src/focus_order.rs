//! A form's Tab order: Tab focuses its next field, Shift+Tab the previous one, wrapping within the form; Space or Enter presses a focused button, Esc on it is `Dismiss` for the form.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{App, FocusHandle, InteractiveElement, KeyBinding, SharedString, Window};

gpui::actions!(focus_order, [NextField, PreviousField, Press, Dismiss]);

/// A form's element: Tab and Shift+Tab are bound here only, so a terminal and every other view keep Tab.
pub const CONTEXT: &str = "FocusOrder";
/// A focused button's element: Space and Enter press it, Esc says `Dismiss` to the form around it.
pub const BUTTON_CONTEXT: &str = "FocusedButton";

/// `el` taking the keyboard under `focus` (a [`FocusOrder`] member): [`crate::button::ring`]ed in the focus colour while
/// focused, over any other ring; Space or Enter `press`. A [`crate::button::Button`]'s focus, and a field that opens a picker.
pub fn pressable(el: gpui::Stateful<gpui::Div>, focus: &FocusHandle, press: impl Fn(&Press, &mut Window, &mut App) + 'static, cx: &App) -> gpui::Stateful<gpui::Div> {
    use gpui::Styled as _;
    use theme::ActiveTheme as _;
    let focused = crate::button::ring(cx.theme().colors().border_focused);
    el.track_focus(focus).key_context(BUTTON_CONTEXT).on_action(press).focus(move |s| s.shadow(vec![focused]))
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextField, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PreviousField, Some(CONTEXT)),
        KeyBinding::new("space", Press, Some(BUTTON_CONTEXT)),
        KeyBinding::new("enter", Press, Some(BUTTON_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(BUTTON_CONTEXT)),
    ]);
}

/// A form's fields and buttons in Tab order, by their focus handles.
#[derive(Clone, Default)]
pub struct FocusOrder(Vec<FocusHandle>);

impl FocusOrder {
    pub fn new(handles: impl IntoIterator<Item = FocusHandle>) -> Self {
        Self(handles.into_iter().collect())
    }

    pub fn handles(&self) -> &[FocusHandle] {
        &self.0
    }

    pub fn first(&self) -> Option<FocusHandle> {
        self.0.first().cloned()
    }

    /// The member holding the focus: the last one containing it, so a field's own popup counts as the field.
    pub fn focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.0.iter().rposition(|h| h.contains_focused(window, cx))
    }

    /// The next member (the previous one with `back`) focused, wrapping; the first (the last) while none holds the focus.
    pub fn step(&self, back: bool, window: &mut Window, cx: &mut App) {
        if let Some(to) = stepped(self.focused(window, cx), self.0.len(), back) {
            self.0[to].focus(window, cx);
        }
    }

    /// `form` with Tab and Shift+Tab stepping through this order.
    pub fn on<E: InteractiveElement>(self, form: E) -> E {
        let back = self.clone();
        form.key_context(CONTEXT)
            .on_action(move |_: &NextField, window, cx| self.step(false, window, cx))
            .on_action(move |_: &PreviousField, window, cx| back.step(true, window, cx))
    }
}

/// A view's focusable buttons by key, each handle kept across frames, and its Tab order: the members in the order
/// [`Members::handle`] and [`Members::add`] were called since [`Members::begin`], its render's paint order.
#[derive(Default)]
pub struct Members {
    handles: RefCell<HashMap<SharedString, FocusHandle>>,
    order: RefCell<Vec<FocusHandle>>,
}

impl Members {
    /// A new frame's order, before its first member.
    pub fn begin(&self) {
        self.order.borrow_mut().clear();
    }

    /// The handle kept for `key`, made on its first call; the next member of this frame's order.
    pub fn handle(&self, key: impl Into<SharedString>, cx: &App) -> FocusHandle {
        let handle = self.handles.borrow_mut().entry(key.into()).or_insert_with(|| cx.focus_handle()).clone();
        self.add(handle.clone());
        handle
    }

    /// `handle` (a field's) as the next member of this frame's order.
    pub fn add(&self, handle: FocusHandle) {
        self.order.borrow_mut().push(handle);
    }

    /// The handle kept for `key`, if one was made.
    pub fn get(&self, key: &str) -> Option<FocusHandle> {
        self.handles.borrow().get(key).cloned()
    }

    /// The key of the kept handle holding the focus.
    pub fn focused(&self, window: &Window, cx: &App) -> Option<SharedString> {
        self.handles.borrow().iter().find(|(_, h)| h.contains_focused(window, cx)).map(|(k, _)| k.clone())
    }

    /// This frame's members in order.
    pub fn order(&self) -> FocusOrder {
        FocusOrder::new(self.order.borrow().iter().cloned())
    }
}

/// The member after `at` of `n` (before it with `back`), wrapping; the first (the last) from none; `None` of none.
pub fn stepped(at: Option<usize>, n: usize, back: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match at {
        Some(at) if back => (at + n - 1) % n,
        Some(at) => (at + 1) % n,
        None if back => n - 1,
        None => 0,
    })
}

#[cfg(test)]
mod tests;

//! A form's Tab order: Tab focuses its next field, Shift+Tab the previous one, wrapping within the form; Space or Enter presses a focused button.

use gpui::{App, FocusHandle, InteractiveElement, KeyBinding, Window};

gpui::actions!(focus_order, [NextField, PreviousField, Press]);

/// A form's element: Tab and Shift+Tab are bound here only, so a terminal and every other view keep Tab.
pub const CONTEXT: &str = "FocusOrder";
/// A focused button's element: Space and Enter press it.
pub const BUTTON_CONTEXT: &str = "FocusedButton";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextField, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PreviousField, Some(CONTEXT)),
        KeyBinding::new("space", Press, Some(BUTTON_CONTEXT)),
        KeyBinding::new("enter", Press, Some(BUTTON_CONTEXT)),
    ]);
}

/// A form's fields and buttons in Tab order, by their focus handles.
#[derive(Clone, Default)]
pub struct FocusOrder(Vec<FocusHandle>);

impl FocusOrder {
    pub fn new(handles: impl IntoIterator<Item = FocusHandle>) -> Self {
        Self(handles.into_iter().collect())
    }

    /// The member holding the focus: the last one containing it, so a field's own popup counts as the field.
    pub fn focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.0.iter().rposition(|h| h.contains_focused(window, cx))
    }

    /// The next member (the previous one with `back`) focused, wrapping; the first (the last) while none holds the focus.
    pub fn step(&self, back: bool, window: &mut Window, cx: &mut App) {
        let n = self.0.len();
        if n == 0 {
            return;
        }
        let to = match self.focused(window, cx) {
            Some(at) if back => (at + n - 1) % n,
            Some(at) => (at + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        self.0[to].focus(window, cx);
    }

    /// `form` with Tab and Shift+Tab stepping through this order.
    pub fn on<E: InteractiveElement>(self, form: E) -> E {
        let back = self.clone();
        form.key_context(CONTEXT)
            .on_action(move |_: &NextField, window, cx| self.step(false, window, cx))
            .on_action(move |_: &PreviousField, window, cx| back.step(true, window, cx))
    }
}

#[cfg(test)]
mod tests;

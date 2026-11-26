//! Read-only text the user selects and copies (UI-KIT §4.5): a read-only, plain [`TextInput`] kept per element id.

use gpui::{App, ElementId, Entity, IntoElement, RenderOnce, SharedString, Window, div, prelude::*};

use super::TextInput;

/// `text` laid out as a plain [`TextInput`] in the style it sits in, wrapped: mouse and Shift+arrow selection, Ctrl+C.
#[derive(IntoElement)]
pub struct TextView {
    id: ElementId,
    text: SharedString,
    wrap: bool,
}

impl TextView {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self { id: id.into(), text: text.into(), wrap: true }
    }

    /// One line, cut by its box rather than wrapped.
    pub fn one_line(mut self) -> Self {
        self.wrap = false;
        self
    }

    /// The input behind the element with `id`, made on its first paint.
    fn input(id: impl Into<ElementId>, wrap: bool, window: &mut Window, cx: &mut App) -> Entity<TextInput> {
        window.use_keyed_state(id, cx, move |window, cx| {
            let input = TextInput::new("", None, window, cx).plain().read_only();
            if wrap { input.multi_line() } else { input }
        })
    }
}

impl RenderOnce for TextView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let input = Self::input(self.id, self.wrap, window, cx);
        input.update(cx, |input, cx| {
            if input.text() != self.text.as_ref() {
                input.set_text(&self.text, cx);
            }
        });
        div().w_full().min_w_0().child(input)
    }
}

#[cfg(test)]
mod tests;

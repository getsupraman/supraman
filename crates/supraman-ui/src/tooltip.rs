use gpui::{AnyView, App, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

/// A plain text tooltip: `.tooltip(Tooltip::text(..))`; [`LAST_SHOWN`] is the text it last showed.
pub struct Tooltip(SharedString);

thread_local! {
    pub static LAST_SHOWN: std::cell::RefCell<Option<SharedString>> = const { std::cell::RefCell::new(None) };
}

impl Tooltip {
    pub fn text(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let text = text.into();
        move |_, cx| {
            LAST_SHOWN.with(|last| *last.borrow_mut() = Some(text.clone()));
            cx.new(|_| Tooltip(text.clone())).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        div().pl_2().pt_2().child(
            div()
                .max_w(px(320.))
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .text_xs()
                .font_family(crate::fonts::UI)
                .text_color(colors.text)
                .shadow_md()
                .child(self.0.clone()),
        )
    }
}

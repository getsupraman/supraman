//! A hover card as Edge's tab hover shows one: a title, then one `label  value` line per row.

use std::cell::RefCell;

use gpui::{AnyView, App, SharedString, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverCard {
    pub title: String,
    pub rows: Vec<(&'static str, String)>,
}

impl HoverCard {
    /// The title, then one `label  value` line per row. The selftest compares this.
    pub fn text(&self) -> String {
        let mut text = self.title.clone();
        for (label, value) in &self.rows {
            text.push_str(&format!("\n{label}  {value}"));
        }
        text
    }
}

thread_local! {
    /// The card shown last, as [`HoverCard::text`]; the selftest reads it.
    pub static LAST_CARD: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// A tooltip builder: the card is put together when it is shown, so it is current.
pub fn tooltip(card: impl Fn() -> HoverCard + 'static) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        let card = card();
        LAST_CARD.with(|last| *last.borrow_mut() = Some(card.text()));
        cx.new(|_| CardView(card)).into()
    }
}

struct CardView(HoverCard);

impl Render for CardView {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let quiet = colors.text.opacity(0.62);
        let rows = self.0.rows.iter().map(|(label, value)| {
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().w(px(64.)).text_color(quiet).child(*label))
                .child(div().min_w_0().child(SharedString::from(value.clone())))
        });
        div().pl_2().pt_2().child(
            div()
                .w(px(360.))
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .text_xs()
                .font_family(crate::fonts::UI)
                .text_color(colors.text)
                .shadow_md()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().pb_1().child(SharedString::from(self.0.title.clone())))
                .children(rows),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_the_title_then_a_line_per_row() {
        let card = HoverCard { title: "t".into(), rows: vec![("kind", "shell".into()), ("repo", "x".into())] };
        assert_eq!(card.text(), "t\nkind  shell\nrepo  x");
        assert_eq!(HoverCard { title: "t".into(), rows: vec![] }.text(), "t");
    }
}

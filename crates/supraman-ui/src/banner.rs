//! The notice box: text in a status colour on a tint of that colour, never the theme's status
//! `*_background` fill.

use gpui::{App, Div, Hsla, IntoElement, div, prelude::*};
use theme::ActiveTheme as _;

pub const TINT: f32 = 0.12;

pub fn tint(color: Hsla) -> Hsla {
    color.opacity(TINT)
}

/// `color` text on its [`tint`]; the caller places it and adds the body.
pub fn tinted(color: Hsla) -> Div {
    filled(color, tint(color))
}

/// The text colour on the element fill: a notice that is not a problem.
pub fn plain(cx: &App) -> Div {
    let colors = cx.theme().colors();
    filled(colors.text, colors.element_background)
}

fn filled(fg: Hsla, bg: Hsla) -> Div {
    div().p_2().rounded_md().bg(bg).text_sm().text_color(fg)
}

/// `body` beside a dismiss button at the top right.
pub fn dismissable(body: impl IntoElement, dismiss: impl IntoElement) -> Div {
    div().flex().items_start().gap_2().child(div().flex_1().min_w_0().child(body)).child(dismiss)
}

/// `mark` (an icon, a logo) beside `text` that wraps in the room the mark leaves.
pub fn marked(mark: impl IntoElement, text: impl IntoElement) -> Div {
    div().flex().items_start().gap_2().child(mark).child(div().flex_1().min_w_0().child(text))
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Render, SharedString, TestAppContext, Window, px};

    use super::*;
    use crate::hit::{HitTargets, hit_target};

    struct Notice(HitTargets);

    impl Render for Notice {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let words = "Not signed in to Workman at https://work.supraman.ai, so sessions run without Workman.";
            let text = div().relative().child(SharedString::from(words)).child(hit_target(&self.0, "title".into()));
            let close = div().relative().size(px(16.)).flex_none().child(hit_target(&self.0, "close".into()));
            div().w(px(220.)).child(tinted(gpui::red()).child(dismissable(marked(div().size(px(16.)).flex_none(), text), close)))
        }
    }

    #[gpui::test]
    fn a_long_title_beside_a_mark_wraps_short_of_the_close_button(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let hits = HitTargets::default();
        let (_, cx) = cx.add_window_view(|_, _| Notice(hits.clone()));
        cx.run_until_parked();
        let (title, close) = (hits.borrow()["title"].bounds, hits.borrow()["close"].bounds);
        assert!(title.right() <= close.left(), "the title stops before the ×: {title:?} {close:?}");
        assert!(title.size.height > close.size.height * 2., "it wraps onto more lines: {title:?}");
    }
}

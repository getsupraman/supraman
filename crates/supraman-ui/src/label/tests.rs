use gpui::{Context, Entity, Pixels, Render, TestAppContext, VisualTestContext, Window, px};

use super::*;
use crate::hit::HitTargets;

struct Labels {
    hits: HitTargets,
}

impl Render for Labels {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let green = cx.theme().status().success;
        let hits = &self.hits;
        div()
            .flex()
            .flex_col()
            .gap(tokens::space::L)
            .p(tokens::space::L)
            .child(Label::new("A dot").mark(Mark::Filled(green)).size(tokens::text::SMALL).hit(hits, "dot"))
            .child(Label::new("An icon").mark(Icon::Folder).size(tokens::text::BODY).hit(hits, "icon"))
            .child(Label::new("A tick").mark(LabelMark::Tick(true)).size(tokens::text::BODY).hit(hits, "tick"))
            .child(Label::new("In a slot").mark(Icon::Folder).slot(px(24.)).size(tokens::text::SMALL).hit(hits, "slot"))
            .child(Label::new("A label cut short of its room").mark(Icon::Star).clipped("cut-label", false).size(tokens::text::SMALL).w(px(80.)).hit(hits, "cut"))
            .child(div().h(px(40.)).flex().items_center().child(Label::new("Centred").mark(Icon::Play).centred().size(tokens::text::BODY).hit(hits, "centred")))
    }
}

fn centre(view: &Entity<Labels>, key: &str, cx: &mut VisualTestContext) -> Pixels {
    view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds.center().y)).unwrap_or_else(|| panic!("{key} paints"))
}

fn below(size: tokens::Text, cx: &mut VisualTestContext) -> Pixels {
    cx.update(|window, _| {
        let mut style = window.text_style();
        style.font_size = size.size.into();
        crate::ink::offset(&style, window)
    })
}

#[gpui::test]
fn a_dot_an_icon_and_a_tick_sit_on_the_letters_middle(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Labels { hits: HitTargets::default() });
    cx.run_until_parked();
    for (key, size) in [("dot", tokens::text::SMALL), ("icon", tokens::text::BODY), ("tick", tokens::text::BODY), ("slot", tokens::text::SMALL), ("cut", tokens::text::SMALL), ("centred", tokens::text::BODY)] {
        let (mark, text, below) = (centre(&view, &format!("{key}-mark"), cx), centre(&view, &format!("{key}-text"), cx), below(size, cx));
        assert!(below > px(0.), "the letters' middle is below the line's");
        assert!(crate::ink::level(mark, text + below), "{key}: the mark on the letters' middle: mark {mark:?}, line {text:?}, letters {below:?} below it");
    }
}

#[gpui::test]
fn a_centred_label_puts_its_letters_and_mark_on_its_boxs_middle(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Labels { hits: HitTargets::default() });
    cx.run_until_parked();
    let mark = view.read_with(cx, |v, _| v.hits.borrow()["centred-mark"].bounds);
    let text = view.read_with(cx, |v, _| v.hits.borrow()["centred-text"].bounds);
    let below = below(tokens::text::BODY, cx);
    assert!(crate::ink::level(text.center().y + below, mark.center().y), "letters and mark on one middle");
    assert!(text.center().y < mark.center().y, "the letters moved up, not the mark down");
}

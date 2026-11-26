//! The drag and drop section's samples: a row of cards to reorder, a drop zone, the chip.

use gpui::{AnyElement, App, Axis, Context, SharedString, div, prelude::*, rems};
use theme::ActiveTheme as _;

use super::Gallery;
use super::sections::{group, row};
use crate::dnd::{self, DropZone, Payload, Sortable, lands};
use crate::hit::hit_target;
use crate::tokens;

pub(super) const CARDS: usize = 4;

#[derive(Clone)]
struct Card(usize);

impl Payload for Card {
    fn title(&self) -> SharedString {
        format!("Card {}", self.0 + 1).into()
    }
}

fn cards() -> Sortable<Card> {
    Sortable::new("kit-dnd", Axis::Horizontal)
}

pub(super) fn section(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let boxed = |key: String| {
        div()
            .id(SharedString::from(key.clone()))
            .flex()
            .items_center()
            .justify_center()
            .h(tokens::control::LARGE.height)
            .rounded(tokens::radius::S)
            .border(tokens::stroke::HAIRLINE)
            .border_color(colors.border)
            .bg(colors.panel_background)
            .text_size(tokens::text::BODY)
            .child(hit_target(&g.hits, key))
    };
    let order = g.cards.clone();
    let mut list = row().gap(tokens::space::L);
    for (ix, &card) in order.iter().enumerate() {
        let key = format!("kit-dnd-{card}");
        let from = order.clone();
        let lands = move |c: &Card, _: &App| from.iter().position(|&n| n == c.0).and_then(|at| lands(at, ix));
        let dropped = cx.listener(move |g: &mut Gallery, c: &Card, _, cx| {
            if let Some(at) = g.cards.iter().position(|&n| n == c.0) {
                crate::tabs::reorder(&mut g.cards, at, ix);
                cx.notify();
            }
        });
        let item = boxed(key.clone()).w(rems(7.)).child(Card(card).title());
        let item = cards().item(item, key.clone(), lands, dropped, cx);
        list = list.child(dnd::source(item, key, Card(card), cx));
    }
    let zone = DropZone::new("kit-dnd-zone", boxed("kit-dnd-zone".into()).w(rems(30.)).h(rems(5.)).text_color(colors.text_muted).child("Drop a card here"))
        .take::<Card>(|_, _, _| true, |_, _, _| {})
        .element();
    let chip = dnd::chip("A dragged row's chip".into(), cx);
    div()
        .flex()
        .flex_col()
        .gap(tokens::space::XXL)
        .child(group("dnd::Sortable: drag a card along the row", cx).child(list))
        .child(group("dnd::DropZone", cx).child(zone))
        .child(group("dnd::chip", cx).child(row().child(chip)))
        .into_any_element()
}

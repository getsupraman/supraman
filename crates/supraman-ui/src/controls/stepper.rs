//! The one − n + control.

use gpui::{Context, Div, Pixels, SharedString, Window, div, prelude::*, px};

use crate::controls::button::{Button, Size};
use crate::focus_order::Members;
use crate::hit::{HitTargets, hit_target};
use crate::label::Label;

/// − `value` +: two [`Size::Medium`] buttons of `members`' Tab order under `<key>-dec` and `<key>-inc`, `step` given -1 or 1
/// by a click, Space or Enter; the value between them bold, its letters on theirs, under `<key>-value`.
pub fn stepper<V: 'static>(
    hits: &HitTargets,
    key: &str,
    value: impl Into<SharedString>,
    members: &Members,
    step: impl Fn(&mut V, i32, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Div {
    let mut one = |by: i32, part: &str, words: &'static str| {
        let key = format!("{key}-{part}");
        let step = step.clone();
        let focus = members.handle(key.clone(), cx);
        Button::new(hits, key).label(words).size(Size::Medium).acting(&focus, move |this, window, cx| step(this, by, window, cx), cx)
    };
    let (dec, inc) = (one(-1, "dec", "−"), one(1, "inc", "+"));
    let value = Label::element(div().relative().child(value.into()).child(hit_target(hits, format!("{key}-value")))).centred();
    let value = div().min_w(STEPPER_VALUE).flex().justify_center().text_size(Size::Medium.control().text.size).font_weight(gpui::FontWeight::BOLD).child(value);
    Button::line().child(dec).child(value).child(inc)
}

/// The least width of a stepper's value, so a step from 9 to 10 moves neither button.
const STEPPER_VALUE: Pixels = px(28.);

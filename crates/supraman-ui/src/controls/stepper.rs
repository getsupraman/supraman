//! The one − n + control.

use gpui::{ClickEvent, Context, Div, Pixels, SharedString, Window, div, prelude::*, px};

use crate::controls::button::{Button, Size};
use crate::focus_order::Members;
use crate::hit::{HitTargets, hit_target};
use crate::label::Label;

/// − `value` +: two [`Size::Medium`] buttons under `<key>-dec` and `<key>-inc`, `step` given -1 or 1 by a click, and by Space
/// or Enter while in `members`' Tab order (none: a bar's, clicked only); the value between them bold, its letters on theirs, under `<key>-value`.
pub fn stepper<'a, V: 'static>(
    hits: &HitTargets,
    key: &str,
    value: impl Into<SharedString>,
    members: impl Into<Option<&'a Members>>,
    step: impl Fn(&mut V, i32, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Div {
    let members = members.into();
    let mut one = |by: i32, part: &str, words: &'static str| {
        let key = format!("{key}-{part}");
        let step = step.clone();
        let button = Button::new(hits, key.clone()).label(words).size(Size::Medium);
        match members {
            Some(members) => {
                let focus = members.handle(key, cx);
                button.acting(&focus, move |this, window, cx| step(this, by, window, cx), cx)
            }
            None => button.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| step(this, by, window, cx))),
        }
    };
    let (dec, inc) = (one(-1, "dec", "−"), one(1, "inc", "+"));
    let value = Label::element(div().relative().child(value.into()).child(hit_target(hits, format!("{key}-value")))).centred();
    let value = div().min_w(STEPPER_VALUE).flex().justify_center().text_size(Size::Medium.control().text.size).font_weight(gpui::FontWeight::BOLD).child(value);
    Button::line().child(dec).child(value).child(inc)
}

/// The least width of a stepper's value, so a step from 9 to 10 moves neither button.
const STEPPER_VALUE: Pixels = px(28.);

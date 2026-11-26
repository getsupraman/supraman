//! The buttons ([`Button`], [`ButtonSplit`]), the tick mark, choices and pills. Each takes `key` as its element id, its hover group and its [`hit_target`] key.

use gpui::{App, Context, Div, FocusHandle, Hsla, Pixels, SharedString, Stateful, StyleRefinement, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::focus_order::Press;
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::label::Label;
use crate::tokens::{self, Control, radius, space, stroke};

mod build;

pub use build::{Button, ButtonSplit, Glyph};

fn base(hits: &HitTargets, key: SharedString, hover: impl FnOnce(StyleRefinement) -> StyleRefinement, active: Hsla) -> Stateful<Div> {
    keyed(hits, RowKeys::one(key), hover, active)
}

fn keyed(hits: &HitTargets, keys: RowKeys, hover: impl FnOnce(StyleRefinement) -> StyleRefinement, active: Hsla) -> Stateful<Div> {
    div()
        .id(keys.id)
        .group(keys.group)
        .relative()
        .cursor_pointer()
        .hover(hover)
        .active(|s| s.bg(active))
        .child(hit_target(hits, keys.hit.to_string()))
}

/// Borderless, the ghost fill on hover, a deeper one pressed; the caller lays it out.
pub fn ghost(hits: &HitTargets, key: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    base(hits, key.into(), |s| s.bg(colors.ghost_element_hover), colors.ghost_element_active)
}

/// A [`ghost`] list row; while it is the selected one it is filled `selected` and its hover is off.
pub fn row(hits: &HitTargets, key: impl Into<SharedString>, selected: Option<Hsla>, cx: &App) -> Stateful<Div> {
    row_in(hits, RowKeys::one(key.into()), selected, cx)
}

/// A [`row`] whose element id, hover group and hit key differ, on [`crate::list::row`]: open while a popup anchored to its hit key is.
pub fn row_in(hits: &HitTargets, keys: RowKeys, selected: Option<Hsla>, cx: &App) -> Stateful<Div> {
    let chrome = crate::list::Chrome::new().selected(selected.is_some());
    crate::list::row(hits, keys, selected.map_or(chrome, |fill| chrome.chosen(fill)), cx)
}

/// A [`row_in`]'s element id, hover group and hit key.
pub struct RowKeys {
    pub id: SharedString,
    pub group: SharedString,
    pub hit: SharedString,
}

impl RowKeys {
    pub fn one(key: SharedString) -> Self {
        Self { id: key.clone(), group: key.clone(), hit: key }
    }
}

/// A toned [`Button::icon_only`]'s colours: Neutral [`look`]'s Solid; a status tone its Outline tinted at rest and its Solid hovered.
pub(crate) fn icon_look(tone: Tone, cx: &App) -> Look {
    let solid = look(tone, Fill::Solid, cx);
    if tone == Tone::Neutral {
        return solid;
    }
    let outline = look(tone, Fill::Outline, cx);
    Look { fill: outline.hover, border: outline.border, text: outline.text, hover: solid.fill, hover_text: solid.text, active: solid.hover }
}

/// What a labelled [`Button`] does: Danger loses work or cannot be taken back, Success is GitMan's safe forward action, Neutral the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Success,
    Danger,
}

/// Solid, the default; every answer of a prompt is Solid (`confirm::answer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Solid,
    Outline,
}

/// A labelled button's size: one of [`tokens::control`]'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// A row's, a panel's or a toolbar's button, in the 12 px text beside it.
    Small,
    /// A form's, a question's and a settings page's, as high as a one-line field, in the 14 px body text.
    Medium,
}

impl Size {
    pub const ALL: [Size; 2] = [Size::Small, Size::Medium];

    pub fn control(self) -> Control {
        match self {
            Size::Small => tokens::control::SMALL,
            Size::Medium => tokens::control::MEDIUM,
        }
    }

    /// Height, side padding and text size.
    pub fn table(self) -> (Pixels, Pixels, gpui::Rems) {
        let c = self.control();
        (c.height, c.padding, c.text.size)
    }

    pub fn height(self) -> Pixels {
        self.control().height
    }
}

/// The one gap between the buttons of a row (a question's answers, a form's, a settings control's).
pub const GAP: Pixels = space::L;

/// How far a ring (the focus's, a default answer's) stands out of its button.
pub const RING: Pixels = stroke::FOCUS;

/// A [`Button`]'s colours: its fill, border and text, its fill and text hovered, and its fill pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    pub fill: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub hover: Hsla,
    pub hover_text: Hsla,
    pub active: Hsla,
}

/// The colours of `tone` in `fill`: success `status().success`, danger `status().error`, neutral the element grey.
pub(crate) fn look(tone: Tone, fill: Fill, cx: &App) -> Look {
    let colors = cx.theme().colors();
    let tone = match tone {
        Tone::Neutral => None,
        Tone::Success => Some(cx.theme().status().success),
        Tone::Danger => Some(cx.theme().status().error),
    };
    let clear = gpui::transparent_black();
    match (tone, fill) {
        (None, Fill::Solid) => Look { fill: colors.element_background, border: colors.border, text: colors.text, hover: colors.element_hover, hover_text: colors.text, active: colors.element_active },
        (None, Fill::Outline) => Look { fill: clear, border: colors.border, text: colors.text, hover: colors.element_hover, hover_text: colors.text, active: colors.element_active },
        (Some(tone), Fill::Solid) => Look { fill: tone, border: tone, text: crate::themes::on(tone), hover: tone.opacity(SOLID_HOVER), hover_text: crate::themes::on(tone), active: tone.opacity(SOLID_ACTIVE) },
        (Some(tone), Fill::Outline) => Look { fill: clear, border: tone, text: tone, hover: tone.opacity(TINT), hover_text: tone, active: tone.opacity(TINT_HOVER) },
    }
}

const TINT: f32 = 0.2;
const TINT_HOVER: f32 = 0.3;
const SOLID_HOVER: f32 = 0.85;
const SOLID_ACTIVE: f32 = 0.7;

/// − `value` +: two [`Size::Medium`] buttons of `members`' Tab order under `<key>-dec` and `<key>-inc`, `step` given -1 or 1
/// by a click, Space or Enter; the value between them bold, its letters on theirs, under `<key>-value`.
pub fn stepper<V: 'static>(
    hits: &HitTargets,
    key: &str,
    value: impl Into<SharedString>,
    members: &crate::focus_order::Members,
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

/// A quiet outlined pill in `fg`, lit on hover; its label's letters centred, painted under `<key>-label`.
pub fn pill(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, fg: Hsla, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    let key = key.into();
    let label = Label::element(div().relative().child(label.into()).child(hit_target(hits, format!("{key}-label")))).centred();
    base(hits, key, |s| s.bg(colors.element_hover).border_color(colors.border).text_color(colors.text), colors.element_active)
        .flex()
        .flex_none()
        .items_center()
        .h(tokens::control::SMALL.height)
        .px(space::L)
        .rounded(radius::S)
        .border(stroke::HAIRLINE)
        .border_color(colors.border_variant)
        .text_size(tokens::text::SMALL)
        .text_color(fg)
        .child(label)
}

/// A [`TICK`] square, checked and filled with `fill` while `on`, the same size either way; a read-only mark on its own.
pub fn tick(on: bool, fill: Hsla, cx: &App) -> Div {
    let colors = cx.theme().colors();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(TICK)
        .rounded(radius::S)
        .border(stroke::HAIRLINE)
        .border_color(if on { fill } else { colors.border })
        .when(on, |d| d.bg(fill).child(Icon::Check.svg(tokens::icon::XS, gpui::white())))
}

/// One option of a choice under `key`, its hit key too; the selected one is drawn as a selected list row.
pub fn choice(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, selected: bool, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    chosen(div().id(key.clone()), selected, cx)
        .relative()
        .px(space::L)
        .py(space::XS)
        .rounded(radius::S)
        .border(stroke::HAIRLINE)
        .border_color(if selected { colors.border_focused } else { colors.border })
        .text_size(tokens::text::BODY)
        .child(Label::new(label).centred().flex_1())
        .child(hit_target(hits, key.to_string()))
}

/// `list`, a row or column of `count` [`choice`]s, one keyboard member under `focus`: one [`crate::focus::ring`] around
/// the group while the focus came by keyboard, an arrow key picking the choice before or after `chosen` (`pick` its index), stopping at the ends.
pub fn choices<V: 'static>(list: Div, focus: &FocusHandle, count: usize, chosen: Option<usize>, pick: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static, cx: &mut Context<V>) -> Div {
    list.relative().track_focus(focus).rounded(radius::S).child(crate::focus::ring(focus, radius::S)).on_key_down(cx.listener(move |this, e: &gpui::KeyDownEvent, window, cx| {
        let back = match e.keystroke.key.as_str() {
            "up" | "left" => true,
            "down" | "right" => false,
            _ => return,
        };
        if e.keystroke.modifiers.modified() {
            return;
        }
        cx.stop_propagation();
        if let Some(to) = neighbour(chosen, count, back) {
            pick(this, to, window, cx);
        }
    }))
}

/// The choice before `at` of `n` (after it unless `back`), `None` past an end; the first (the last) from none.
pub fn neighbour(at: Option<usize>, n: usize, back: bool) -> Option<usize> {
    match at {
        _ if n == 0 => None,
        None if back => Some(n - 1),
        None => Some(0),
        Some(at) if back => at.checked_sub(1),
        Some(at) => (at + 1 < n).then_some(at + 1),
    }
}

/// One option of a [`crate::segmented`] group: [`choice`]'s fills, `icon` and `label`, under `key`.
pub fn segment(hits: &HitTargets, key: impl Into<SharedString>, icon: Option<Icon>, label: Option<SharedString>, selected: bool, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    let fg = if selected { colors.text } else { colors.text_muted };
    let label = match label {
        Some(words) => Label::new(words),
        None => Label::none(),
    };
    chosen(div().id(key.clone()), selected, cx)
        .relative()
        .flex_none()
        .h_full()
        .px(space::L)
        .child(label.mark_if(icon).size(tokens::text::SMALL).color(fg).gap(space::S).centred())
        .child(hit_target(hits, key.to_string()))
}

fn chosen(d: Stateful<Div>, selected: bool, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    d.flex()
        .items_center()
        .cursor_pointer()
        .when(selected, |d| d.bg(colors.element_selected))
        .when(!selected, |d| d.bg(colors.element_background).hover(|s| s.bg(colors.element_hover)))
}

/// A tick box's side.
pub const TICK: Pixels = px(14.);

#[cfg(test)]
mod tests;


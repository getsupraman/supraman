//! The buttons. Each takes `key` as its element id, its hover group and its [`hit_target`] key.

use std::rc::Rc;

use gpui::{
    App, BoxShadow, Context, Div, ElementId, FocusHandle, Hsla, Pixels, Rems, SharedString, Stateful, StyleRefinement, Svg, Window, div,
    point, prelude::*, px, rems,
};
use theme::ActiveTheme as _;

use crate::focus_order::{BUTTON_CONTEXT, Press};
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::ink;

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

/// A [`row`] whose element id, hover group and hit key differ (the sidebar's session row).
pub fn row_in(hits: &HitTargets, keys: RowKeys, selected: Option<Hsla>, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    match selected {
        Some(fill) => keyed(hits, keys, move |s| s.bg(fill), colors.ghost_element_active).bg(fill),
        None => keyed(hits, keys, |s| s.bg(colors.ghost_element_hover), colors.ghost_element_active),
    }
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

/// A square ghost button around `icon`, which takes the text colour on hover.
pub fn icon(hits: &HitTargets, key: impl Into<SharedString>, icon: Svg, side: Pixels, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let text = cx.theme().colors().text;
    ghost(hits, key.clone(), cx)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(side)
        .rounded_sm()
        .child(icon.group_hover(key, |s| s.text_color(text)))
}

/// A square icon button in [`icon_look`]'s colours of `tone`; the icon keeps its own colour and takes the hover's text colour while hovered.
pub fn icon_toned(hits: &HitTargets, key: impl Into<SharedString>, icon: Svg, side: Pixels, tone: Tone, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let look = icon_look(tone, cx);
    base(hits, key.clone(), move |s| s.bg(look.hover), look.active)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(side)
        .rounded_sm()
        .border_1()
        .border_color(look.border)
        .bg(look.fill)
        .child(icon.group_hover(key, move |s| s.text_color(look.hover_text)))
}

/// An [`icon_toned`]'s colours: Neutral [`look`]'s Solid; a status tone its Outline tinted at rest and its Solid hovered.
pub fn icon_look(tone: Tone, cx: &App) -> Look {
    let solid = look(tone, Fill::Solid, cx);
    if tone == Tone::Neutral {
        return solid;
    }
    let outline = look(tone, Fill::Outline, cx);
    Look { fill: outline.hover, border: outline.border, text: outline.text, hover: solid.fill, hover_text: solid.text, active: solid.hover }
}

/// An [`icon`] always laid out and painted (slot and hit key kept), shown while `group` is hovered or `shown`.
pub fn revealed(hits: &HitTargets, key: impl Into<SharedString>, icon: Svg, side: Pixels, group: impl Into<SharedString>, shown: bool, cx: &App) -> Stateful<Div> {
    reveal(self::icon(hits, key, icon, side, cx), group, shown)
}

/// Any `button` shown as [`revealed`] shows its icon: always laid out, visible while `group` is hovered or `shown`.
pub fn reveal(button: Stateful<Div>, group: impl Into<SharedString>, shown: bool) -> Stateful<Div> {
    button.when(!shown, |d| d.opacity(0.).group_hover(group, |s| s.opacity(1.)))
}

/// An action bar button: `icon` in `fg`, then any label; the selected fill while `on`.
pub fn toolbar(hits: &HitTargets, key: impl Into<SharedString>, icon: Svg, on: bool, fg: Hsla, cx: &App) -> Stateful<Div> {
    toolbar_turning(hits, key, icon, on, false, fg, cx)
}

/// [`toolbar`] whose icon turns once a second while `turning`; it then also paints under `<key>-turning`.
pub fn toolbar_turning(hits: &HitTargets, key: impl Into<SharedString>, icon: Svg, on: bool, turning: bool, fg: Hsla, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    let icon = icon.group_hover(key.clone(), |s| s.text_color(colors.text));
    let frame = base(hits, key.clone(), |s| s.bg(colors.ghost_element_hover).text_color(colors.text), colors.ghost_element_active)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_1()
        .h(px(26.))
        .min_w(px(26.))
        .px(px(5.))
        .rounded_md()
        .text_xs()
        .text_color(fg)
        .when(on, |d| d.bg(colors.ghost_element_selected));
    if !turning {
        return frame.child(icon);
    }
    frame.child(hit_target(hits, format!("{key}-turning"))).child(crate::icon::turning(icon, ElementId::Name(format!("{key}-turning").into())))
}

/// A split button: `main` and its `menu` half (a ⌄), each a [`toolbar`] under its own key, joined in one group by a rule.
pub fn split(main: Stateful<Div>, menu: Stateful<Div>, cx: &App) -> Div {
    let rule = cx.theme().colors().border;
    div()
        .flex()
        .flex_none()
        .items_center()
        .child(main.rounded_r_none())
        .child(div().flex_none().w(px(1.)).h(px(14.)).bg(rule))
        .child(menu.rounded_l_none().min_w(px(18.)).px(px(2.)))
}

/// What a labelled [`action`] does: Danger loses work or cannot be taken back, Success is GitMan's safe forward action, Neutral the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Success,
    Danger,
}

/// Solid for a form's main answer (Enter's), Outline for the rest; a question's answers are all Solid (`confirm::answer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Solid,
    Outline,
}

/// A labelled button's size: its height, side padding and text size together, one row of [`Size::table`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// A row's, a panel's or a toolbar's button, in the 12 px text beside it.
    Small,
    /// A form's, a question's and a settings page's, as high as a one-line field, in the 14 px body text.
    Medium,
}

impl Size {
    pub const ALL: [Size; 2] = [Size::Small, Size::Medium];

    /// Height, side padding and text size.
    pub fn table(self) -> (Pixels, Pixels, Rems) {
        match self {
            Size::Small => (px(20.), px(8.), rems(0.75)),
            Size::Medium => (crate::text_input::SINGLE_HEIGHT, px(12.), rems(0.875)),
        }
    }

    pub fn height(self) -> Pixels {
        self.table().0
    }
}

/// `button` (an [`action`]) at `size`; its letters stay centred, since [`ink::text`] reads the size it is given.
pub fn sized(button: Stateful<Div>, size: Size) -> Stateful<Div> {
    let (height, padding, text) = size.table();
    button.h(height).px(padding).text_size(text)
}

/// The one gap between the buttons of a row (a question's answers, a form's, a settings control's).
pub const GAP: Pixels = px(8.);

/// A row of buttons [`GAP`] apart, centred on one line.
pub fn line() -> Div {
    div().flex().flex_none().items_center().gap(GAP)
}

/// How far a ring (the focus's, a default answer's) stands out of its button.
pub const RING: Pixels = px(2.);

/// A [`RING`]-wide ring in `color` around a box.
pub fn ring(color: Hsla) -> BoxShadow {
    BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: RING, inset: false }
}

/// An [`action`]'s colours: its fill, border and text, its fill and text hovered, and its fill pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub fill: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub hover: Hsla,
    pub hover_text: Hsla,
    pub active: Hsla,
}

/// The colours of `tone` in `fill`: success `status().success`, danger `status().error`, neutral the element grey.
pub fn look(tone: Tone, fill: Fill, cx: &App) -> Look {
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
        (Some(tone), Fill::Solid) => Look { fill: tone, border: tone, text: on(tone), hover: tone.opacity(SOLID_HOVER), hover_text: on(tone), active: tone.opacity(SOLID_ACTIVE) },
        (Some(tone), Fill::Outline) => Look { fill: clear, border: tone, text: tone, hover: tone.opacity(TINT), hover_text: tone, active: tone.opacity(TINT_HOVER) },
    }
}

/// White or black, whichever reads better on `fill`.
fn on(fill: Hsla) -> Hsla {
    let (white, black) = (gpui::white(), gpui::black());
    if crate::themes::contrast(white, fill) >= crate::themes::contrast(black, fill) { white } else { black }
}

const TINT: f32 = 0.2;
const TINT_HOVER: f32 = 0.3;
const SOLID_HOVER: f32 = 0.85;
const SOLID_ACTIVE: f32 = 0.7;

/// Every labelled button: `label`'s letters centred at [`Size::Small`] ([`sized`] for another) in [`look`]'s colours of `tone` and `fill`; the letters paint under `<key>-label`.
pub fn action(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, tone: Tone, fill: Fill, cx: &App) -> Stateful<Div> {
    labelled(hits, key.into(), None, label.into(), tone, fill, cx)
}

/// An [`action`] with `icon` in its text colour before the label, centred as its letters are.
pub fn action_with(hits: &HitTargets, key: impl Into<SharedString>, icon: Icon, label: impl Into<SharedString>, tone: Tone, fill: Fill, cx: &App) -> Stateful<Div> {
    let text = look(tone, fill, cx).text;
    labelled(hits, key.into(), Some(icon.svg(12., text).into_any_element()), label.into(), tone, fill, cx)
}

/// An [`action_with`] whose icon turns once a second (work it started runs); it also paints under `<key>-turning`.
pub fn action_turning(hits: &HitTargets, key: impl Into<SharedString>, icon: Icon, label: impl Into<SharedString>, tone: Tone, fill: Fill, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let text = look(tone, fill, cx).text;
    let turning = crate::icon::turning(icon.svg(12., text), ElementId::Name(format!("{key}-turning").into()));
    labelled(hits, key.clone(), Some(turning.into_any_element()), label.into(), tone, fill, cx).child(hit_target(hits, format!("{key}-turning")))
}

fn labelled(hits: &HitTargets, key: SharedString, icon: Option<gpui::AnyElement>, label: SharedString, tone: Tone, fill: Fill, cx: &App) -> Stateful<Div> {
    let look = look(tone, fill, cx);
    let hover_border = if tone == Tone::Neutral { cx.theme().colors().border_selected } else { look.border };
    let letters = ink::text(div().relative().child(label).child(hit_target(hits, format!("{key}-label"))));
    let button = base(hits, key, move |s| s.bg(look.hover).border_color(hover_border), look.active)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_1()
        .rounded_sm()
        .border_1()
        .border_color(look.border)
        .bg(look.fill)
        .text_color(look.text)
        .whitespace_nowrap()
        .children(icon)
        .child(letters);
    sized(button, Size::Small)
}

/// `button` dimmed, with the plain cursor, while it cannot act; its click must then do nothing.
pub fn dimmed(button: Stateful<Div>) -> Stateful<Div> {
    button.opacity(0.4).cursor_default()
}

/// A quiet outlined pill in `fg`, lit on hover; its label's letters centred, painted under `<key>-label`.
pub fn pill(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, fg: Hsla, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    let key = key.into();
    let label = ink::text(div().relative().child(label.into()).child(hit_target(hits, format!("{key}-label"))));
    base(hits, key, |s| s.bg(colors.element_hover).border_color(colors.border).text_color(colors.text), colors.element_active)
        .flex()
        .flex_none()
        .items_center()
        .h(px(20.))
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(colors.border_variant)
        .text_xs()
        .text_color(fg)
        .child(label)
}

/// A tick box: [`tick`] in the accent colour on the letters of the caller's label ([`ink::mark`]); the caller adds its click.
pub fn checkbox(hits: &HitTargets, key: impl Into<SharedString>, on: bool, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    let square = tick(on, colors.text_accent, cx).relative().group_hover(key.clone(), |s| s.border_color(if on { colors.text_accent } else { colors.border_selected }));
    div().id(key.clone()).group(key.clone()).flex_none().cursor_pointer().child(ink::mark(square.child(hit_target(hits, key.to_string()))))
}

/// [`checkbox`] taking the keyboard under `focus` (a form's [`crate::focus_order::FocusOrder`] member): its square bordered in the focus colour while focused, Space toggling it by `press`.
pub fn focusable_checkbox(hits: &HitTargets, key: impl Into<SharedString>, on: bool, focus: &FocusHandle, press: impl Fn(&Press, &mut Window, &mut App) + 'static, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    let ring = colors.border_focused;
    let square = tick(on, colors.text_accent, cx)
        .relative()
        .group_hover(key.clone(), |s| s.border_color(if on { colors.text_accent } else { colors.border_selected }))
        .track_focus(focus)
        .key_context(BUTTON_CONTEXT)
        .on_action(press)
        .focus(move |s| s.border_2().border_color(ring));
    div().id(key.clone()).group(key.clone()).flex_none().cursor_pointer().child(ink::mark(square.child(hit_target(hits, key.to_string()))))
}

/// `button` taking the keyboard under `focus` (a [`crate::focus_order::FocusOrder`] member): [`ring`]ed in the focus colour while focused, over any other ring; Space or Enter `press`.
pub fn focusable(button: Stateful<Div>, focus: &FocusHandle, press: impl Fn(&Press, &mut Window, &mut App) + 'static, cx: &App) -> Stateful<Div> {
    let focused = ring(cx.theme().colors().border_focused);
    button.track_focus(focus).key_context(BUTTON_CONTEXT).on_action(press).focus(move |s| s.shadow(vec![focused]))
}

/// [`focusable`] under `focus`, `act` run by a click and by Space or Enter alike.
pub fn acting<V: 'static>(button: Stateful<Div>, focus: &FocusHandle, act: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static, cx: &mut Context<V>) -> Stateful<Div> {
    let act = Rc::new(act);
    let pressed = act.clone();
    let button = button.on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| act(this, window, cx)));
    focusable(button, focus, cx.listener(move |this, _: &Press, window, cx| pressed(this, window, cx)), cx)
}

/// A 14 px square, checked and filled with `fill` while `on`, the same size either way; a read-only mark on its own.
pub fn tick(on: bool, fill: Hsla, cx: &App) -> Div {
    let colors = cx.theme().colors();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.))
        .rounded_sm()
        .border_1()
        .border_color(if on { fill } else { colors.border })
        .when(on, |d| d.bg(fill).child(Icon::Check.svg(10., gpui::white())))
}

/// One option of a choice; the selected one is drawn as a selected list row.
pub fn choice(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    chosen(div().id(id), selected, cx)
        .gap_2()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(if selected { colors.border_focused } else { colors.border })
        .text_sm()
        .child(div().flex_1().child(label.into()))
}

/// One option of a [`crate::segmented`] group: [`choice`]'s fills, `icon` and `label`, under `key`.
pub fn segment(hits: &HitTargets, key: impl Into<SharedString>, icon: Option<Icon>, label: Option<SharedString>, selected: bool, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    let fg = if selected { colors.text } else { colors.text_muted };
    chosen(div().id(key.clone()), selected, cx)
        .relative()
        .flex_none()
        .gap_1()
        .h_full()
        .px_2()
        .text_xs()
        .text_color(fg)
        .children(icon.map(|i| i.svg(12., fg)))
        .children(label)
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

#[cfg(test)]
mod tests;

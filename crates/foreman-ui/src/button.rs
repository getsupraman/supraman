//! The buttons. Each takes `key` as its element id, its hover group and its [`hit_target`] key.

use gpui::{App, Div, ElementId, Hsla, Pixels, SharedString, Stateful, StyleRefinement, Svg, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};

fn base(hits: &HitTargets, key: SharedString, hover: impl FnOnce(StyleRefinement) -> StyleRefinement, active: Hsla) -> Stateful<Div> {
    div()
        .id(key.clone())
        .group(key.clone())
        .relative()
        .cursor_pointer()
        .hover(hover)
        .active(|s| s.bg(active))
        .child(hit_target(hits, key.to_string()))
}

/// Borderless, the ghost fill on hover, a deeper one pressed; the caller lays it out.
pub fn ghost(hits: &HitTargets, key: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    base(hits, key.into(), |s| s.bg(colors.ghost_element_hover), colors.ghost_element_active)
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

/// An action bar button: `icon` in `fg`, then any label; the selected fill while `on`.
pub fn toolbar(hits: &HitTargets, key: impl Into<SharedString>, icon: Svg, on: bool, fg: Hsla, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    base(hits, key.clone(), |s| s.bg(colors.ghost_element_hover).text_color(colors.text), colors.ghost_element_active)
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
        .when(on, |d| d.bg(colors.ghost_element_selected))
        .child(icon.group_hover(key, |s| s.text_color(colors.text)))
}

/// A small bordered button: one height, the label centred, a lighter fill and border on hover.
pub fn bordered(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    base(hits, key.into(), |s| s.bg(colors.element_hover).border_color(colors.border_selected), colors.element_active)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(20.))
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(colors.border)
        .bg(colors.element_background)
        .text_xs()
        .text_color(colors.text)
        .whitespace_nowrap()
        .child(label.into())
}

/// A ghost button with a `border` outline and a text label.
pub fn outline(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, border: Hsla, cx: &App) -> Stateful<Div> {
    ghost(hits, key, cx)
        .flex_none()
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(border)
        .text_xs()
        .child(label.into())
}

/// A quiet outlined pill in `fg` whose border, fill and text come up on hover.
pub fn pill(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, fg: Hsla, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    base(hits, key.into(), |s| s.bg(colors.element_hover).border_color(colors.border).text_color(colors.text), colors.element_active)
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
        .child(label.into())
}

/// A bordered button sized by its text; no hit key.
pub fn small(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    div()
        .id(id)
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(colors.border)
        .bg(colors.element_background)
        .text_sm()
        .cursor_pointer()
        .hover(|s| s.bg(colors.element_hover))
        .active(|s| s.bg(colors.element_active))
        .child(label.into())
}

/// One option of a choice; the selected one is drawn as a selected list row.
pub fn choice(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(if selected { colors.border_focused } else { colors.border })
        .when(selected, |d| d.bg(colors.element_selected))
        .when(!selected, |d| d.bg(colors.element_background).hover(|s| s.bg(colors.element_hover)))
        .text_sm()
        .cursor_pointer()
        .child(div().flex_1().child(label.into()))
}

//! The buttons. Each takes `key` as its element id, its hover group and its [`hit_target`] key.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, Div, ElementId, Hsla, Pixels, SharedString, Stateful, StyleRefinement, Svg, Transformation, div, percentage,
    prelude::*, px,
};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::ink;

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

/// An [`icon`] button that is always laid out and painted, so its slot never shifts and its hit key exists, but
/// shows only while the `group` (its row) is hovered or `shown` (the row is selected).
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
    frame.child(hit_target(hits, format!("{key}-turning"))).child(icon.with_animation(
        ElementId::Name(format!("{key}-turning").into()),
        Animation::new(Duration::from_secs(1)).repeat(),
        |icon, turned| icon.with_transformation(Transformation::rotate(percentage(turned))),
    ))
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

/// `button` dimmed, with the plain cursor, while it cannot act; its click must then do nothing.
pub fn dimmed(button: Stateful<Div>) -> Stateful<Div> {
    button.opacity(0.4).cursor_default()
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

/// A quiet outlined pill in `fg` whose border, fill and text come up on hover; its label's letters centred in it,
/// painted under `<key>-label`.
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

/// A tick box: [`tick`] in the accent colour, its border lit on hover, on the letters of the label the caller puts
/// beside it ([`ink::mark`]); the caller adds its click.
pub fn checkbox(hits: &HitTargets, key: impl Into<SharedString>, on: bool, cx: &App) -> Stateful<Div> {
    let key = key.into();
    let colors = cx.theme().colors();
    let square = tick(on, colors.text_accent, cx).relative().group_hover(key.clone(), |s| s.border_color(if on { colors.text_accent } else { colors.border_selected }));
    div().id(key.clone()).group(key.clone()).flex_none().cursor_pointer().child(ink::mark(square.child(hit_target(hits, key.to_string()))))
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

/// One option of a [`crate::segmented`] group: [`choice`]'s fills without a border of its own, `icon` and
/// `label` side by side; paints under `key`.
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
mod tests {
    use gpui::{Context, Pixels, Render, TestAppContext, VisualTestContext, Window, rems};

    use super::*;

    struct Row {
        hits: HitTargets,
    }

    impl Render for Row {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let label = div().relative().child("View all files").child(hit_target(&self.hits, "label".into()));
            let row = div().flex().items_center().gap_1p5().text_xs().child(checkbox(&self.hits, "tick", true, cx)).child(label);
            div().flex().flex_col().gap_2().p_2().child(row).child(div().flex().child(pill(&self.hits, "pill", "main", gpui::red(), cx)))
        }
    }

    fn centre(row: &gpui::Entity<Row>, key: &str, cx: &mut VisualTestContext) -> Pixels {
        row.read_with(cx, |r, _| r.hits.borrow().get(key).map(|h| h.bounds.center().y)).unwrap_or_else(|| panic!("{key} paints"))
    }

    #[gpui::test]
    fn a_tick_box_sits_on_its_labels_letters_and_a_pills_letters_on_its_middle(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (row, cx) = cx.add_window_view(|_, _| Row { hits: HitTargets::default() });
        cx.run_until_parked();
        let below = cx.update(|window, _| {
            let mut style = window.text_style();
            style.font_size = rems(0.75).into();
            crate::ink::offset(&style, window)
        });
        assert!(below > px(0.), "the letters' middle is below the line's");
        assert!(crate::ink::level(centre(&row, "tick", cx), centre(&row, "label", cx) + below), "the square on the label's letters");
        assert!(crate::ink::level(centre(&row, "pill", cx), centre(&row, "pill-label", cx) + below), "the pill's letters on its middle");
    }
}

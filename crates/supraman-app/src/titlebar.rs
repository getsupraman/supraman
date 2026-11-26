use gpui::{AnyElement, App, Div, FontWeight, Pixels, Stateful, Window, WindowControlArea, div, prelude::*, px};
use supraman_ui::button;
use supraman_ui::fonts;
use supraman_ui::hit::HitTargets;
use theme::ActiveTheme as _;

pub const HEIGHT: Pixels = px(34.);

/// `minimizable` false for a modal window, which minimises with its owner.
pub fn render(title: &'static str, actions: Option<AnyElement>, minimizable: bool, hits: &HitTargets, window: &Window, cx: &App) -> AnyElement {
    let colors = cx.theme().colors();
    div()
        .id("title-bar")
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .w_full()
        .h(HEIGHT)
        .bg(colors.title_bar_background)
        .border_b_1()
        .border_color(colors.border)
        .window_control_area(WindowControlArea::Drag)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .pl_3()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(crate::brand::supraman(16.))
                .child(title),
        )
        .child(
            div()
                .flex()
                .h_full()
                .items_center()
                .children(actions)
                .child(caption_buttons(minimizable, hits, window, cx)),
        )
        .into_any_element()
}

pub fn icon_button(hits: &HitTargets, key: &'static str, glyph: &'static str, cx: &App) -> Stateful<Div> {
    button::ghost_muted(hits, key, cx)
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .w(px(46.))
        .h_full()
        .font_family(fonts::icons())
        .text_size(px(12.))
        .child(glyph)
}

fn caption_buttons(minimizable: bool, hits: &HitTargets, window: &Window, cx: &App) -> AnyElement {
    let max = if window.is_maximized() { Caption::Restore } else { Caption::Maximize };
    div()
        .flex()
        .h_full()
        .font_family(fonts::icons())
        .when(minimizable, |d| d.child(Caption::Minimize.render(hits, cx)))
        .child(max.render(hits, cx))
        .child(Caption::Close.render(hits, cx))
        .into_any_element()
}

#[derive(Clone, Copy)]
enum Caption {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl Caption {
    // Windows answers a click through the non-client `area`, so this is no `button`: the hit key only records where it is painted.
    fn render(self, hits: &HitTargets, cx: &App) -> AnyElement {
        let (id, glyph, area) = match self {
            Self::Minimize => ("caption-min", "\u{e921}", WindowControlArea::Min),
            Self::Maximize => ("caption-max", "\u{e922}", WindowControlArea::Max),
            Self::Restore => ("caption-max", "\u{e923}", WindowControlArea::Max),
            Self::Close => ("caption-close", "\u{e8bb}", WindowControlArea::Close),
        };
        button::caption(hits, id, matches!(self, Self::Close), cx)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .w(px(46.))
            .h_full()
            .text_size(px(10.))
            .window_control_area(area)
            .child(glyph)
            .into_any_element()
    }
}

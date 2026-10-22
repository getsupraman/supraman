//! Client-drawn title bar, as Zed does it on Windows: the window opens with a transparent titlebar,
//! and gpui hit-tests `WindowControlArea`s so dragging, snap layouts and the caption buttons stay
//! native.

use gpui::{AnyElement, App, FontWeight, Hsla, Pixels, Rgba, Window, WindowControlArea, div, prelude::*, px};
use theme::ActiveTheme as _;

pub const HEIGHT: Pixels = px(34.);

pub fn render(window: &Window, cx: &App) -> AnyElement {
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
                .pl_3()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Foreman"),
        )
        .child(caption_buttons(window, cx))
        .into_any_element()
}

fn caption_buttons(window: &Window, cx: &App) -> AnyElement {
    let max = if window.is_maximized() { Caption::Restore } else { Caption::Maximize };
    div()
        .flex()
        .h_full()
        .font_family(icon_font())
        .child(Caption::Minimize.render(cx))
        .child(max.render(cx))
        .child(Caption::Close.render(cx))
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
    fn render(self, cx: &App) -> AnyElement {
        let colors = cx.theme().colors();
        let (id, glyph, area) = match self {
            Self::Minimize => ("caption-min", "\u{e921}", WindowControlArea::Min),
            Self::Maximize => ("caption-max", "\u{e922}", WindowControlArea::Max),
            Self::Restore => ("caption-max", "\u{e923}", WindowControlArea::Max),
            Self::Close => ("caption-close", "\u{e8bb}", WindowControlArea::Close),
        };
        let (hover_bg, hover_fg): (Hsla, Hsla) = match self {
            Self::Close => (Rgba { r: 232. / 255., g: 17. / 255., b: 32. / 255., a: 1. }.into(), gpui::white()),
            _ => (colors.ghost_element_hover, colors.text),
        };
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .w(px(46.))
            .h_full()
            .text_size(px(10.))
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .window_control_area(area)
            .child(glyph)
            .into_any_element()
    }
}

/// Segoe Fluent Icons ships with Windows 11 (build 22000+); Windows 10 has the same glyphs in MDL2.
#[cfg(windows)]
fn icon_font() -> &'static str {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    let mut version = unsafe { std::mem::zeroed() };
    let ok = unsafe { RtlGetVersion(&mut version) }.is_ok();
    if ok && version.dwBuildNumber >= 22000 { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }
}

#[cfg(not(windows))]
fn icon_font() -> &'static str {
    "Segoe Fluent Icons"
}

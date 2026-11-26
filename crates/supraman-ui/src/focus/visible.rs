//! `focus_visible`: the focus came by keyboard (a key since the window's last mouse press), and the kit's one focus
//! ring, painted only then (UI-KIT §4.3).

use gpui::{App, BorderStyle, DispatchPhase, FocusHandle, Global, IntoElement, MouseDownEvent, Pixels, Styled as _, Subscription, Window, canvas, quad};
use theme::ActiveTheme as _;

use super::state;
use crate::button::RING;

struct Keys(#[allow(dead_code)] Subscription);

impl Global for Keys {}

/// Each frame, from the window's root: a key marks the window keyboard-driven, a mouse press clears it.
pub(super) fn track(window: &mut Window, cx: &mut App) {
    if !cx.has_global::<Keys>() {
        let keys = cx.observe_keystrokes(|_, window, cx| keyed(window, cx));
        cx.set_global(Keys(keys));
    }
    window.on_mouse_event(|_: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            state(window, cx).borrow_mut().keyboard = false;
        }
    });
}

/// The window's input is the keyboard's until its next mouse press: what a key does, for a focus that stands in for one.
pub fn keyed(window: &mut Window, cx: &mut App) {
    state(window, cx).borrow_mut().keyboard = true;
}

/// Whether `focus` holds the keys and came by keyboard.
pub fn visible(focus: &FocusHandle, window: &mut Window, cx: &mut App) -> bool {
    focus.is_focused(window) && state(window, cx).borrow().keyboard
}

/// The kit's focus ring: a [`RING`]-wide outline in the focus colour just outside its parent's box (`relative`), its
/// corners `radius` out, painted while `focus` is [`visible`]; never a shadow, which would fill a clear box.
pub fn ring(focus: &FocusHandle, radius: Pixels) -> impl IntoElement {
    let focus = focus.clone();
    ring_while(radius, move |window, cx| visible(&focus, window, cx))
}

/// [`ring`] painted while `shown` holds: a default answer's, Enter's while nothing else in its panel holds the keys.
pub fn ring_while(radius: Pixels, shown: impl Fn(&mut Window, &mut App) -> bool + 'static) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            if shown(window, cx) {
                let color = cx.theme().colors().border_focused;
                window.paint_quad(quad(bounds.dilate(RING), radius + RING, gpui::transparent_black(), RING, color, BorderStyle::Solid));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests;

//! One interaction state per element and its one resolve to appearance, in a fixed precedence (UI-KIT §4.1; the idea
//! from gpui-kit's `base/state_style.rs`). The pointer's hover and press are gpui's paint ([`Look::hover`],
//! [`Look::pressed`]); a hover the kit tracks itself (a menu's highlight) is [`State::hovered`].

use gpui::{Hsla, InteractiveElement, SharedString, StatefulInteractiveElement, Styled, prelude::FluentBuilder};

use crate::tokens;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub hovered: bool,
    pub selected: bool,
    /// It anchors a popup still open ([`crate::popup::is_open`]).
    pub open: bool,
    pub focus_visible: bool,
    pub disabled: bool,
    /// It is the source of the drag under way ([`crate::dnd::dragging`]).
    pub dragging: bool,
    /// A payload it accepts hovers it ([`crate::dnd::DropZone`]).
    pub drop_over: bool,
}

/// A control's fill in each state; `None` keeps the one below it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fills {
    pub rest: Option<Hsla>,
    pub hover: Option<Hsla>,
    pub pressed: Option<Hsla>,
    pub selected: Option<Hsla>,
    pub open: Option<Hsla>,
    pub drop_over: Option<Hsla>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub fill: Option<Hsla>,
    /// The pointer's hover fill, none while a state above it holds the fill.
    pub hover: Option<Hsla>,
    pub pressed: Option<Hsla>,
    pub ring: bool,
    pub opacity: f32,
}

/// Rest, then hovered, pressed, selected, open, drop_over, the focus ring, disabled and dragging last.
pub fn resolve(state: State, fills: Fills) -> Look {
    let mut fill = fills.rest;
    if state.hovered {
        fill = fills.hover.or(fill);
    }
    let selected = state.selected && fills.selected.is_some();
    if selected {
        fill = fills.selected;
    }
    let open = state.open && fills.open.is_some();
    if open {
        fill = fills.open;
    }
    let over = state.drop_over && fills.drop_over.is_some();
    if over {
        fill = fills.drop_over;
    }
    let live = !state.disabled;
    Look {
        fill,
        hover: fills.hover.filter(|_| live && !state.hovered && !selected && !open && !over),
        pressed: fills.pressed.filter(|_| live),
        ring: state.focus_visible,
        opacity: if state.disabled || state.dragging { tokens::DIMMED } else { 1. },
    }
}

impl Look {
    /// The fill, opacity and pointer's hover on `el`, the hover under `group` (the element's own without one).
    pub fn paint<E: InteractiveElement + Styled + FluentBuilder>(self, el: E, group: Option<SharedString>) -> E {
        let Look { fill, hover, opacity, .. } = self;
        el.when_some(fill, |d, bg| d.bg(bg))
            .when_some(hover, |d, bg| match &group {
                Some(group) => d.group_hover(group.clone(), move |s| s.bg(bg)),
                None => d.hover(move |s| s.bg(bg)),
            })
            .when(opacity < 1., |d| d.opacity(opacity))
    }

    /// [`Look::paint`] with the pressed fill.
    pub fn paint_pressed<E: StatefulInteractiveElement + Styled + FluentBuilder>(self, el: E, group: Option<SharedString>) -> E {
        let pressed = self.pressed;
        self.paint(el, group).when_some(pressed, |d, bg| d.active(move |s| s.bg(bg)))
    }
}

#[cfg(test)]
mod tests;

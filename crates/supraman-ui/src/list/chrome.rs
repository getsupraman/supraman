//! A list row's fills, painted by the list under the row's content: its band at rest, the hover and press, the
//! selection (`tokens::roles` `selection` while the list holds the keys, `selection_inactive` else) and the open fill
//! of a popup anchored to the row, in [`crate::state::resolve`]'s precedence.

use gpui::{App, Div, Hsla, Pixels, SharedString, Stateful, div, prelude::*};
use theme::ActiveTheme as _;

use crate::controls::button::RowKeys;
use crate::hit::{HitTargets, hit_target};
use crate::state::{Fills, State, resolve};

/// What the list knows of one row as it paints it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chrome {
    selected: bool,
    focused: bool,
    band: Option<Hsla>,
    hover: Option<Hsla>,
    inset: Pixels,
    radius: Pixels,
    still: bool,
    chosen: Option<Hsla>,
}

impl Chrome {
    pub fn new() -> Self {
        Self::default()
    }

    /// It is the list's selected row.
    pub fn selected(self, selected: bool) -> Self {
        Self { selected, ..self }
    }

    /// The list holds the keys, so its selection is the active one.
    pub fn focused(self, focused: bool) -> Self {
        Self { focused, ..self }
    }

    /// The row's own fill at rest (a table's band, a blocked session's tint).
    pub fn band(self, band: Option<Hsla>) -> Self {
        Self { band, ..self }
    }

    /// In place of the ghost hover (a tinted row's stronger tint).
    pub fn hover_fill(self, hover: Hsla) -> Self {
        Self { hover: Some(hover), ..self }
    }

    /// The fills held short of the row's top and bottom.
    pub fn inset(self, inset: Pixels) -> Self {
        Self { inset, ..self }
    }

    pub fn radius(self, radius: Pixels) -> Self {
        Self { radius, ..self }
    }

    /// No hover or press: a row that does nothing on a click (a caption, an empty line).
    pub fn still(self) -> Self {
        Self { still: true, ..self }
    }

    /// The selected fill given, in place of the roles' (`button::row`'s callers in the kit).
    pub(crate) fn chosen(self, fill: Hsla) -> Self {
        Self { chosen: Some(fill), ..self }
    }

    /// Its fills from the theme's roles.
    pub fn fills(&self, cx: &App) -> Fills {
        let (colors, roles) = (cx.theme().colors(), crate::tokens::roles(cx));
        let live = !self.still;
        Fills {
            rest: self.band,
            hover: live.then(|| self.hover.unwrap_or(colors.ghost_element_hover)),
            pressed: live.then_some(colors.ghost_element_active),
            selected: Some(self.chosen.unwrap_or(if self.focused { roles.selection } else { roles.selection_inactive })),
            open: Some(roles.open),
            drop_over: None,
        }
    }

    /// The selection fill it paints, `None` while it is not selected.
    pub fn selection(&self, cx: &App) -> Option<Hsla> {
        self.selected.then(|| self.fills(cx).selected).flatten()
    }

    /// The fill element, the first child of its `relative` row: hovered and pressed with `group`, open while a popup
    /// anchored to `anchor` is ([`crate::popup::open_fill`]).
    pub fn fill(self, anchor: impl Into<SharedString>, group: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
        let (fills, group) = (self.fills(cx), group.into());
        let look = resolve(State { selected: self.selected, ..State::default() }, fills);
        let pressed = look.pressed;
        let el = div().id("list-fill").absolute().left_0().right_0().top(self.inset).bottom(self.inset).rounded(self.radius);
        look.paint(el, Some(group.clone()))
            .when_some(pressed, |d, bg| d.group_active(group, move |s| s.bg(bg)))
            .child(crate::popup::open_fill(anchor).radius(self.radius))
    }
}

/// A list row under `keys` (its element id, hover group and hit key), its [`Chrome`] painted under what the owner adds.
pub fn row(hits: &HitTargets, keys: RowKeys, chrome: Chrome, cx: &App) -> Stateful<Div> {
    let fill = chrome.fill(keys.hit.clone(), keys.group.clone(), cx);
    div().id(keys.id).group(keys.group).relative().when(!chrome.still, |d| d.cursor_pointer()).child(fill).child(hit_target(hits, keys.hit.to_string()))
}

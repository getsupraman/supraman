//! An element's painted size read back the next frame: what a view lays itself out by.

use std::{cell::Cell, rc::Rc};

use gpui::{App, EntityId, IntoElement, Pixels, Size, Styled, Window, canvas};

/// Records its parent's size each frame; `changed` says whether it differs from the last, and a change asks for
/// another frame (and notifies `view` when given), since gpui drops a refresh asked for while it draws.
pub fn probe(view: Option<EntityId>, changed: impl Fn(Size<Pixels>) -> bool + 'static) -> impl IntoElement {
    canvas(
        move |bounds, window: &mut Window, cx: &mut App| {
            if changed(bounds.size) {
                window.defer(cx, move |window, cx| {
                    if let Some(view) = view {
                        cx.notify(view);
                    }
                    window.refresh();
                });
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A view's own painted width, kept between frames: `None` until it first painted.
#[derive(Clone, Default)]
pub struct Width(Rc<Cell<Option<Pixels>>>);

impl Width {
    pub fn get(&self) -> Option<Pixels> {
        self.0.get()
    }

    /// Whether the width is known and under `limit`.
    pub fn under(&self, limit: Pixels) -> bool {
        self.get().is_some_and(|w| w < limit)
    }

    /// The probe for the view `view`'s element, absolute over it; a new width re-renders the view.
    pub fn probe(&self, view: EntityId) -> impl IntoElement {
        let cell = self.0.clone();
        probe(Some(view), move |size| {
            let changed = cell.get() != Some(size.width);
            cell.set(Some(size.width));
            changed
        })
    }
}

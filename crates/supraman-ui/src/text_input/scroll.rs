//! A multi-line field's rows as a [`ScrollbarHandle`]: its prepaint reads them in and takes the move the bar asks for.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Bounds, Pixels, Point, Size, point, size};

use crate::scroll::ScrollbarHandle;

#[derive(Clone, Default)]
pub(super) struct TextScroll(Rc<RefCell<Rows>>);

#[derive(Default)]
struct Rows {
    viewport: Bounds<Pixels>,
    content: Pixels,
    top: Pixels,
    wanted: Option<Pixels>,
}

impl TextScroll {
    /// The move the bar asked for since the last frame.
    pub(super) fn take_wanted(&self) -> Option<Pixels> {
        self.0.borrow_mut().wanted.take()
    }

    /// The rows as this frame lays them out: `top` scrolled off above the viewport.
    pub(super) fn read(&self, viewport: Bounds<Pixels>, content: Pixels, top: Pixels) {
        let mut rows = self.0.borrow_mut();
        rows.viewport = viewport;
        rows.content = content;
        rows.top = top;
    }
}

impl ScrollbarHandle for TextScroll {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().viewport
    }

    fn offset(&self) -> Point<Pixels> {
        point(Pixels::ZERO, -self.0.borrow().top)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let mut rows = self.0.borrow_mut();
        rows.top = -offset.y;
        rows.wanted = Some(-offset.y);
    }

    fn content_size(&self) -> Size<Pixels> {
        let rows = self.0.borrow();
        size(rows.viewport.size.width, rows.content.max(rows.viewport.size.height))
    }
}

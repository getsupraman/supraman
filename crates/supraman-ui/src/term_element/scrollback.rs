//! A terminal's scrollback as a [`ScrollbarHandle`]: the grid's prepaint reads it in and applies a move the bar asks for.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Bounds, Pixels, Point, Size, point, size};
use supraman_term::Terminal;

use crate::scroll::ScrollbarHandle;

#[derive(Clone, Default)]
pub struct Scrollback(Rc<RefCell<Lines>>);

#[derive(Default)]
struct Lines {
    viewport: Bounds<Pixels>,
    line_height: Pixels,
    history: usize,
    display_offset: usize,
    wanted: Option<usize>,
}

impl Scrollback {
    /// Queues the move the bar asked for since the last frame; before the grid syncs.
    pub(super) fn apply(&self, terminal: &mut Terminal) {
        let mut lines = self.0.borrow_mut();
        if let Some(wanted) = lines.wanted.take() {
            match wanted.cmp(&lines.display_offset) {
                std::cmp::Ordering::Greater => terminal.scroll_up_by(wanted - lines.display_offset),
                std::cmp::Ordering::Less => terminal.scroll_down_by(lines.display_offset - wanted),
                std::cmp::Ordering::Equal => {}
            }
        }
    }

    /// The grid as it was synced this frame.
    pub(super) fn read(&self, terminal: &Terminal, viewport: Bounds<Pixels>, line_height: Pixels) {
        let content = terminal.last_content();
        let mut lines = self.0.borrow_mut();
        lines.viewport = viewport;
        lines.line_height = line_height;
        lines.history = content.total_lines.saturating_sub(content.screen_lines);
        lines.display_offset = content.display_offset.min(lines.history);
    }
}

impl ScrollbarHandle for Scrollback {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().viewport
    }

    fn offset(&self) -> Point<Pixels> {
        let lines = self.0.borrow();
        point(Pixels::ZERO, -(lines.line_height * (lines.history - lines.display_offset) as f32))
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let mut lines = self.0.borrow_mut();
        if lines.line_height <= Pixels::ZERO {
            return;
        }
        let from_top = ((-offset.y / lines.line_height).round().max(0.) as usize).min(lines.history);
        lines.wanted = Some(lines.history - from_top);
    }

    fn content_size(&self) -> Size<Pixels> {
        let lines = self.0.borrow();
        size(lines.viewport.size.width, lines.viewport.size.height + lines.line_height * lines.history as f32)
    }
}

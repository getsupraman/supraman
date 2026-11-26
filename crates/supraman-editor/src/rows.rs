//! Where an editor shows its rows, in window coordinates and as buffer rows.

use std::ops::Range;

use editor::display_map::{DisplayPoint, DisplayRow, DisplaySnapshot};
use editor::{Bias, Editor};
use gpui::{App, Entity, Pixels, Point, Window};
use multi_buffer::MultiBufferRow;

fn line_height(editor: &mut Editor, window: &Window, cx: &App) -> Pixels {
    editor.style(cx).text.line_height_in_pixels(window.rem_size())
}

/// The display rows `row` (of the editor's text) spans.
fn display_span(snapshot: &DisplaySnapshot, row: u32) -> Range<u32> {
    let end = text::Point::new(row, snapshot.buffer_snapshot().line_len(MultiBufferRow(row)));
    snapshot.point_to_display_point(text::Point::new(row, 0), Bias::Left).row().0..snapshot.point_to_display_point(end, Bias::Left).row().0 + 1
}

/// The row of `editor`'s text under `position` (window coordinates); `None` over a filler or outside the editor.
pub fn row_at(editor: &Entity<Editor>, position: Point<Pixels>, window: &mut Window, cx: &mut App) -> Option<u32> {
    editor.update(cx, |e, cx| {
        let bounds = *e.last_bounds()?;
        if !bounds.contains(&position) {
            return None;
        }
        let height = line_height(e, window, cx);
        let top = e.scroll_position(cx).y;
        let snapshot = e.display_snapshot(cx);
        let shown = top + f64::from((position.y - bounds.origin.y) / height);
        if shown < 0. || shown > f64::from(snapshot.max_point().row().0 + 1) {
            return None;
        }
        let display = DisplayRow(shown.floor() as u32);
        let row = snapshot.display_point_to_point(DisplayPoint::new(display, 0), Bias::Left).row;
        display_span(&snapshot, row).contains(&display.0).then_some(row)
    })
}

/// Where `editor` shows the text rows `rows` (window coordinates), across its width and cut to its bounds; `None` while none is in view.
pub fn rows_bounds(editor: &Entity<Editor>, rows: Range<u32>, window: &mut Window, cx: &mut App) -> Option<gpui::Bounds<Pixels>> {
    editor.update(cx, |e, cx| {
        let bounds = *e.last_bounds()?;
        let height = line_height(e, window, cx);
        let top = e.scroll_position(cx).y;
        let snapshot = e.display_snapshot(cx);
        let (first, last) = (display_span(&snapshot, rows.start).start, display_span(&snapshot, rows.end.saturating_sub(1).max(rows.start)).end);
        let at = |display: u32| bounds.origin.y + height * (f64::from(display) - top) as f32;
        let (y0, y1) = (at(first).max(bounds.top()), at(last).min(bounds.bottom()));
        (y1 > y0).then(|| gpui::Bounds::from_corners(gpui::point(bounds.left(), y0), gpui::point(bounds.right(), y1)))
    })
}

/// The window y of the top of `row` (of `editor`'s text); `None` while it is out of view.
#[cfg(any(test, feature = "test-support"))]
pub fn row_top(editor: &Entity<Editor>, row: u32, window: &mut Window, cx: &mut App) -> Option<Pixels> {
    editor.update(cx, |e, cx| {
        let bounds = *e.last_bounds()?;
        let height = line_height(e, window, cx);
        let top = e.scroll_position(cx).y;
        let snapshot = e.display_snapshot(cx);
        let y = height * (f64::from(display_span(&snapshot, row).start) - top) as f32;
        (y >= -height * 0.5 && y < bounds.size.height).then(|| bounds.origin.y + y)
    })
}

#[cfg(any(test, feature = "test-support"))]
pub fn left(editor: &Entity<Editor>, cx: &App) -> Option<Pixels> {
    editor.read(cx).last_bounds().map(|b| b.origin.x)
}

/// The rows of `editor`'s newest non-empty selection; one ending at a row's start leaves that row out.
pub fn selected_rows(editor: &Entity<Editor>, cx: &mut App) -> Option<Range<u32>> {
    editor.update(cx, |e, cx| {
        let snapshot = e.display_snapshot(cx);
        let range = e.selections.newest::<text::Point>(&snapshot).range();
        if range.is_empty() {
            return None;
        }
        let end = if range.end.column > 0 || range.end.row == range.start.row { range.end.row + 1 } else { range.end.row };
        Some(range.start.row..end)
    })
}

/// Each row's buffer row and whether it is an inline diff's removed row; `None` for a row of no buffer.
pub fn row_lines(editor: &Entity<Editor>, cx: &App) -> Vec<Option<(u32, bool)>> {
    let snapshot = editor.read(cx).buffer().read(cx).snapshot(cx);
    snapshot
        .row_infos(MultiBufferRow(0))
        .map(|info| {
            let removed = info.diff_status.is_some_and(|s| s.kind == buffer_diff::DiffHunkStatusKind::Deleted);
            info.buffer_row.map(|row| (row, removed))
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub fn select_rows(editor: &Entity<Editor>, rows: Range<u32>, window: &mut Window, cx: &mut App) {
    editor.update(cx, |e, cx| {
        let range = text::Point::new(rows.start, 0)..text::Point::new(rows.end, 0);
        e.change_selections(editor::SelectionEffects::no_scroll(), window, cx, |s| s.select_ranges([range]));
    });
}

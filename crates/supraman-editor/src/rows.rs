//! Where an editor shows its rows: the row under a window position, a row's top, the rows selected, and the
//! buffer row each of its rows shows.

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

/// The window y of the top of `row` (of `editor`'s text); `None` while it is out of view.
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

/// The window x where `editor`'s area starts; `None` before it was painted.
pub fn left(editor: &Entity<Editor>, cx: &App) -> Option<Pixels> {
    editor.read(cx).last_bounds().map(|b| b.origin.x)
}

/// The window x where `editor`'s area ends; `None` before it was painted.
pub fn right(editor: &Entity<Editor>, cx: &App) -> Option<Pixels> {
    editor.read(cx).last_bounds().map(|b| b.origin.x + b.size.width)
}

/// The rows of `editor`'s newest selection while it selects text; a selection ending at a row's start leaves
/// that row out.
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

/// Each row of `editor`'s text: the row of its buffer it shows and whether it is a removed one (an inline
/// diff's old rows); `None` for a row of no buffer.
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

/// `f` each time `editor` scrolls.
pub fn on_scroll(editor: &Entity<Editor>, f: impl Fn(&mut App) + 'static, cx: &mut App) -> gpui::Subscription {
    cx.subscribe(editor, move |_, e: &editor::EditorEvent, cx| {
        if let editor::EditorEvent::ScrollPositionChanged { .. } = e {
            f(cx);
        }
    })
}

/// Selects `rows` of `editor`'s text whole.
pub fn select_rows(editor: &Entity<Editor>, rows: Range<u32>, window: &mut Window, cx: &mut App) {
    editor.update(cx, |e, cx| {
        let range = text::Point::new(rows.start, 0)..text::Point::new(rows.end, 0);
        e.change_selections(editor::SelectionEffects::no_scroll(), window, cx, |s| s.select_ranges([range]));
    });
}

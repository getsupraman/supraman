//! The inline diff: the new text with each removed run shown above what replaced it, painted with the
//! same line and word backgrounds as the split.

use std::ops::Range;
use std::sync::Arc;

use buffer_diff::DiffHunkStatusKind;
use editor::display_map::HighlightKey;
use editor::{Editor, RowHighlightOptions};
use gpui::{App, AppContext as _, Context, Entity, Window};
use multi_buffer::{Anchor, MultiBuffer, MultiBufferRow, MultiBufferSnapshot};
use text::{Point, ToPoint as _};
use theme::ActiveTheme as _;

use crate::look::diff_colors;
use crate::{Diff, read_only_editor};

struct Removed;
struct Added;

/// `rows` of `snapshot` as a range `highlight_rows` paints every one of: it ends inside the last row, past
/// its text, unless that row is empty (a range ending at a row's start leaves the row out).
pub(crate) fn whole_rows(snapshot: &MultiBufferSnapshot, rows: Range<u32>) -> Range<Anchor> {
    let last = rows.end.max(rows.start + 1) - 1;
    let len = snapshot.line_len(MultiBufferRow(last));
    let end = if len > 0 { Point::new(last, len) } else { Point::new(last + 1, 0).min(snapshot.max_point()) };
    snapshot.anchor_before(Point::new(rows.start, 0))..snapshot.anchor_before(end)
}

/// Where each old row shown as removed and each new row sits in the editor.
struct Placed {
    old_at: Vec<Option<u32>>,
    new_at: Vec<Option<u32>>,
}

impl Placed {
    fn of(snapshot: &MultiBufferSnapshot, rows: (u32, u32)) -> Placed {
        let mut p = Placed { old_at: vec![None; rows.0 as usize + 1], new_at: vec![None; rows.1 as usize + 1] };
        for info in snapshot.row_infos(MultiBufferRow(0)) {
            let (Some(row), Some(MultiBufferRow(at))) = (info.buffer_row, info.multibuffer_row) else { continue };
            let removed = info.diff_status.is_some_and(|s| s.kind == DiffHunkStatusKind::Deleted);
            if let Some(slot) = if removed { &mut p.old_at } else { &mut p.new_at }.get_mut(row as usize) {
                *slot = Some(at);
            }
        }
        p
    }

    /// The editor rows `rows` of one side sit on, as runs.
    fn runs(at: &[Option<u32>], rows: Range<u32>) -> Vec<Range<u32>> {
        let mut runs: Vec<Range<u32>> = Vec::new();
        for row in rows.filter_map(|r| at.get(r as usize).copied().flatten()) {
            match runs.last_mut() {
                Some(run) if run.end == row => run.end += 1,
                _ => runs.push(row..row + 1),
            }
        }
        runs
    }
}

/// `words` (byte ranges of `text`) as ranges of the editor, through `at`, each row's place in it.
fn words_at(snapshot: &MultiBufferSnapshot, text: &text::BufferSnapshot, at: &[Option<u32>], words: &[Range<usize>]) -> Vec<Range<Anchor>> {
    let point = |offset: usize| {
        let p = offset.to_point(text);
        at.get(p.row as usize).copied().flatten().map(|row| Point::new(row, p.column))
    };
    words.iter().filter_map(|w| Some(snapshot.anchor_before(point(w.start)?)..snapshot.anchor_after(point(w.end)?))).collect()
}

fn mark(editor: &mut Editor, diff: &Diff, cx: &mut Context<Editor>) {
    let snapshot = editor.buffer().read(cx).snapshot(cx);
    let placed = Placed::of(&snapshot, diff.row_counts(cx));
    for h in diff.hunks.iter() {
        for run in Placed::runs(&placed.old_at, h.old_rows.clone()) {
            editor.highlight_rows::<Removed>(whole_rows(&snapshot, run), |cx| diff_colors(cx.theme()).removed.line, RowHighlightOptions::default(), cx);
        }
        for run in Placed::runs(&placed.new_at, h.new_rows.clone()) {
            editor.highlight_rows::<Added>(whole_rows(&snapshot, run), |cx| diff_colors(cx.theme()).added.line, RowHighlightOptions::default(), cx);
        }
    }
    let (old, new) = (diff.old.read(cx).snapshot(), diff.new.read(cx).snapshot());
    let (mut ranges, mut removed) = (Vec::new(), Vec::new());
    for h in diff.hunks.iter() {
        for (words, is_removed) in [(words_at(&snapshot, &old, &placed.old_at, &h.old_words), true), (words_at(&snapshot, &new, &placed.new_at, &h.new_words), false)] {
            removed.extend(std::iter::repeat_n(is_removed, words.len()));
            ranges.extend(words);
        }
    }
    let removed: Arc<[bool]> = removed.into();
    editor.highlight_background(
        HighlightKey::Editor,
        &ranges,
        move |i, t| {
            let tint = if removed.get(*i).copied().unwrap_or(false) { diff_colors(t).removed } else { diff_colors(t).added };
            tint.word
        },
        cx,
    );
}

/// One column: the new text with each removed run shown above what replaced it, rows and changed words
/// tinted as in [`crate::SplitDiff`].
pub fn inline_editor(diff: &Diff, window: &mut Window, cx: &mut App) -> Entity<Editor> {
    let (new, buffer_diff) = (diff.new.clone(), diff.buffer_diff.clone());
    let diff = diff.clone();
    cx.new(|cx| {
        let multi = cx.new(|cx| {
            let mut multi = MultiBuffer::singleton(new, cx);
            multi.add_diff(buffer_diff, cx);
            multi.set_all_diff_hunks_expanded(cx);
            multi
        });
        let mut editor = read_only_editor(multi, window, cx);
        editor.set_expand_all_diff_hunks(cx);
        mark(&mut editor, &diff, cx);
        editor
    })
}

/// The rows of `editor` with a line background, for a test.
pub fn tinted_rows(editor: &Entity<Editor>, window: &mut Window, cx: &mut App) -> Vec<u32> {
    editor.update(cx, |e, cx| e.highlighted_display_rows(window, cx).into_keys().map(|r| r.0).collect())
}

/// The word backgrounds of `editor` as (display row, columns, colour), for a test.
pub fn tinted_words(editor: &Entity<Editor>, window: &mut Window, cx: &mut App) -> Vec<(u32, Range<u32>, gpui::Hsla)> {
    editor.update(cx, |e, cx| {
        let snapshot = e.snapshot(window, cx);
        let words = e.background_highlights_in_range(Anchor::Min..Anchor::Max, &snapshot.display_snapshot, cx.theme());
        words.into_iter().map(|(r, color)| (r.start.row().0, r.start.column()..r.end.column(), color)).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    fn diff(old: &str, new: &str, cx: &mut TestAppContext) -> Diff {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let task = cx.update(|cx| Diff::compute(old.into(), new.into(), None, cx));
        cx.run_until_parked();
        cx.foreground_executor().block_test(task)
    }

    #[gpui::test]
    fn removed_rows_above_added_ones_are_tinted_one_row_each(cx: &mut TestAppContext) {
        let d = diff("a\nb\nc\n", "a\nB\nc\nd\n", cx);
        let cx = cx.add_empty_window();
        let editor = cx.update(|window, cx| inline_editor(&d, window, cx));
        cx.run_until_parked();
        assert_eq!(cx.update(|_, cx| crate::text(&editor, cx)), "a\nb\nB\nc\nd\n");
        assert_eq!(cx.update(|window, cx| tinted_rows(&editor, window, cx)), [1, 2, 4]);
    }

    #[gpui::test]
    fn changed_words_are_tinted_on_the_removed_and_the_added_row(cx: &mut TestAppContext) {
        let d = diff("a\nlet x = 1;\n", "a\nlet x = 22;\n", cx);
        let cx = cx.add_empty_window();
        let editor = cx.update(|window, cx| inline_editor(&d, window, cx));
        cx.run_until_parked();
        let colors = cx.update(|_, cx| diff_colors(cx.theme()));
        assert_eq!(cx.update(|window, cx| tinted_words(&editor, window, cx)), [(1, 8..9, colors.removed.word), (2, 8..10, colors.added.word)]);
    }
}

use std::ops::Range;
use std::sync::Arc;

use buffer_diff::BufferDiff;
use gpui::{App, AppContext as _, AsyncApp, Entity, Task};
use language::{Buffer, Language};
use text::{Point, ToPoint as _};

use crate::read_only_buffer;

/// One change: the rows it covers in the old text and in the new one (either may be empty), and the
/// changed words inside them as byte ranges of each text ([`crate::words::word_ranges`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_rows: Range<u32>,
    pub new_rows: Range<u32>,
    pub old_words: Vec<Range<usize>>,
    pub new_words: Vec<Range<usize>>,
}

/// The old and new texts of a file as read-only buffers, with Zed's diff of the two.
#[derive(Clone)]
pub struct Diff {
    pub old: Entity<Buffer>,
    pub new: Entity<Buffer>,
    pub buffer_diff: Entity<BufferDiff>,
    pub hunks: Arc<[Hunk]>,
}

/// Rows `range` covers: a range ending at a line's start stops before that line.
fn rows(range: Range<Point>) -> Range<u32> {
    let end = if range.end.column > 0 { range.end.row + 1 } else { range.end.row };
    range.start.row..end.max(range.start.row)
}

impl Diff {
    /// Builds the buffers now, diffs their lines and then the words inside each hunk on the background executor.
    pub fn compute(old: String, new: String, language: Option<Arc<Language>>, cx: &mut App) -> Task<Diff> {
        let (old_text, new_text): (Arc<str>, Arc<str>) = (Arc::from(old.as_str()), Arc::from(new.as_str()));
        let old = read_only_buffer(old, language.clone(), cx);
        let new = read_only_buffer(new, language, cx);
        let snapshot = new.read(cx).text_snapshot();
        let buffer_diff = cx.new(|cx| BufferDiff::new_with_base_text_buffer(&snapshot, old.clone(), cx));
        let base = old.read(cx).snapshot();
        let update = buffer_diff.read(cx).update_diff(snapshot, &base, Some(old_text.clone()), cx);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let update = update.await;
            let rows = cx.update(|cx| {
                buffer_diff.update(cx, |d, cx| d.set_snapshot(update, cx));
                Self::hunks_of(&buffer_diff, &new, cx)
            });
            let hunks = cx.background_spawn(async move { crate::words::with_words(&old_text, &new_text, rows) }).await;
            Diff { old, new, buffer_diff, hunks: hunks.into() }
        })
    }

    fn hunks_of(buffer_diff: &Entity<BufferDiff>, new: &Entity<Buffer>, cx: &App) -> Vec<Hunk> {
        let snapshot = new.read(cx).text_snapshot();
        let diff = buffer_diff.read(cx).snapshot(cx);
        let base = diff.base_text().clone();
        diff.hunks(&snapshot)
            .map(|h| {
                let old_range = base.offset_to_point(h.diff_base_byte_range.start)..base.offset_to_point(h.diff_base_byte_range.end);
                Hunk { old_rows: rows(old_range), new_rows: rows(h.range.clone()), old_words: Vec::new(), new_words: Vec::new() }
            })
            .collect()
    }

    /// Rows of the old text and of the new one, the last line counted when it has text.
    pub fn row_counts(&self, cx: &App) -> (u32, u32) {
        let count = |b: &Entity<Buffer>| {
            let max = b.read(cx).max_point();
            max.row + u32::from(max.column > 0)
        };
        (count(&self.old), count(&self.new))
    }

    /// The new text's point of a byte offset, for a test.
    pub fn new_point(&self, offset: usize, cx: &App) -> Point {
        offset.to_point(&self.new.read(cx).snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    fn compute(old: &str, new: &str, cx: &mut TestAppContext) -> Diff {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let task = cx.update(|cx| Diff::compute(old.into(), new.into(), None, cx));
        cx.run_until_parked();
        cx.foreground_executor().block_test(task)
    }

    #[gpui::test]
    fn hunks_cover_rows_on_both_sides(cx: &mut TestAppContext) {
        let d = compute("a\nb\nc\nd\ne\n", "a\nB\nc\ne\nf\n", cx);
        let rows: Vec<_> = d.hunks.iter().map(|h| (h.old_rows.clone(), h.new_rows.clone())).collect();
        assert_eq!(rows, [(1..2, 1..2), (3..4, 3..3), (5..5, 4..5)]);
    }

    #[gpui::test]
    fn an_added_file_is_one_hunk_of_new_rows(cx: &mut TestAppContext) {
        let d = compute("", "x\ny\n", cx);
        let rows: Vec<_> = d.hunks.iter().map(|h| (h.old_rows.clone(), h.new_rows.clone())).collect();
        assert_eq!(rows, [(0..0, 0..2)]);
        let d = compute("x\ny\n", "", cx);
        let rows: Vec<_> = d.hunks.iter().map(|h| (h.old_rows.clone(), h.new_rows.clone())).collect();
        assert_eq!(rows, [(0..2, 0..0)]);
    }

    #[gpui::test]
    fn a_changed_word_marks_its_line_and_the_word_on_both_sides(cx: &mut TestAppContext) {
        let d = compute("a\nlet x = 1;\nc\n", "a\nlet x = 2;\nc\n", cx);
        let h = &d.hunks[0];
        assert_eq!((h.old_rows.clone(), h.new_rows.clone()), (1..2, 1..2));
        assert_eq!((h.old_words.as_slice(), h.new_words.as_slice()), (&[10..11][..], &[10..11][..]), "{h:?}");
    }

    #[gpui::test]
    fn fragments_of_a_renamed_call_join_into_one_word(cx: &mut TestAppContext) {
        let (old, new) = ("{\n    await slot.ReacquireAsync(Aborted);\n}\n", "{\n    await slot.EndWaitAsync(Aborted);\n}\n");
        let d = compute(old, new, cx);
        let h = &d.hunks[0];
        let words = |text: &'static str, ranges: &[std::ops::Range<usize>]| ranges.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>();
        assert_eq!((words(old, &h.old_words), words(new, &h.new_words)), (vec!["ReacquireAsync"], vec!["EndWaitAsync"]));
    }

    #[gpui::test]
    fn added_and_removed_blocks_mark_whole_lines_and_no_words(cx: &mut TestAppContext) {
        let d = compute("a\nb\nc\n", "a\nx\ny\nb\n", cx);
        let rows: Vec<_> = d.hunks.iter().map(|h| (h.old_rows.clone(), h.new_rows.clone(), h.old_words.len() + h.new_words.len())).collect();
        assert_eq!(rows, [(1..1, 1..3, 0), (2..3, 4..4, 0)]);
    }
}

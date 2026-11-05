use editor::display_map::{BlockPlacement, BlockProperties, BlockStyle, HighlightKey};
use editor::{Editor, EditorEvent, RowHighlightOptions};
use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, DispatchPhase, Entity, HitboxBehavior, IntoElement as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, Subscription, Window, canvas, div, point,
    prelude::*, px,
};
use language::Buffer;
use multi_buffer::{Anchor, MultiBuffer};
use text::{Point, ToPoint as _};
use theme::ActiveTheme as _;

use crate::look::{diff_colors, filler};
use crate::ruler::{self, Mark, Ruler};
use crate::{Diff, Hunk, read_only_editor};

/// Removed rows' background on the old side.
struct Removed;
/// Added rows' background on the new side.
struct Added;

/// The old text left and the new right, each with line numbers, rows aligned by striped fillers and
/// scrolled together, and the change ruler at the right edge.
pub struct SplitDiff {
    pub old: Entity<Editor>,
    pub new: Entity<Editor>,
    /// Set while one side copies the other's scroll, so the copy does not echo back.
    syncing: bool,
    marks: Vec<Mark>,
    total_rows: u32,
    dragging: Rc<Cell<bool>>,
    _scrolls: [Subscription; 2],
}

/// A striped filler of `rows` rows above `row` of `editor`'s buffer, or below its last row when `row` is past it.
fn spacer(editor: &Editor, row: u32, rows: u32, cx: &App) -> BlockProperties<Anchor> {
    let snapshot = editor.buffer().read(cx).snapshot(cx);
    let max = snapshot.max_point();
    let placement = if row > max.row {
        BlockPlacement::Below(snapshot.anchor_after(max))
    } else {
        BlockPlacement::Above(snapshot.anchor_before(Point::new(row, 0)))
    };
    // The editor re-measures a block by its element, so the filler must be as tall as its rows.
    let render = std::sync::Arc::new(|cx: &mut editor::display_map::BlockContext| filler(cx.line_height * cx.height as f32).into_any_element());
    BlockProperties { placement, height: Some(rows), style: BlockStyle::Spacer, render, priority: 0 }
}

/// The spacers each side needs so both show a hunk on the same rows: `(old side, new side)` as `(row, rows)`.
pub(crate) fn spacers(hunks: &[Hunk]) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
    let (mut old, mut new) = (Vec::new(), Vec::new());
    for h in hunks {
        let (o, n) = (h.old_rows.len() as u32, h.new_rows.len() as u32);
        if o > n {
            new.push((h.new_rows.end, o - n));
        } else if n > o {
            old.push((h.old_rows.end, n - o));
        }
    }
    (old, new)
}

fn mark(editor: &mut Editor, hunks: &[Hunk], side: Side, buffer: &Entity<Buffer>, cx: &mut Context<Editor>) {
    let snapshot = editor.buffer().read(cx).snapshot(cx);
    let text = buffer.read(cx).snapshot();
    let mut words = Vec::new();
    for h in hunks {
        let (rows, ws) = match side {
            Side::Old => (&h.old_rows, &h.old_words),
            Side::New => (&h.new_rows, &h.new_words),
        };
        if rows.is_empty() {
            continue;
        }
        let range = crate::inline::whole_rows(&snapshot, rows.clone());
        match side {
            Side::Old => editor.highlight_rows::<Removed>(range, |cx| diff_colors(cx.theme()).removed.line, RowHighlightOptions::default(), cx),
            Side::New => editor.highlight_rows::<Added>(range, |cx| diff_colors(cx.theme()).added.line, RowHighlightOptions::default(), cx),
        }
        words.extend(ws.iter().map(|w| snapshot.anchor_before(w.start.to_point(&text))..snapshot.anchor_after(w.end.to_point(&text))));
    }
    let word: fn(&usize, &theme::Theme) -> gpui::Hsla = match side {
        Side::Old => |_, t| diff_colors(t).removed.word,
        Side::New => |_, t| diff_colors(t).added.word,
    };
    editor.highlight_background(HighlightKey::Editor, &words, word, cx);
}

#[derive(Clone, Copy)]
enum Side {
    Old,
    New,
}

fn side(diff: &Diff, side: Side, spacers: &[(u32, u32)], window: &mut Window, cx: &mut App) -> Entity<Editor> {
    let buffer = match side {
        Side::Old => diff.old.clone(),
        Side::New => diff.new.clone(),
    };
    let hunks = diff.hunks.clone();
    cx.new(|cx| {
        let multi = cx.new(|cx| MultiBuffer::singleton(buffer.clone(), cx));
        let mut editor = read_only_editor(multi, window, cx);
        let blocks: Vec<_> = spacers.iter().map(|(row, rows)| spacer(&editor, *row, *rows, cx)).collect();
        editor.insert_blocks(blocks, None, cx);
        mark(&mut editor, &hunks, side, &buffer, cx);
        editor
    })
}

impl SplitDiff {
    pub fn new(diff: &Diff, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (old_spacers, new_spacers) = spacers(&diff.hunks);
        let old = side(diff, Side::Old, &old_spacers, window, cx);
        let new = side(diff, Side::New, &new_spacers, window, cx);
        let follow = |from: &Entity<Editor>, to: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(from, window, move |this: &mut SplitDiff, from, e: &EditorEvent, window, cx| {
                if let EditorEvent::ScrollPositionChanged { .. } = e {
                    this.copy_scroll(from, &to, window, cx);
                }
            })
        };
        let scrolls = [follow(&old, new.clone(), window, cx), follow(&new, old.clone(), window, cx)];
        let total_rows = new.update(cx, |e, cx| e.display_snapshot(cx).max_point().row().0 + 1);
        SplitDiff { old, new, syncing: false, marks: ruler::marks(&diff.hunks), total_rows, dragging: Rc::default(), _scrolls: scrolls }
    }

    /// Scrolls both sides so the view centres on `fraction` of the ruler.
    pub fn scroll_to(&mut self, fraction: f32, window: &mut Window, cx: &mut Context<Self>) {
        let total = self.total_rows;
        self.new.update(cx, |e, cx| {
            let (x, visible) = (e.scroll_position(cx).x, e.visible_line_count().unwrap_or(0.));
            e.set_scroll_position(point(x, ruler::scroll_row_at(fraction, visible, total)), window, cx);
        });
    }

    fn ruler(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (y, visible) = self.new.update(cx, |e, cx| (e.scroll_position(cx).y, e.visible_line_count().unwrap_or(0.)));
        let status = cx.theme().status();
        let ruler = Ruler {
            marks: self.marks.clone(),
            total: self.total_rows,
            view: y..y + visible,
            removed: status.deleted,
            added: status.created,
            view_fill: cx.theme().colors().scrollbar_thumb_background,
        };
        let (this, dragging) = (cx.entity().downgrade(), self.dragging.clone());
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |bounds, hitbox, window, cx| {
                ruler.paint(bounds, window, cx);
                let at = move |y: Pixels| f32::from(y - bounds.origin.y) / f32::from(bounds.size.height).max(1.);
                let (down, held) = (this.clone(), dragging.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && e.button == MouseButton::Left && hitbox.is_hovered(window) {
                        held.set(true);
                        let _ = down.update(cx, |s, cx| s.scroll_to(at(e.position.y), window, cx));
                        cx.stop_propagation();
                    }
                });
                let held = dragging.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && held.get() && e.dragging() {
                        let _ = this.update(cx, |s, cx| s.scroll_to(at(e.position.y), window, cx));
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, _, _, _| dragging.set(false));
            },
        )
        .flex_none()
        .w(px(ruler::WIDTH))
        .h_full()
    }

    fn copy_scroll(&mut self, from: &Entity<Editor>, to: &Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        let position = from.update(cx, |e, cx| e.scroll_position(cx));
        to.update(cx, |e, cx| {
            if e.scroll_position(cx) != position {
                e.set_scroll_position(position, window, cx);
            }
        });
        self.syncing = false;
        cx.notify();
    }

    /// The display rows a test compares: each side's rows with its spacers.
    pub fn display_rows(&self, cx: &mut App) -> (u32, u32) {
        let rows = |e: &Entity<Editor>, cx: &mut App| e.update(cx, |e, cx| e.display_snapshot(cx).max_point().row().0);
        (rows(&self.old, cx), rows(&self.new, cx))
    }

    /// Both sides' scroll rows, for a test.
    pub fn scroll_rows(&self, cx: &mut App) -> (f64, f64) {
        let y = |e: &Entity<Editor>, cx: &mut App| e.update(cx, |e, cx| e.scroll_position(cx).y);
        (y(&self.old, cx), y(&self.new, cx))
    }
}

impl Render for SplitDiff {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().colors().border_variant;
        div()
            .size_full()
            .flex()
            .child(div().flex_1().min_w_0().h_full().border_r_1().border_color(border).child(self.old.clone()))
            .child(div().flex_1().min_w_0().h_full().border_r_1().border_color(border).child(self.new.clone()))
            .child(self.ruler(cx))
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::*;

    fn hunk(old: Range<u32>, new: Range<u32>) -> Hunk {
        Hunk { old_rows: old, new_rows: new, old_words: Vec::new(), new_words: Vec::new() }
    }

    #[test]
    fn spacers_pad_the_shorter_side_after_each_hunk() {
        let hunks = [hunk(1..2, 1..2), hunk(3..5, 3..3), hunk(6..6, 4..7)];
        let (old, new) = spacers(&hunks);
        assert_eq!(old, [(6, 3)]);
        assert_eq!(new, [(3, 2)]);
    }
}

#[cfg(test)]
mod aligned {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn both_sides_show_the_same_rows(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let cases = [("", "1
2
3
4
5
"), ("q
", "q
1
2
3
"), ("x
", "a
b
x
"), ("bye
", ""), ("a
b
", "a
b
c
d
"), ("a
b
c
d
", "a
d
"), ("a
b", "a
B
c")];
        for (old, new) in cases {
            let task = cx.update(|cx| Diff::compute(old.into(), new.into(), None, cx));
            cx.run_until_parked();
            let diff = cx.foreground_executor().block_test(task);
            let window = cx.add_window(|window, cx| SplitDiff::new(&diff, window, cx));
            cx.run_until_parked();
            let (l, r) = window.update(cx, |s, _, cx| s.display_rows(cx)).unwrap();
            assert_eq!(l, r, "{old:?} -> {new:?}");
        }
    }

    #[gpui::test]
    fn the_filler_is_as_tall_as_the_other_sides_lines_and_every_changed_row_is_tinted(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let task = cx.update(|cx| Diff::compute("a\nb\nc\n".into(), "a\nB\nx\ny\nc\n".into(), None, cx));
        cx.run_until_parked();
        let diff = cx.foreground_executor().block_test(task);
        let window = cx.add_window(|window, cx| SplitDiff::new(&diff, window, cx));
        cx.run_until_parked();
        let (filler, old, new) = window
            .update(cx, |s, window, cx| {
                let shown = s.display_rows(cx).0;
                (shown - diff.row_counts(cx).0, crate::tinted_rows(&s.old, window, cx), crate::tinted_rows(&s.new, window, cx))
            })
            .unwrap();
        assert_eq!(filler, 2, "the old side's filler stands for the two added lines");
        assert_eq!((old, new), (vec![1], vec![1, 2, 3]));
    }

    #[gpui::test]
    fn the_ruler_scrolls_both_sides_to_where_it_is_clicked(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let old: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let task = cx.update(|cx| Diff::compute(old.clone(), old.replace("line 150\n", "line 150 changed\n"), None, cx));
        cx.run_until_parked();
        let diff = cx.foreground_executor().block_test(task);
        let window = cx.add_window(|window, cx| SplitDiff::new(&diff, window, cx));
        cx.run_until_parked();
        window.update(cx, |s, window, cx| s.scroll_to(0.75, window, cx)).unwrap();
        cx.run_until_parked();
        let (old_y, new_y) = window.update(cx, |s, _, cx| s.scroll_rows(cx)).unwrap();
        assert_eq!(old_y, new_y);
        assert!(old_y > 100., "{old_y}");
    }

    #[gpui::test]
    fn one_side_scrolls_the_other(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let old: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 100\n", "line one hundred\n");
        let task = cx.update(|cx| Diff::compute(old, new, None, cx));
        cx.run_until_parked();
        let diff = cx.foreground_executor().block_test(task);
        let window = cx.add_window(|window, cx| SplitDiff::new(&diff, window, cx));
        cx.run_until_parked();
        for (from_old, y) in [(true, 40.), (false, 90.)] {
            window
                .update(cx, |s, window, cx| {
                    let side = if from_old { s.old.clone() } else { s.new.clone() };
                    side.update(cx, |e, cx| e.set_scroll_position(gpui::point(0., y), window, cx));
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(window.update(cx, |s, _, cx| s.scroll_rows(cx)).unwrap(), (y, y), "scrolled from the {} side", if from_old { "old" } else { "new" });
        }
    }
}

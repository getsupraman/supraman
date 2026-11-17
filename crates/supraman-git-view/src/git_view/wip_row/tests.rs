//! The graph's WIP row field through the real tab: a light one-line field inside the row's band.

use gpui::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px};
use supraman_gitman::fixture::Fixture;
use supraman_ui::clipped::cut_at;
use supraman_ui::text_input::TEXT_LEFT;

use super::{COUNTS_KEY, FIELD, FIELD_KEY, STILL_KEY};
use crate::git_view::graph::{GAP, ROW};
use crate::git_view::test_support::{self, Opts, click, repaint, select_wip};
use crate::git_view::{GitView, Selected};

#[gpui::test]
fn the_wip_rows_field_is_a_light_one_line_field_inside_the_rows_band_selected_or_not(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().editor().size(1200., 900.), cx);
    let band = |cx: &mut gpui::VisualTestContext| {
        let hits = repaint(&view, cx);
        let hits = hits.borrow();
        let (field, row) = (hits[FIELD_KEY].bounds, hits["git-row-0"].bounds);
        (field, row)
    };
    let still = band(cx);
    select_wip(&view, cx);
    let live = band(cx);
    for (field, row) in [still, live] {
        assert!(field.size.width >= gpui::px(FIELD), "at least GitKraken's width: {field:?}");
        assert!(field.size.height <= gpui::px(ROW - GAP), "one line inside the row's band: {field:?}");
        assert!(field.top() >= row.top() && field.bottom() <= row.bottom(), "the field {field:?} inside its row {row:?}");
    }
    assert_eq!(still.0, live.0, "selecting the row leaves the field where it was");
}

const LONG: &str = "Stage, discard and commit from the WIP panel, then push the branch and open a pull request for the reviewers to read";

/// The field, the counts and the message column as painted now.
fn message_cell(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
    let hits = repaint(view, cx);
    let hits = hits.borrow();
    (hits[FIELD_KEY].bounds, hits[COUNTS_KEY].bounds, hits["git-col-message"].bounds)
}

fn row_input(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Entity<supraman_ui::text_input::TextInput> {
    view.read_with(cx, |v, cx| v.wip_panel.read(cx).composer().map(|c| c.row.clone())).expect("the WIP row's field is made")
}

/// The still label shows the summary from the field's text start and is cut inside the field painted now.
fn shows_its_start_cut(field: Bounds<Pixels>, why: &str) {
    let cut = cut_at(STILL_KEY).unwrap_or_else(|| panic!("{why}: the summary is cut with an ellipsis"));
    assert!((cut.left() - field.left() - TEXT_LEFT).abs() < px(1.), "{why}: the label {cut:?} starts at the field's {field:?} text start");
    assert!((cut.size.width - (field.size.width - TEXT_LEFT * 2.)).abs() < px(1.), "{why}: the label {cut:?} fills this frame's field {field:?}");
}

#[gpui::test]
fn a_long_summary_shows_its_start_cut_until_the_field_holds_the_keys_and_the_field_fills_the_column(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new
").unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().editor().size(1600., 900.), cx);
    view.update(cx, |v, cx| v.draft_commit(LONG, cx));
    let input = row_input(&view, cx);
    let (field, counts, column) = message_cell(&view, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Wip));
    assert!(!cx.update(|window, cx| input.read(cx).is_focused(window)), "the row is selected, its field not focused");
    shows_its_start_cut(field, "selected");
    assert!(field.size.width > px(FIELD), "the field {field:?} takes the column's {column:?} room, not GitKraken's least width, beside {counts:?}");
    assert!(field.right() <= counts.left() && counts.right() <= column.right() + px(1.), "the counts {counts:?} whole right of the field {field:?} in {column:?}");

    view.update(cx, |v, cx| v.select_ix(1, cx));
    test_support::resize(1230., 900., cx);
    let (narrow, counts, narrower) = message_cell(&view, cx);
    assert!(narrower.size.width < column.size.width, "the column narrows with the window: {column:?} to {narrower:?}");
    assert!(((field.size.width - narrow.size.width) - (column.size.width - narrower.size.width)).abs() < px(1.), "the field {field:?} to {narrow:?} tracks the column {column:?} to {narrower:?}");
    shows_its_start_cut(narrow, "unselected");
    assert!(narrow.right() <= counts.left() && counts.right() <= narrower.right() + px(1.), "the counts {counts:?} stay whole in {narrower:?}");

    click(&view, FIELD_KEY, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Wip), "a click on the unselected row's field selects the row");
    assert!(!cx.update(|window, cx| input.read(cx).is_focused(window)), "and leaves the keys with the graph");
    click(&view, FIELD_KEY, cx);
    assert!(cx.update(|window, cx| input.read(cx).is_focused(window)), "a click on the selected row's field gives it the keys");
    test_support::redraw(&view, cx);
    assert!(input.read_with(cx, |i, _| i.caret_painted.get()), "the focused field is the input, its caret painted");
    assert!(test_support::painted(&view, COUNTS_KEY, cx), "the counts stay painted beside the input");
}

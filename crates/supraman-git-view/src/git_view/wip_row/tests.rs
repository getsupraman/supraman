//! The graph's WIP row field through the real tab: a light one-line field inside the row's band.

use gpui::TestAppContext;
use supraman_gitman::fixture::Fixture;

use super::{FIELD, FIELD_KEY};
use crate::git_view::graph::{GAP, ROW};
use crate::git_view::test_support::{self, Opts, repaint, select_wip};

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
        assert_eq!(field.size.width, gpui::px(FIELD), "GitKraken's width: {field:?}");
        assert!(field.size.height <= gpui::px(ROW - GAP), "one line inside the row's band: {field:?}");
        assert!(field.top() >= row.top() && field.bottom() <= row.bottom(), "the field {field:?} inside its row {row:?}");
    }
    assert_eq!(still.0, live.0, "selecting the row leaves the field where it was");
}

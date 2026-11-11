//! The graph's selection shows whether the graph has the keys.

use gpui::{Entity, Hsla, TestAppContext, VisualTestContext};
use theme::ActiveTheme as _;
use supraman_gitman::fixture::Fixture;
use supraman_ui::themes::selection;

use crate::git_view::test_support::{Opts, click, keys, open, row_of};
use crate::git_view::*;

fn shown(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (Option<usize>, Option<Hsla>) {
    view.read_with(cx, |v, _| (v.table.selected(), v.table.selection_shown()))
}

#[gpui::test]
fn the_graph_selection_is_inactive_while_another_list_has_the_keys(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let colors = cx.update(|_, cx| cx.theme().colors().clone());
    let (active, inactive) = (selection(&colors, true), selection(&colors, false));
    let second = row_of(&view, f.sha("second"), cx);
    click(&view, &format!("git-row-{second}"), cx);
    assert_eq!(shown(&view, cx), (Some(second), Some(active)), "a click gives the graph the keys");

    click(&view, "refpanel-row-local/feature", cx);
    let rename = row_of(&view, f.sha("rename"), cx);
    assert_eq!(shown(&view, cx), (Some(rename), Some(inactive)), "the ref panel has the keys; the graph shows its commit inactive");
    keys(&view, "down", cx);
    let (next, fill) = shown(&view, cx);
    assert_ne!(next, Some(rename), "Down moves the ref panel to its next ref, whose commit the graph shows");
    assert_eq!(fill, Some(inactive), "the ref panel keeps the keys");

    let next = next.expect("a commit is selected");
    click(&view, &format!("git-row-{next}"), cx);
    assert_eq!(shown(&view, cx), (Some(next), Some(active)));
    keys(&view, "down", cx);
    assert_eq!(shown(&view, cx), (Some(next + 1), Some(active)), "Down moves the graph again");
}

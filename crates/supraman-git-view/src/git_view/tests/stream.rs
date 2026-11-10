use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext, px};
use supraman_core::process;
use supraman_gitman::fixture::Generated;
use supraman_gitman::{FIRST_PAGE, NEXT_PAGE};

use crate::git_view::refresh::tests::{fresh, paged, rows};
use crate::git_view::seek::tests::tag;
use crate::git_view::test_support::{Opts, open, redraw};
use crate::git_view::GitView;

/// Waits up to 10 s for `pid` to end.
fn gone(pid: u32) -> bool {
    process::wait_any_exit(&[pid], Duration::from_secs(10)) || !process::alive(pid)
}

fn git_pid(view: &Entity<GitView>, cx: &mut VisualTestContext) -> u32 {
    view.read_with(cx, |v, cx| v.pages.read(cx).history.as_ref().and_then(|h| h.pid())).expect("the walk's git waits to write the rest")
}

#[gpui::test]
fn paging_ten_pages_reads_on_from_the_first_pages_git(cx: &mut TestAppContext) {
    let g = Generated::shared(FIRST_PAGE + 12 * NEXT_PAGE);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    let before = process::spawned_here();
    paged(&view, 10, cx);
    assert_eq!(process::spawned_here() - before, 0, "no git started for ten pages");
    assert_eq!(view.read_with(cx, |v, cx| v.pages.read(cx).commits), FIRST_PAGE + 10 * NEXT_PAGE);
    let wip = view.read_with(cx, |v, cx| v.pages.read(cx).rows[0].wip());
    assert_eq!(rows(&view, cx), fresh(&g, wip, FIRST_PAGE + 10 * NEXT_PAGE), "the rows a fresh read lays out");
}

#[gpui::test]
fn a_walk_change_and_closing_the_tab_kill_the_walks_git(cx: &mut TestAppContext) {
    let g = Generated::shared(FIRST_PAGE + 20 * NEXT_PAGE);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    let first = git_pid(&view, cx);
    let key = "refs/heads/topic/240".to_string();
    view.update(cx, |v, cx| v.ref_panel().update(cx, |p, cx| p.toggle_hidden(&[key], cx)));
    cx.run_until_parked();
    assert!(gone(first), "the old walk's git is gone");
    let second = git_pid(&view, cx);
    assert_ne!(first, second, "the new walk has its own");
    drop(view);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    assert!(gone(second), "a closed tab's git is gone");
}

#[gpui::test]
fn paging_after_the_tail_dropped_reads_what_a_fresh_read_does(cx: &mut TestAppContext) {
    let total = 12_000;
    let g = Generated::shared(total);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    view.update(cx, |v, cx| v.seek_sha(tag(&g, "t1").into(), cx));
    cx.run_until_parked();
    view.update(cx, |v, cx| {
        let top = v.pages.read(cx).rows[0].sha.clone();
        v.select_sha(&top, cx);
        v.table.set_scroll_top(px(0.));
    });
    redraw(&view, cx);
    let (kept, at) = view.read_with(cx, |v, cx| (v.pages.read(cx).commits, v.pages.read(cx).history.as_ref().map(|h| h.at())));
    assert!(at.is_some_and(|at| at > kept), "the tail dropped below what the walk read: {kept} rows, read to {at:?}");
    while view.read_with(cx, |v, cx| v.pages.read(cx).more) {
        view.update(cx, |v, cx| v.table.select(Some(v.pages.read(cx).rows.len() - 1)));
        paged(&view, 1, cx);
    }
    let wip = view.read_with(cx, |v, cx| v.pages.read(cx).rows[0].wip());
    assert_eq!(rows(&view, cx), fresh(&g, wip, total), "paged in again as a fresh read lays them out");
}

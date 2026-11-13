use gpui::{TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, Generated, git_in};
use supraman_gitman::{FIRST_PAGE, NEXT_PAGE, Row};

use crate::git_view::live::{FOCUS_DELAY, Refresh};
use crate::git_view::live::tests::{close, follow, following};
use crate::git_view::test_support::{Opts, open, redraw};
use crate::git_view::test_support::repaint;
use crate::git_view::*;

/// A commit with `message` on `parent`, as `branch`.
fn commit_on(g: &Generated, parent: &str, branch: &str, message: &str) -> String {
    let env = [("GIT_AUTHOR_NAME", "R".to_string()), ("GIT_AUTHOR_EMAIL", "r@example.test".to_string()), ("GIT_COMMITTER_NAME", "R".to_string()), ("GIT_COMMITTER_EMAIL", "r@example.test".to_string())];
    let sha = git_in(&g.git, &g.dir, &["commit-tree", &format!("{parent}^{{tree}}"), "-p", parent, "-m", message], &env, None).trim().to_string();
    git_in(&g.git, &g.dir, &["update-ref", &format!("refs/heads/{branch}"), &sha], &[], None);
    sha
}

fn refreshed(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.refresh_graph(None, cx));
    cx.run_until_parked();
    redraw(view, cx);
}

#[gpui::test]
fn refresh_widens_the_graph_column_for_new_lanes(cx: &mut TestAppContext) {
    let g = Generated::copy(60);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    let before = view.read_with(cx, |v, _| v.graph_max).expect("measured on open");
    for n in 0..4 {
        commit_on(&g, "main~5", &format!("side/{n}"), &format!("Side {n}"));
    }
    refreshed(&view, cx);
    let after = view.read_with(cx, |v, _| v.graph_max).expect("measured");
    assert!(after > before, "four new lanes widen the column's max: {before:?} to {after:?}");
}

#[gpui::test]
fn refresh_that_fails_says_so_over_the_graph_until_one_succeeds(cx: &mut TestAppContext) {
    let g = Generated::copy(30);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    let notice = |cx: &mut VisualTestContext| repaint(&view, cx).borrow().contains_key("git-follow-notice-dismiss");
    assert!(!notice(cx), "no notice after a good read");
    let rows = view.read_with(cx, |v, cx| v.pages.read(cx).rows.len());
    let (git, gone) = (g.dir.join(".git"), g.dir.join("git-gone"));
    std::fs::rename(&git, &gone).unwrap();
    refreshed(&view, cx);
    let said = view.read_with(cx, |v, _| v.follow.graph_failed.clone());
    std::fs::rename(&gone, &git).unwrap();
    assert!(said.is_some_and(|s| s.starts_with("Could not read the repository again")), "the failure is kept for the banner");
    assert!(notice(cx), "the banner is painted");
    assert_eq!(view.read_with(cx, |v, cx| v.pages.read(cx).rows.len()), rows, "the rows read before stay");
    refreshed(&view, cx);
    assert!(!notice(cx), "a good read takes the notice away");
}

fn refreshes(view: &Entity<GitView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.follow.refreshes)
}

fn activated(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.focus_regained(cx));
    cx.executor().advance_clock(FOCUS_DELAY * 2);
    cx.run_until_parked();
}

#[gpui::test]
fn refresh_of_a_hidden_tab_waits_until_it_is_shown(cx: &mut TestAppContext) {
    let g = Generated::copy(30);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    view.update(cx, |v, cx| v.set_shown(false, cx));
    let before = refreshes(&view, cx);
    activated(&view, cx);
    view.update(cx, |v, cx| v.live_refresh(Refresh::Wip { files: None }, cx));
    view.update(cx, |v, cx| v.live_refresh(Refresh::Graph, cx));
    cx.run_until_parked();
    assert_eq!(refreshes(&view, cx), before, "a hidden tab reads nothing");
    let top = commit_on(&g, "main", "main", "While hidden");
    view.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_eq!(refreshes(&view, cx), before + 1, "shown, it reads once what changed");
    assert!(view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().any(|r| r.sha.as_ref() == top)), "the commit made while hidden");
    view.update(cx, |v, cx| v.set_shown(false, cx));
    view.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_eq!(refreshes(&view, cx), before + 1, "nothing changed meanwhile, nothing read");
}

fn walks(view: &Entity<GitView>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |v, _| v.follow.graph_token)
}

#[gpui::test]
fn wip_read_that_lands_on_a_hidden_tab_walks_the_graph_once_shown(cx: &mut TestAppContext) {
    let g = Generated::copy(30);
    git_in(&g.git, &g.dir, &["reset", "-q", "--hard"], &[], None);
    git_in(&g.git, &g.dir, &["clean", "-q", "-fd"], &[], None);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    let has_wip = |cx: &mut VisualTestContext| view.update(cx, |v, cx| v.wip_row(cx).is_some());
    assert!(!has_wip(cx), "a clean checkout has no WIP row");
    let (walked, read) = (walks(&view, cx), refreshes(&view, cx));
    std::fs::write(g.dir.join("new.txt"), "new").unwrap();
    view.update(cx, |v, cx| v.refresh_wip(None, cx));
    view.update(cx, |v, cx| v.set_shown(false, cx));
    cx.run_until_parked();
    assert_eq!(walks(&view, cx), walked, "a hidden tab starts no walk when the WIP row comes");
    view.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_eq!((walks(&view, cx), refreshes(&view, cx)), (walked + 1, read + 1), "shown, it walks once");
    assert!(has_wip(cx), "the WIP row is there");
}

#[gpui::test]
fn refresh_on_activation_only_for_a_tab_without_watchers(cx: &mut TestAppContext) {
    let g = Generated::copy(30);
    follow(cx);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    following(&view, cx);
    let before = refreshes(&view, cx);
    activated(&view, cx);
    assert_eq!(refreshes(&view, cx), before, "the watchers cover the window coming back");
    close(view, cx);
}

pub(crate) fn paged(view: &Entity<GitView>, pages: usize, cx: &mut VisualTestContext) {
    for _ in 0..pages {
        view.update(cx, |v, cx| v.load_more(cx));
        cx.run_until_parked();
    }
}

/// The rows of the repo's first `count` commits laid out in one go.
pub(crate) fn fresh(g: &Generated, wip: bool, count: usize) -> Vec<Row> {
    let repo = g.repo();
    let refs = supraman_gitman::refs(&repo).unwrap();
    let history = supraman_gitman::history(&repo, &refs.tips(), 0, count).unwrap();
    supraman_gitman::Layout::new(&refs, wip).extend(&history.commits)
}

pub(crate) fn rows(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<Row> {
    view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().map(|r| r.row.clone()).collect())
}

#[gpui::test]
fn refresh_after_a_commit_on_top_keeps_the_rows_paged_below_and_the_place(cx: &mut TestAppContext) {
    let total = FIRST_PAGE + 3 * NEXT_PAGE;
    let g = Generated::copy(total);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    paged(&view, 2, cx);
    let loaded = FIRST_PAGE + 2 * NEXT_PAGE;
    assert_eq!(view.read_with(cx, |v, cx| v.pages.read(cx).commits), loaded);
    let row = px(graph::ROW);
    let (sha, ix, top) = view.update(cx, |v, cx| {
        let sha = v.pages.read(cx).rows[loaded - 100].sha.clone();
        v.select_sha(&sha, cx);
        v.table.set_scroll_top(row * (loaded - 110) as f32);
        (sha, v.table.selected().unwrap(), v.table.scroll_top())
    });
    redraw(&view, cx);
    let wip = view.read_with(cx, |v, cx| v.pages.read(cx).rows[0].wip());
    let on_top = commit_on(&g, "main", "main", "On top");
    refreshed(&view, cx);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.pages.read(cx).commits, loaded + 1, "the paged rows stay below the new commit");
        assert!(v.pages.read(cx).rows.iter().any(|r| r.sha.as_ref() == on_top));
        assert_eq!(v.selected(cx), Some(Selected::Commit(sha.clone())), "the same commit selected");
        assert_eq!(v.table.selected(), Some(ix + 1), "one row down");
        assert_eq!(v.table.scroll_top(), top + row, "the scroll followed it");
        assert!(v.pages.read(cx).more);
    });
    assert_eq!(rows(&view, cx), fresh(&g, wip, loaded + 1), "laid out as a fresh read of as many commits");
    paged(&view, 1, cx);
    assert_eq!(rows(&view, cx), fresh(&g, wip, total + 1), "and pages on from there as a fresh read does");
}

#[gpui::test]
fn refresh_after_a_rewrite_or_a_hidden_ref_reads_the_first_page_afresh(cx: &mut TestAppContext) {
    let g = Generated::copy(FIRST_PAGE + 3 * NEXT_PAGE);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    paged(&view, 2, cx);
    let rewritten = commit_on(&g, "main~1", "main", "Rewritten");
    refreshed(&view, cx);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.pages.read(cx).commits, FIRST_PAGE, "the old tip is gone: the first page only");
        assert!(v.pages.read(cx).more && v.pages.read(cx).rows.iter().any(|r| r.sha.as_ref() == rewritten));
    });
    paged(&view, 2, cx);
    let key = "refs/heads/topic/240".to_string();
    view.update(cx, |v, cx| v.ref_panel().update(cx, |p, cx| p.toggle_hidden(&[key], cx)));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, cx| v.pages.read(cx).commits), FIRST_PAGE, "a hidden ref: the first page only");
}

/// The selected commit's row, the scroll, and where the row sits below the view's top.
fn place(view: &Entity<GitView>, sha: &str, cx: &mut VisualTestContext) -> (usize, Pixels, Pixels) {
    view.read_with(cx, |v, cx| {
        let ix = v.table.selected().expect("a row selected");
        assert_eq!(v.pages.read(cx).rows[ix].sha.as_ref(), sha, "the selection stays on its commit");
        let top = v.table.scroll_top();
        (ix, top, px(graph::ROW) * ix as f32 - top)
    })
}

fn select_at(view: &Entity<GitView>, ix: usize, top: Pixels, cx: &mut VisualTestContext) -> String {
    let sha = view.update(cx, |v, cx| {
        let sha = v.pages.read(cx).rows[ix].sha.to_string();
        v.select_sha(&sha, cx);
        v.table.set_scroll_top(top);
        sha
    });
    redraw(view, cx);
    sha
}

#[gpui::test]
fn refresh_with_new_commits_on_top_follows_them_while_the_selection_stays_in_view(cx: &mut TestAppContext) {
    let g = Generated::copy(200);
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    let row = px(graph::ROW);
    let first = view.read_with(cx, |v, cx| usize::from(v.pages.read(cx).rows[0].wip()));

    let sha = select_at(&view, first, px(0.), cx);
    commit_on(&g, "main", "main", "An agent's commit");
    refreshed(&view, cx);
    let (ix, top, _) = place(&view, &sha, cx);
    assert_eq!((ix, top), (first + 1, px(0.)), "at the top it stays at the top, the new commit in view");

    let sha = select_at(&view, first + 5, row * 3., cx);
    commit_on(&g, "main", "main", "Another");
    refreshed(&view, cx);
    let (ix, top, _) = place(&view, &sha, cx);
    assert_eq!((ix, top), (first + 6, px(0.)), "scrolled a little: to the top, the selected row still wholly in view");

    let sha = select_at(&view, first + 105, row * 100., cx);
    let (_, before, offset) = place(&view, &sha, cx);
    commit_on(&g, "main", "main", "A third");
    refreshed(&view, cx);
    let (ix, top, now) = place(&view, &sha, cx);
    assert_eq!((ix, top, now), (first + 106, before + row, offset), "far down: the selected row keeps its place on screen");
    view.read_with(cx, |v, cx| {
        assert_eq!(v.selected(cx), Some(Selected::Commit(sha.clone().into())), "the same commit selected");
        assert_eq!(v.pages.read(cx).rows.iter().filter(|r| ["An agent's commit", "Another", "A third"].contains(&&*r.summary)).count(), 3, "each new commit has its row");
    });
}

#[gpui::test]
fn refresh_runs_once_after_the_window_comes_back(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let before = view.read_with(cx, |v, _| v.follow.refreshes);
    view.update(cx, |v, cx| v.focus_regained(cx));
    view.update(cx, |v, cx| v.focus_regained(cx));
    cx.executor().advance_clock(FOCUS_DELAY / 2);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.follow.refreshes), before, "not before the delay");
    cx.executor().advance_clock(FOCUS_DELAY);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.follow.refreshes), before + 1, "one refresh for two activations");
}

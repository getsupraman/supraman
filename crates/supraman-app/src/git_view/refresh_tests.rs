use gpui::{TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Generated, git_in};
use supraman_gitman::{FIRST_PAGE, NEXT_PAGE, Row};

use super::live::{FOCUS_DELAY, Refresh};
use super::live_tests::{close, follow, following};
use super::tests::{open, redraw};
use super::wip::tests::repaint;
use super::*;

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
    let g = Generated::new(60);
    let (view, cx) = open(&g.dir, cx);
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
    let g = Generated::new(30);
    let (view, cx) = open(&g.dir, cx);
    let notice = |cx: &mut VisualTestContext| repaint(&view, cx).borrow().contains_key("git-follow-notice-dismiss");
    assert!(!notice(cx), "no notice after a good read");
    let rows = view.read_with(cx, |v, _| v.rows.len());
    let (git, gone) = (g.dir.join(".git"), g.dir.join("git-gone"));
    std::fs::rename(&git, &gone).unwrap();
    refreshed(&view, cx);
    let said = view.read_with(cx, |v, _| v.follow.graph_failed.clone());
    std::fs::rename(&gone, &git).unwrap();
    assert!(said.is_some_and(|s| s.starts_with("Could not read the repository again")), "the failure is kept for the banner");
    assert!(notice(cx), "the banner is painted");
    assert_eq!(view.read_with(cx, |v, _| v.rows.len()), rows, "the rows read before stay");
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
    let g = Generated::new(30);
    let (view, cx) = open(&g.dir, cx);
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
    assert!(view.read_with(cx, |v, _| v.rows.iter().any(|r| r.sha.as_ref() == top)), "the commit made while hidden");
    view.update(cx, |v, cx| v.set_shown(false, cx));
    view.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_eq!(refreshes(&view, cx), before + 1, "nothing changed meanwhile, nothing read");
}

#[gpui::test]
fn refresh_on_activation_only_for_a_tab_without_watchers(cx: &mut TestAppContext) {
    let g = Generated::new(30);
    follow(cx);
    let (view, cx) = open(&g.dir, cx);
    following(&view, cx);
    let before = refreshes(&view, cx);
    activated(&view, cx);
    assert_eq!(refreshes(&view, cx), before, "the watchers cover the window coming back");
    close(view, cx);
}

fn paged(view: &Entity<GitView>, pages: usize, cx: &mut VisualTestContext) {
    for _ in 0..pages {
        view.update(cx, |v, cx| v.load_more(cx));
        cx.run_until_parked();
    }
}

/// The rows of the repo's first `count` commits laid out in one go.
fn fresh(g: &Generated, wip: bool, count: usize) -> Vec<Row> {
    let repo = g.repo();
    let refs = supraman_gitman::refs(&repo).unwrap();
    let history = supraman_gitman::history(&repo, &refs.tips(), 0, count).unwrap();
    supraman_gitman::Layout::new(&refs, wip).extend(&history.commits)
}

fn rows(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<Row> {
    view.read_with(cx, |v, _| v.rows.iter().map(|r| r.row.clone()).collect())
}

#[gpui::test]
fn refresh_after_a_commit_on_top_keeps_the_rows_paged_below_and_the_place(cx: &mut TestAppContext) {
    let total = FIRST_PAGE + 3 * NEXT_PAGE;
    let g = Generated::new(total);
    let (view, cx) = open(&g.dir, cx);
    paged(&view, 2, cx);
    let loaded = FIRST_PAGE + 2 * NEXT_PAGE;
    assert_eq!(view.read_with(cx, |v, _| v.commits), loaded);
    let row = px(graph::ROW);
    let (sha, ix, top) = view.update(cx, |v, cx| {
        let sha = v.rows[loaded - 100].sha.clone();
        v.select_sha(&sha, cx);
        v.table.set_scroll_top(row * (loaded - 110) as f32);
        (sha, v.table.selected().unwrap(), v.table.scroll_top())
    });
    redraw(&view, cx);
    let wip = view.read_with(cx, |v, _| v.rows[0].wip());
    let on_top = commit_on(&g, "main", "main", "On top");
    refreshed(&view, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.commits, loaded + 1, "the paged rows stay below the new commit");
        assert!(v.rows.iter().any(|r| r.sha.as_ref() == on_top));
        assert_eq!(v.selected(), Some(Selected::Commit(sha.clone())), "the same commit selected");
        assert_eq!(v.table.selected(), Some(ix + 1), "one row down");
        assert_eq!(v.table.scroll_top(), top + row, "the scroll followed it");
        assert!(v.more);
    });
    assert_eq!(rows(&view, cx), fresh(&g, wip, loaded + 1), "laid out as a fresh read of as many commits");
    paged(&view, 1, cx);
    assert_eq!(rows(&view, cx), fresh(&g, wip, total + 1), "and pages on from there as a fresh read does");
}

#[gpui::test]
fn refresh_after_a_rewrite_or_a_hidden_ref_reads_the_first_page_afresh(cx: &mut TestAppContext) {
    let g = Generated::new(FIRST_PAGE + 3 * NEXT_PAGE);
    let (view, cx) = open(&g.dir, cx);
    paged(&view, 2, cx);
    let rewritten = commit_on(&g, "main~1", "main", "Rewritten");
    refreshed(&view, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.commits, FIRST_PAGE, "the old tip is gone: the first page only");
        assert!(v.more && v.rows.iter().any(|r| r.sha.as_ref() == rewritten));
    });
    paged(&view, 2, cx);
    let key = "refs/heads/topic/240".to_string();
    view.update(cx, |v, cx| v.ref_panel().update(cx, |p, cx| p.toggle_hidden(&[key], cx)));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.commits), FIRST_PAGE, "a hidden ref: the first page only");
}

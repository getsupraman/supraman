use gpui::{TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Generated, git_in};

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

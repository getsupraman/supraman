use gpui::{TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Generated, git_in};

use super::tests::{open, redraw};
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


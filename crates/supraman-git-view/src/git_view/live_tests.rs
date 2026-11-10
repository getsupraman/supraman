use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext};
use supraman_core::watch::Batch;
use supraman_gitman::Dirs;
use supraman_gitman::fixture::{Fixture, Generated, git_in};

use super::cells::ref_cell;
use super::diff::Content;
use super::live::{Debounces, FOCUS_DELAY, Refresh, git_class, git_refresh, sent, work_files};
use super::tests::{open, redraw};
use super::wip::Section;
use super::wip::tests::{open_file, open_with_editor, select_wip};
use super::*;

const DEADLINE: Duration = Duration::from_secs(10);

/// Runs the app until `done`, failing after `within`; woken by each refresh the watchers' threads send.
fn until(what: &str, within: Duration, cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) -> Duration {
    let start = Instant::now();
    loop {
        let seen = sent::count();
        cx.run_until_parked();
        if done(cx) {
            return start.elapsed();
        }
        assert!(sent::past(seen, within.saturating_sub(start.elapsed())), "{what}: not within {within:?}");
    }
}

/// Tabs this test opens from here on follow their repositories.
pub(super) fn follow(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(|cx| cx.set_global(super::refresh::Follows(Debounces { git: Duration::from_millis(20), work: Duration::from_millis(50) })));
}

pub(super) fn following(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    until("the watchers start", DEADLINE, cx, |cx| view.read_with(cx, |v, _| v.follow.live().is_some()));
}

/// Closes the test's window and waits for the watchers to end, so none wakes the app after the test.
pub(super) fn close(view: Entity<GitView>, cx: &mut VisualTestContext) -> Vec<supraman_core::watch::Stopped> {
    let stopped = view.read_with(cx, |v, _| v.follow.live().expect("following").stopped());
    drop(view);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    assert!(stopped.iter().all(|s| s.wait(Duration::from_secs(5))), "the watchers' threads end with the tab");
    cx.run_until_parked();
    stopped
}

#[gpui::test]
fn live_refresh_shows_a_new_commit_and_its_refs_within_a_second_and_an_edit_within_three(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    follow(cx);
    let (view, cx) = open(&f.dir, cx);
    following(&view, cx);
    f.git(&["commit", "-q", "--no-verify", "-m", "Made while watched"]);
    let head = f.git(&["rev-parse", "HEAD"]);
    let took = until("the new commit", DEADLINE, cx, |cx| {
        view.read_with(cx, |v, _| {
            v.rows.iter().find(|r| r.sha.as_ref() == head).is_some_and(|r| ref_cell(&r.row, false).first.is_some_and(|g| g.name == "main" && g.head()))
        })
    });
    eprintln!("the commit and main on it after {took:?}");
    let ahead = view.read_with(cx, |v, _| v.rows.iter().find(|r| r.sha.as_ref() == f.sha("ahead")).map(|r| r.row.refs.len()));
    assert_eq!(ahead, Some(0), "main moved off its old commit");

    let added = |v: &GitView| v.wip.map_or(0, |w| w.added);
    let before = view.read_with(cx, |v, _| added(v));
    f.write("live.txt", b"new\n");
    let took = until("the edit in the WIP", DEADLINE, cx, |cx| view.read_with(cx, |v, _| added(v) == before + 1));
    eprintln!("the edit after {took:?}");
    close(view, cx);
}

#[gpui::test]
fn live_refresh_keeps_the_selection_and_the_scroll_on_the_same_commit(cx: &mut TestAppContext) {
    let g = Generated::copy(400);
    let (view, cx) = open(&g.dir, cx);
    let row = px(graph::ROW);
    let (sha, ix, top) = view.update(cx, |v, cx| {
        let sha = v.rows[100].sha.clone();
        v.select_sha(&sha, cx);
        v.table.set_scroll_top(row * 90. + px(5.));
        (sha, v.table.selected().unwrap(), v.table.scroll_top())
    });
    redraw(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.table.scroll_top()), top, "the scroll as set");
    let top_sha = view.read_with(cx, |v, _| v.rows[90].sha.clone());
    let env = [("GIT_AUTHOR_NAME", "Live".to_string()), ("GIT_AUTHOR_EMAIL", "live@example.test".to_string()), ("GIT_COMMITTER_NAME", "Live".to_string()), ("GIT_COMMITTER_EMAIL", "live@example.test".to_string())];
    for n in 0..3 {
        let sha = git_in(&g.git, &g.dir, &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", &format!("On top {n}")], &env, None);
        git_in(&g.git, &g.dir, &["update-ref", "refs/heads/main", sha.trim()], &[], None);
    }
    view.update(cx, |v, cx| v.refresh_graph(None, cx));
    cx.run_until_parked();
    redraw(&view, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selected(), Some(Selected::Commit(sha.clone())), "the same commit selected");
        assert_eq!(v.table.selected(), Some(ix + 3), "at its new row, three down");
        assert_eq!(v.table.scroll_top(), top + row * 3., "the scroll followed it");
        assert_eq!(v.rows[ix + 3 - 10].sha, top_sha, "the same commit on top of the view");
        assert_eq!(v.rows.iter().filter(|r| r.summary.starts_with("On top")).count(), 3);
    });
}

#[gpui::test]
fn live_refresh_keeps_the_open_diff_and_reloads_it_when_its_file_changes(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    follow(cx);
    let (view, cx) = open_with_editor(&f.dir, cx);
    following(&view, cx);
    select_wip(&view, cx);
    let (diff, _, new) = open_file(&view, Section::Unstaged, "u.txt", cx);
    assert_eq!(new, "untracked\n");
    f.write("u.txt", b"untracked\nand more\n");
    until("the diff reloaded", DEADLINE, cx, |cx| {
        diff.read_with(cx, |d, cx| matches!(d.content(), Content::Text(t) if t.diff.new.read(cx).text() == "untracked\nand more\n"))
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.diff().map(Entity::entity_id), Some(diff.entity_id()), "the same viewer stays open");
        assert_eq!(v.selected(), Some(Selected::Wip));
    });
    drop(diff);
    close(view, cx);
}

#[gpui::test]
fn live_refresh_runs_once_after_the_window_comes_back(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, cx);
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

#[gpui::test]
fn live_refresh_stops_the_watchers_when_the_tab_closes(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    follow(cx);
    let (view, cx) = open(&f.dir, cx);
    following(&view, cx);
    let live = view.read_with(cx, |v, _| v.follow.live().unwrap().stopped());
    assert_eq!(live.len(), 2, "the git dir's watcher and the work dir's");
    assert!(live.iter().all(|s| !s.stopped()), "watching while the tab is open");
    close(view, cx);
}

fn dirs(top: &str, git_dir: &str, common_dir: &str) -> Dirs {
    Dirs { top: top.into(), git_dir: git_dir.into(), common_dir: common_dir.into() }
}

fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, part| p.join(part))
}

#[test]
fn live_refresh_classifies_each_change_in_the_git_dirs() {
    let main = dirs("C:/r", "C:/r/.git", "C:/r/.git");
    let wt = dirs("C:/wt", "C:/r/.git/worktrees/wt", "C:/r/.git");
    let git = Path::new("C:/r/.git");
    let wip = Some(Refresh::Wip { files: None });
    for (rel, in_main, in_wt) in [
        ("HEAD", Some(Refresh::Graph), Some(Refresh::Graph)),
        ("refs/heads/main", Some(Refresh::Graph), Some(Refresh::Graph)),
        ("refs/tags/v1", Some(Refresh::Graph), Some(Refresh::Graph)),
        ("packed-refs", Some(Refresh::Graph), Some(Refresh::Graph)),
        ("logs/HEAD", Some(Refresh::Graph), Some(Refresh::Graph)),
        ("index", wip.clone(), None),
        ("MERGE_HEAD", wip.clone(), None),
        ("worktrees/wt/HEAD", Some(Refresh::Graph), Some(Refresh::Graph)),
        ("worktrees/wt/index", None, wip.clone()),
        ("worktrees/wt/rebase-merge/done", None, wip.clone()),
        ("refs/heads/main.lock", None, None),
        ("index.lock", None, None),
        ("objects/ab/cdef", None, None),
        ("logs/refs/heads/main", None, None),
        ("config", None, None),
    ] {
        let path = under(git, rel);
        assert_eq!((git_class(&main, &path), git_class(&wt, &path)), (in_main, in_wt), "{rel}");
    }
    let batch = |rels: &[&str]| Batch { paths: rels.iter().map(|r| under(git, r)).collect(), rescan: false };
    assert_eq!(git_refresh(&main, &batch(&["index", "refs/heads/x", "objects/1"])), Some(Refresh::Graph), "the graph's refresh covers the WIP's");
    assert_eq!(git_refresh(&main, &batch(&["index.lock", "index"])), wip);
    assert_eq!(git_refresh(&main, &batch(&["objects/1"])), None);
    assert_eq!(git_refresh(&main, &Batch { paths: Vec::new(), rescan: true }), Some(Refresh::Graph), "lost changes read everything");
}

#[test]
fn live_refresh_takes_the_work_dirs_files_without_the_git_dir() {
    let main = dirs("C:/r", "C:/r/.git", "C:/r/.git");
    let top = Path::new("C:/r");
    let batch = Batch { paths: ["src/a.rs", ".git/index", ".git/refs/heads/x", "", "target/out.bin", "docs"].iter().map(|r| if r.is_empty() { top.to_path_buf() } else { under(top, r) }).collect(), rescan: false };
    assert_eq!(work_files(&main, &batch), ["src/a.rs", "target/out.bin", "docs"], "ignored ones go to git check-ignore next");
}

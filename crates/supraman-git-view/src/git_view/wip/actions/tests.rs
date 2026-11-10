//! The WIP panel's writes through the real tab: its buttons, keys, Confirm and the write gate, on fixture copies (C196).


use gpui::{Context, Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::journal;
use supraman_gitman::{ChangeKind, FileChange};

use crate::git_view::wip::compose::counter_colour;
use crate::git_view::test_support::{self, Opts, click, keys, painted, redraw, select_wip, toast};
use crate::git_view::wip::*;
use crate::git_view::undo::UNDO_KEY;
use crate::git_view::write_gate::WORKING;
use crate::git_view::GitView;
use crate::occupancy;

/// A Git tab of `folder`, its WIP row selected, whose write gate reads `live`.
fn open<'a>(folder: &std::path::Path, live: occupancy::Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    let (view, cx) = open_as(folder, live, cx);
    select_wip(&view, cx);
    (view, cx)
}

/// A Git tab of `folder` as it opens, whose write gate reads `live`.
pub(crate) fn open_as<'a>(folder: &std::path::Path, live: occupancy::Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    test_support::open(folder, Opts::default().live(live).editor().size(1200., 900.), cx)
}

pub(crate) fn panel<R>(view: &Entity<GitView>, cx: &mut VisualTestContext, read: impl FnOnce(&WipPanel, &gpui::App) -> R) -> R {
    view.read_with(cx, |v, cx| read(v.wip_panel().read(cx), cx))
}

fn lists(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (Vec<String>, Vec<String>) {
    let shown = panel(view, cx, |p, _| p.shown().expect("the working directory is shown"));
    let paths = |files: &[FileChange]| files.iter().map(|f| f.path.clone()).collect();
    (paths(&shown.unstaged), paths(&shown.staged))
}

pub(crate) fn texts(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (String, String, String) {
    panel(view, cx, |p, cx| {
        let c = p.composer().expect("the commit area is made");
        (c.summary.read(cx).text().to_string(), c.row.read(cx).text().to_string(), c.description.read(cx).text().to_string())
    })
}

fn focus(view: &Entity<GitView>, which: fn(&compose::Composer) -> &Entity<supraman_ui::text_input::TextInput>, cx: &mut VisualTestContext) {
    let input = panel(view, cx, |p, _| which(p.composer().unwrap()).clone());
    cx.update(|window, cx| input.update(cx, |i, cx| i.focus(window, cx)));
    redraw(view, cx);
}

fn focused(view: &Entity<GitView>, which: fn(&compose::Composer) -> &Entity<supraman_ui::text_input::TextInput>, cx: &mut VisualTestContext) -> bool {
    let input = panel(view, cx, |p, _| which(p.composer().unwrap()).clone());
    cx.update(|window, cx| input.read(cx).is_focused(window))
}

fn head(f: &Fixture) -> String {
    f.git(&["log", "-1", "--format=%B"])
}

fn undo(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.undo_bar().update(cx, |u, cx| u.undo(cx)));
    cx.run_until_parked();
    redraw(view, cx);
}

#[gpui::test]
fn wip_write_file_and_all_buttons_stage_unstage_and_discard_after_asking(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    assert_eq!(lists(&view, cx), (vec!["a.txt".into(), "b.txt".into(), "u.txt".into()], vec!["a.txt".into(), "renamed.txt".into(), "s.txt".into()]));

    click(&view, "wip-stage-u.txt", cx);
    assert!(lists(&view, cx).1.contains(&"u.txt".to_string()), "Stage File stages the untracked file");
    click(&view, "wip-unstage-renamed.txt", cx);
    let (unstaged, staged) = lists(&view, cx);
    assert!(unstaged.contains(&"renamed.txt".to_string()) && !staged.contains(&"renamed.txt".to_string()), "Unstage File takes the rename's old path along");
    assert!(f.git(&["ls-files", "--", "new.txt"]) == "new.txt", "the old path is back in the index");
    click(&view, "wip-stage-all", cx);
    assert_eq!(lists(&view, cx).0, Vec::<String>::new(), "Stage All Changes");
    click(&view, "wip-unstage-all", cx);
    assert_eq!(lists(&view, cx).1, Vec::<String>::new(), "Unstage All Changes");

    click(&view, "wip-discard-u.txt", cx);
    assert_eq!(panel(&view, cx, |p, _| p.asking().map(str::to_string)).as_deref(), Some("Discard the changes to 1 file? You can undo it."));
    assert!(f.dir.join("u.txt").exists(), "nothing goes before the answer");
    click(&view, "wip-confirm-yes", cx);
    assert!(!f.dir.join("u.txt").exists());
    assert_eq!(toast(&view, cx).as_deref(), Some("Discarded the changes to 1 file"));
    undo(&view, cx);
    assert!(f.dir.join("u.txt").exists(), "Undo brings the discarded file back");

    let before = f.git(&["status", "--porcelain", "--untracked-files=all"]);
    click(&view, "wip-discard-all", cx);
    assert!(panel(&view, cx, |p, _| p.asking().is_some_and(|q| q.starts_with("Discard all changes? Staged, unstaged and new untracked files go; ignored files stay."))));
    click(&view, "wip-confirm-cancel", cx);
    assert_eq!(f.git(&["status", "--porcelain", "--untracked-files=all"]), before, "cancel discards nothing");
    click(&view, "wip-discard-all", cx);
    click(&view, "wip-confirm-yes", cx);
    assert_eq!(f.git(&["status", "--porcelain", "--untracked-files=all"]), "", "everything is discarded");
    undo(&view, cx);
    assert_eq!(f.git(&["status", "--porcelain", "--untracked-files=all"]), before, "Undo restores all of it");
}

#[gpui::test]
fn wip_write_a_session_in_the_worktree_is_not_asked_about(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let live = occupancy::Live::default();
    live.set_sessions(vec![occupancy::LiveSession { name: "fm-alpha".into(), cwd: f.dir.join("src"), activity: supraman_ui::marks::Activity::Working, attach: None }]);
    let (view, cx) = open(&f.dir, live, cx);
    click(&view, "wip-stage-u.txt", cx);
    let (unstaged, staged) = lists(&view, cx);
    assert!(staged.contains(&"u.txt".to_string()) && unstaged.contains(&"b.txt".to_string()), "the write ran without a question");

    let file = FileChange { kind: ChangeKind::Deleted, path: "b.txt".into(), old_path: None, similarity: None };
    view.update(cx, |v, cx| gate_free_stage(v, file.clone(), cx));
    cx.run_until_parked();
    assert!(lists(&view, cx).1.contains(&"b.txt".to_string()));
}

fn gate_free_stage(v: &mut GitView, file: FileChange, cx: &mut Context<GitView>) {
    v.wip_panel().update(cx, |p, cx| p.stage(vec![file], cx));
}

#[gpui::test]
fn wip_write_a_second_write_is_not_queued_while_one_runs(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    let file = |path: &str| FileChange { kind: ChangeKind::Untracked, path: path.into(), old_path: None, similarity: None };
    f.write("v.txt", b"v\n");
    let busy = view.update(cx, |v, cx| {
        v.wip_panel().update(cx, |p, cx| {
            p.stage(vec![file("u.txt")], cx);
            let busy = p.busy(cx);
            p.stage(vec![file("v.txt")], cx);
            busy
        })
    });
    assert_eq!(busy, Some(WORKING));
    cx.run_until_parked();
    redraw(&view, cx);
    let staged = f.at(&f.dir, &["diff", "--cached", "--name-only"]);
    assert!(staged.lines().any(|l| l == "u.txt") && !staged.lines().any(|l| l == "v.txt"), "the click while one ran queued nothing: {staged}");
}

fn hook(f: &Fixture) {
    std::fs::write(f.dir.join(".git/hooks/commit-msg"), "#!/bin/sh\necho 'commit-msg: no ticket' >&2\nexit 3\n").unwrap();
}

#[gpui::test]
fn wip_commit_area_counts_commits_shows_a_hook_failure_skips_hooks_and_amends(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    hook(&f);
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    let words = |cx: &mut VisualTestContext| panel(&view, cx, |p, cx| p.commit_button(false, cx).map(|(w, _)| w)).unwrap();
    assert_eq!(words(cx), "Type a commit message");
    assert!(painted(&view, "wip-commit", cx), "the commit button");
    assert!(painted(&view, "wip-commit-options", cx) && !painted(&view, "wip-skip-hooks", cx), "Commit options folded");
    click(&view, "wip-commit-options", cx);
    assert!(painted(&view, "wip-skip-hooks", cx), "Skip hooks where the repo has commit hooks");

    focus(&view, |c| &c.summary, cx);
    cx.simulate_input(&"x".repeat(75));
    redraw(&view, cx);
    assert_eq!(texts(&view, cx).1, "x".repeat(75), "the WIP row shows the summary");
    cx.update(|_, cx| {
        let theme = theme::ActiveTheme::theme(cx);
        assert_eq!(counter_colour(compose::left(&"x".repeat(75)), cx), theme.status().warning, "past 72 the counter warns");
        assert_eq!(counter_colour(0, cx), theme.colors().text_muted);
    });
    assert_eq!(compose::left(&texts(&view, cx).0), -3, "nothing is cut");
    keys(&view, "enter", cx);
    assert!(focused(&view, |c| &c.description, cx), "Enter in the summary moves to the description");
    cx.simulate_input("Because");
    assert_eq!(words(cx), "Commit changes to 3 files");

    let before = head(&f);
    keys(&view, "ctrl-enter", cx);
    assert_eq!(head(&f), before, "the hook refused the commit");
    let failure = panel(&view, cx, |p, _| p.composer().unwrap().failure.clone()).expect("the failure is shown");
    assert!(failure.contains("exit code 1") && failure.contains("commit-msg: no ticket"), "{failure}");
    assert!(painted(&view, "wip-commit-failure", cx));

    click(&view, "wip-skip-hooks", cx);
    click(&view, "wip-commit", cx);
    assert_eq!(head(&f), format!("{}\n\nBecause", "x".repeat(75)));
    assert_eq!(texts(&view, cx), (String::new(), String::new(), String::new()), "the fields empty after the commit");
    assert_eq!(toast(&view, cx), Some(format!("Committed \"{}\"", "x".repeat(75))));
    assert_eq!(words(cx), "Stage changes to commit");

    focus(&view, |c| &c.summary, cx);
    cx.simulate_input("Draft");
    click(&view, "wip-amend", cx);
    assert_eq!(texts(&view, cx).0, "x".repeat(75), "Amend fills in HEAD's message");
    assert_eq!(texts(&view, cx).2, "Because");
    assert_eq!(words(cx), "Amend previous commit");
    click(&view, "wip-amend", cx);
    assert_eq!((texts(&view, cx).0, texts(&view, cx).2), ("Draft".to_string(), String::new()), "unticking brings the draft back");

    click(&view, "wip-amend", cx);
    focus(&view, |c| &c.summary, cx);
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("Amended");
    keys(&view, "ctrl-enter", cx);
    assert_eq!(f.git(&["log", "-1", "--format=%s"]), "Amended");
    assert_eq!(f.git(&["rev-parse", "HEAD^"]), f.sha("ahead"), "the amend replaced the commit");
    let preview = journal::preview(&supraman_gitman::write::Writer::open(&f.repo()).unwrap()).unwrap();
    assert_eq!(preview.undo_tooltip(), "Undo commit amend \"Amended\"");
}

#[gpui::test]
fn wip_commit_area_says_merge_and_rebase_steps(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.conflict();
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    let words = |cx: &mut VisualTestContext| panel(&view, cx, |p, cx| p.commit_button(false, cx)).unwrap();
    assert_eq!(words(cx).0, "Commit and merge");
    assert!(painted(&view, "wip-abort", cx));
    assert!(!painted(&view, "wip-commit-options", cx), "no Commit options where the repo has no commit hooks");
    click(&view, "wip-abort", cx);
    assert_eq!(panel(&view, cx, |p, _| p.asking().map(str::to_string)).as_deref(), Some("Abort the merge? The resolutions so far are lost."));
    click(&view, "wip-confirm-yes", cx);
    assert!(!painted(&view, "wip-abort", cx), "the merge is gone");
}

#[gpui::test]
fn wip_keys_one_summary_in_two_fields_commit_stage_and_unstage(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    focus(&view, |c| &c.row, cx);
    cx.simulate_input("From the row");
    assert_eq!(texts(&view, cx).0, "From the row", "the WIP row edits the summary");
    focus(&view, |c| &c.summary, cx);
    cx.simulate_input("!");
    assert_eq!(texts(&view, cx).1, "From the row!", "the summary edits the WIP row");

    focus(&view, |c| &c.row, cx);
    let before = head(&f);
    keys(&view, "enter", cx);
    assert_eq!(head(&f), before, "Enter in the WIP row commits nothing");
    keys(&view, "ctrl-enter", cx);
    assert_eq!(head(&f), "From the row!", "Ctrl+Enter there commits");
    assert!(lists(&view, cx).1.is_empty());

    let table = view.read_with(cx, |v, _| v.table.focus_handle().clone());
    cx.update(|window, cx| window.focus(&table, cx));
    keys(&view, "ctrl-shift-s", cx);
    assert!(lists(&view, cx).0.is_empty(), "Ctrl+Shift+S stages everything");
    assert!(focused(&view, |c| &c.summary, cx), "and gives the summary the keyboard");
    keys(&view, "ctrl-shift-u", cx);
    assert!(lists(&view, cx).1.is_empty(), "Ctrl+Shift+U unstages everything");

    cx.update(|window, cx| window.focus(&table, cx));
    keys(&view, "ctrl-shift-m", cx);
    assert!(focused(&view, |c| &c.summary, cx), "Ctrl+Shift+M focuses the message");
    cx.simulate_input("Staged by keys");
    keys(&view, "ctrl-shift-enter", cx);
    assert_eq!(head(&f), "Staged by keys", "Ctrl+Shift+Enter stages all and commits");
    assert_eq!(f.git(&["status", "--porcelain"]), "");

    f.write("k.txt", b"k\n");
    view.update(cx, |v, cx| v.refresh_wip(None, cx));
    cx.run_until_parked();
    select_wip(&view, cx);
    let list = panel(&view, cx, |p, _| {
        p.unstaged.paths.select(Some(0));
        p.unstaged.paths.focus_handle().clone()
    });
    cx.update(|window, cx| window.focus(&list, cx));
    keys(&view, "s", cx);
    assert_eq!(lists(&view, cx), (Vec::<String>::new(), vec!["k.txt".to_string()]), "S stages the selected file");
    let list = panel(&view, cx, |p, _| {
        p.staged.paths.select(Some(0));
        p.staged.paths.focus_handle().clone()
    });
    cx.update(|window, cx| window.focus(&list, cx));
    keys(&view, "u", cx);
    assert_eq!(lists(&view, cx), (vec!["k.txt".to_string()], Vec::<String>::new()), "U unstages it");
    click(&view, "wip-unstaged-files-row-k.txt", cx);
    assert!(view.read_with(cx, |v, cx| v.diff(cx).is_some()), "the file's diff has the keyboard");
    keys(&view, "s", cx);
    assert_eq!(lists(&view, cx).1, vec!["k.txt".to_string()], "S from the open diff stages its file");
    view.update(cx, |v, cx| v.back_to_graph(cx));
    cx.run_until_parked();
    let list = panel(&view, cx, |p, _| {
        p.staged.paths.select(Some(0));
        p.staged.paths.focus_handle().clone()
    });
    cx.update(|window, cx| window.focus(&list, cx));
    keys(&view, "u", cx);
    focus(&view, |c| &c.summary, cx);
    cx.simulate_input("su");
    assert_eq!(texts(&view, cx).0, "su", "S and U type in a text field");
    assert_eq!(lists(&view, cx).0, vec!["k.txt".to_string()]);
}

/// `side` with W1 to W3 changing c.txt, stopped in GitMan's reword of W1 with W2 dropped: the pick of W3 conflicts.
pub(crate) fn stopped_rebase(f: &Fixture) -> String {
    let commits = ["one", "two", "three"].map(|n| {
        std::fs::write(f.worktree.join("c.txt"), format!("{n}\n")).unwrap();
        f.at(&f.worktree, &["add", "c.txt"]);
        f.at(&f.worktree, &["commit", "-q", "--no-verify", "-m", &format!("W {n}")]);
        f.at(&f.worktree, &["rev-parse", "HEAD"])
    });
    let todo = format!("reword {}\npick {}\n", commits[0], commits[2]);
    let stopped = f.gitmans_rebase(&supraman_gitman::Repo::new(f.git.clone(), &f.worktree), &commits[0], &todo);
    assert_eq!(stopped, supraman_gitman::write::rebase::Rebased::Conflicted);
    commits[2].clone()
}

#[gpui::test]
fn a_stopped_reword_shows_skip_then_continue_once_staged_which_finishes_it_and_undo_puts_the_old_tip_back(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let old = stopped_rebase(&f);
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    let words = |cx: &mut VisualTestContext| panel(&view, cx, |p, cx| p.commit_button(false, cx)).unwrap().0;
    assert_eq!(words(cx), "Skip commit", "nothing staged");
    assert!(painted(&view, "wip-abort", cx));
    std::fs::write(f.worktree.join("c.txt"), b"three\n").unwrap();
    f.at(&f.worktree, &["add", "c.txt"]);
    view.update(cx, |v, cx| v.refresh_after_write(cx));
    cx.run_until_parked();
    select_wip(&view, cx);
    assert_eq!(words(cx), "Continue rebase");
    click(&view, "wip-commit", cx);

    let head = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    assert_eq!(f.at(&f.worktree, &["log", "--format=%s", "-2", "HEAD"]), "W three\nReworded");
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(crate::git_view::Selected::Commit(head.into())), "the new HEAD is selected");
    click(&view, UNDO_KEY, cx);
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), old, "Undo puts the old tip back");
}

#[gpui::test]
fn abort_rebase_restores_the_old_tip_and_pops_gitmans_autostash(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("u.txt"), b"untracked\n").unwrap();
    let old = stopped_rebase(&f);
    assert!(!f.worktree.join("u.txt").exists(), "in the autostash meanwhile");
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    click(&view, "wip-abort", cx);
    assert_eq!(panel(&view, cx, |p, _| p.asking().map(str::to_string)).as_deref(), Some("Abort the rebase? The branch goes back to where it started and the resolutions so far are lost."));
    click(&view, "wip-confirm-yes", cx);
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), old);
    assert_eq!(std::fs::read(f.worktree.join("u.txt")).unwrap(), b"untracked\n");
    assert_eq!(toast(&view, cx).as_deref(), Some(crate::git_view::wip::actions::ABORTED_POPPED));
}

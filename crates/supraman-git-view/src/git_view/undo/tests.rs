use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::journal::{self, Action};
use supraman_gitman::write::{Call, LockSet, Writer};

use crate::git_view::test_support::{self, Opts, click, keys, toast};
use crate::git_view::undo::{REDO_KEY, UNDO_KEY};
use crate::git_view::*;
use crate::occupancy;

/// A Git tab of `folder` with `live` as the fleet's poll keeps it.
fn open<'a>(folder: &std::path::Path, live: occupancy::Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    test_support::open(folder, Opts::default().live(live), cx)
}

fn commit_recorded(dir: &std::path::Path, f: &Fixture, message: &str) {
    Writer::open(&supraman_gitman::Repo::new(f.git.clone(), dir))
        .unwrap()
        .run(LockSet::commit(), |s| {
            s.output(&Call::new(&["commit", "-q", "--no-verify", "--allow-empty", "-m", message]))?;
            journal::record(s, Action::Commit, None)
        })
        .unwrap();
}

fn head(f: &Fixture, dir: &std::path::Path) -> String {
    f.at(dir, &["rev-parse", "HEAD"])
}

fn tooltips(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (bool, String, bool, String) {
    view.read_with(cx, |v, cx| {
        let p = v.undo_bar().read(cx).preview();
        (p.can_undo(), p.undo_tooltip(), p.can_redo(), p.redo_tooltip())
    })
}

#[gpui::test]
fn toolbar_and_keys_undo_and_redo_name_the_action_and_a_merge_disables_both(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let before = head(&f, &f.worktree);
    commit_recorded(&f.worktree, &f, "Toolbar work");
    let after = head(&f, &f.worktree);
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    assert_eq!(tooltips(&view, cx), (true, "Undo commit \"Toolbar work\"".into(), false, "No recent undo to redo".into()));

    click(&view, UNDO_KEY, cx);
    assert_eq!(toast(&view, cx).as_deref(), Some("Undid commit \"Toolbar work\""));
    assert_eq!(head(&f, &f.worktree), before);
    assert_eq!(tooltips(&view, cx).3, "Redo commit \"Toolbar work\"", "the refresh read the preview again");

    click(&view, "git-row-0", cx);
    keys(&view, "ctrl-y", cx);
    assert_eq!((toast(&view, cx).as_deref(), head(&f, &f.worktree)), (Some("Redid commit \"Toolbar work\""), after.clone()));
    keys(&view, "ctrl-z", cx);
    assert_eq!((toast(&view, cx).as_deref(), head(&f, &f.worktree)), (Some("Undid commit \"Toolbar work\""), before.clone()));
    keys(&view, "ctrl-shift-z", cx);
    assert_eq!(head(&f, &f.worktree), after);

    f.conflict();
    view.update(cx, |v, cx| v.refresh_graph(None, cx));
    cx.run_until_parked();
    assert_eq!(tooltips(&view, cx), (false, "Cannot undo during a merge".into(), false, "Cannot redo during a merge".into()));
    let merging = head(&f, &f.worktree);
    click(&view, UNDO_KEY, cx);
    click(&view, REDO_KEY, cx);
    keys(&view, "ctrl-z", cx);
    assert_eq!(head(&f, &f.worktree), merging, "disabled during the merge");
}

#[gpui::test]
fn ctrl_z_in_a_text_field_undoes_the_typing_not_the_repository(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    commit_recorded(&f.worktree, &f, "Kept");
    let kept = head(&f, &f.worktree);
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    click(&view, "git-row-0", cx);
    keys(&view, "ctrl-alt-f", cx);
    keys(&view, "m a i n", cx);
    assert_eq!(view.read_with(cx, |v, cx| v.ref_panel().read(cx).filter()), "main", "the field has the keyboard");
    keys(&view, "ctrl-z", cx);
    assert_eq!(head(&f, &f.worktree), kept);
    assert_eq!(toast(&view, cx), None);
}

#[gpui::test]
fn undo_where_a_live_session_works_runs_without_a_question(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let before = head(&f, &f.worktree);
    commit_recorded(&f.worktree, &f, "Watched");
    let live = occupancy::Live::default();
    live.set_sessions(vec![occupancy::LiveSession { name: "fm-alpha".into(), claude_session_id: String::new(), cwd: f.worktree.join("src"), activity: supraman_ui::marks::Activity::Working, attach: None }]);
    let (view, cx) = open(&f.worktree, live, cx);
    click(&view, UNDO_KEY, cx);
    assert_eq!(head(&f, &f.worktree), before);
    assert_eq!(toast(&view, cx).as_deref(), Some("Undid commit \"Watched\""));
}

#[gpui::test]
fn undo_of_a_hard_reset_asks_before_it_stashes(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let before = head(&f, &f.dir);
    Writer::open(&f.repo())
        .unwrap()
        .run(LockSet::undo(), |s| {
            let snapshot = s.snapshot(&["a.txt"], "Before reset")?;
            s.output(&Call::new(&["reset", "-q", "--hard", "HEAD~1"]))?;
            journal::record(s, Action::Reset { hard: true }, Some(&snapshot))
        })
        .unwrap();
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    click(&view, UNDO_KEY, cx);
    let bar = view.read_with(cx, |v, _| v.undo_bar().clone());
    assert!(bar.read_with(cx, |b, _| b.asking().is_some_and(|q| q.contains("go to a stash before the undo of reset to"))));
    assert_ne!(head(&f, &f.dir), before);
    click(&view, "git-undo-stash-yes", cx);
    assert_eq!(head(&f, &f.dir), before);
}

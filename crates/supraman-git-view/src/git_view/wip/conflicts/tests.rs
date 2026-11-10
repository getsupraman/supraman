//! The conflict panel through the real tab in a merge, a rebase, a cherry-pick, a revert and an external rebase.

use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, run_git};
use supraman_gitman::OperationState;

use crate::git_view::wip::conflicts::headline;
use crate::git_view::test_support::{click, painted};
use crate::git_view::wip::actions::tests::{open_as, panel, stopped_rebase, texts};
use crate::git_view::wip::{Section, Shown};
use crate::git_view::test_support::redraw;
use crate::git_view::{GitView, Selected};
use crate::occupancy;

fn shown(view: &Entity<GitView>, cx: &mut VisualTestContext) -> std::rc::Rc<Shown> {
    panel(view, cx, |p, _| p.shown().expect("the working directory is shown"))
}

fn paths(shown: &Shown, section: Section) -> Vec<String> {
    shown.files(section).iter().map(|f| f.path.clone()).collect()
}

fn header(view: &Entity<GitView>, cx: &mut VisualTestContext) -> String {
    let shown = shown(view, cx);
    assert!(painted(view, "wip-conflict-header", cx), "the conflict header is painted");
    headline(shown.state, shown.conflicted.len())
}

fn words(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    panel(view, cx, |p, cx| p.commit_button(false, cx).map(|(w, _)| w))
}

fn asked(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    panel(view, cx, |p, _| p.asking().map(str::to_string))
}

fn state(f: &Fixture) -> OperationState {
    supraman_gitman::status(&supraman_gitman::Repo::new(f.git.clone(), &f.worktree)).unwrap().state
}

fn wt(f: &Fixture, args: &[&str]) -> String {
    f.at(&f.worktree, args)
}

#[gpui::test]
fn a_merge_conflict_selects_the_wip_row_and_the_panel_resolves_unresolves_takes_and_commits_it(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.conflict();
    let (view, cx) = open_as(&f.worktree, occupancy::Live::default(), cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Wip), "the conflict selects the WIP row");
    assert!(painted(&view, "git-conflict-node", cx), "the conflict node in place of the WIP row");
    assert_eq!(header(&view, cx), "Merge conflicts detected");
    assert!(painted(&view, "wip-conflict-incoming", cx) && painted(&view, "wip-conflict-current", cx), "merging main into side");
    let s = shown(&view, cx);
    assert_eq!((s.under_way.as_ref().map(|u| (u.incoming.as_str(), u.current.as_str()))), Some(("main", "side")));
    assert_eq!(paths(&s, Section::Conflicted), ["a.txt"]);
    assert!(paths(&s, Section::Staged).contains(&"b.txt".to_string()), "the merged files are among the resolved ones");
    assert!(painted(&view, "wip-conflicted-header", cx) && painted(&view, "wip-resolve-all", cx), "Conflicted Files and Mark all resolved");
    assert!(texts(&view, cx).0.starts_with("Merge branch 'main' into side"), "the summary from MERGE_MSG");
    assert_eq!(words(&view, cx).as_deref(), Some("Commit and merge"));

    click(&view, "wip-resolve-a.txt", cx);
    assert_eq!(asked(&view, cx).as_deref(), Some("a.txt still has conflict markers. Mark it resolved anyway?"));
    assert_eq!(wt(&f, &["ls-files", "-u"]).lines().count(), 3, "nothing staged before the answer");
    click(&view, "wip-confirm-yes", cx);
    assert_eq!(wt(&f, &["ls-files", "-u"]), "");
    assert_eq!(header(&view, cx), "Merge in progress");
    assert!(!painted(&view, "git-conflict-node", cx), "the WIP row is back once nothing is conflicted");

    click(&view, "wip-unresolve-a.txt", cx);
    assert_eq!(paths(&shown(&view, cx), Section::Conflicted), ["a.txt"], "Mark Conflicted puts it back");
    click(&view, "wip-take-current-a.txt", cx);
    assert_eq!(std::fs::read_to_string(f.worktree.join("a.txt")).unwrap(), "side\n");
    assert!(shown(&view, cx).conflicted.is_empty());
    click(&view, "wip-commit", cx);
    assert_eq!(state(&f), OperationState::None);
    assert!(wt(&f, &["log", "-1", "--format=%s"]).starts_with("Merge branch 'main' into side"));
    assert_eq!(wt(&f, &["rev-list", "--count", "--no-walk", "--merges", "HEAD"]), "1");
}

#[gpui::test]
fn a_conflict_appearing_later_selects_the_wip_row(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open_as(&f.worktree, occupancy::Live::default(), cx);
    assert_ne!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Wip));
    f.conflict();
    view.update(cx, |v, cx| v.refresh_after_write(cx));
    cx.run_until_parked();
    redraw(&view, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Wip));
    assert!(painted(&view, "git-conflict-node", cx));
}

#[gpui::test]
fn a_stopped_rebase_shows_its_sides_and_take_incoming_then_continue_finishes_it(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    stopped_rebase(&f);
    let (view, cx) = open_as(&f.worktree, occupancy::Live::default(), cx);
    assert_eq!(header(&view, cx), "Rebase conflicts detected");
    let s = shown(&view, cx);
    assert_eq!(s.under_way.as_ref().map(|u| (u.incoming.as_str(), u.step)), Some(("side", Some((2, 2)))));
    assert_eq!(words(&view, cx).as_deref(), Some("Skip commit"));
    click(&view, "wip-take-incoming-c.txt", cx);
    assert_eq!(std::fs::read_to_string(f.worktree.join("c.txt")).unwrap(), "three\n");
    assert_eq!(words(&view, cx).as_deref(), Some("Continue rebase"));
    click(&view, "wip-commit", cx);
    assert_eq!(state(&f), OperationState::None);
    assert_eq!(wt(&f, &["log", "--format=%s", "-2", "HEAD"]), "W three\nReworded");
}

#[gpui::test]
fn a_cherry_pick_shows_only_its_steps_and_continue_skip_and_abort_run_git(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.cherry_pick_conflict();
    let before = wt(&f, &["rev-parse", "HEAD"]);
    let (view, cx) = open_as(&f.worktree, occupancy::Live::default(), cx);
    assert_eq!(header(&view, cx), "Cherry-pick conflicts detected");
    assert!(!painted(&view, "wip-counter", cx) && !painted(&view, "wip-amend", cx), "no message box: git's message is the pick's");
    assert_eq!(words(&view, cx).as_deref(), Some("Skip commit"));
    assert!(painted(&view, "wip-abort", cx));
    click(&view, "wip-abort", cx);
    assert_eq!(asked(&view, cx).as_deref(), Some("Abort the cherry-pick? The branch goes back to where it started and the resolutions so far are lost."));
    click(&view, "wip-confirm-cancel", cx);
    click(&view, "wip-take-incoming-a.txt", cx);
    assert_eq!(words(&view, cx).as_deref(), Some("Continue cherry-pick"));
    click(&view, "wip-commit", cx);
    assert_eq!(state(&f), OperationState::None);
    assert_eq!((wt(&f, &["log", "-1", "--format=%s"]), wt(&f, &["rev-parse", "HEAD^"])), ("Ahead of origin".to_string(), before));
}

#[gpui::test]
fn a_revert_skips_after_asking(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.revert_conflict();
    let before = wt(&f, &["rev-parse", "HEAD"]);
    let (view, cx) = open_as(&f.worktree, occupancy::Live::default(), cx);
    assert_eq!(header(&view, cx), "Revert conflicts detected");
    click(&view, "wip-commit", cx);
    assert_eq!(asked(&view, cx).as_deref(), Some("Skip this commit of the revert? Its changes are left out."));
    click(&view, "wip-confirm-yes", cx);
    assert_eq!((state(&f), wt(&f, &["rev-parse", "HEAD"])), (OperationState::None, before));
}

#[gpui::test]
fn a_revert_aborts_after_asking(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.revert_conflict();
    let before = wt(&f, &["rev-parse", "HEAD"]);
    let (view, cx) = open_as(&f.worktree, occupancy::Live::default(), cx);
    assert!(painted(&view, "wip-abort", cx), "Abort revert");
    click(&view, "wip-abort", cx);
    click(&view, "wip-confirm-yes", cx);
    assert_eq!((state(&f), wt(&f, &["rev-parse", "HEAD"])), (OperationState::None, before));
}

#[gpui::test]
fn an_external_rebase_says_so_and_offers_only_abort(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("a.txt"), b"side\n").unwrap();
    wt(&f, &["commit", "-q", "--no-verify", "-am", "Side's a.txt"]);
    let before = wt(&f, &["rev-parse", "HEAD"]);
    assert!(!run_git(&f.git, &f.worktree, &["rebase", "main"], &[], None).success(), "the rebase stops on a conflict");
    let (view, cx) = open_as(&f.worktree, occupancy::Live::default(), cx);
    assert_eq!(state(&f), OperationState::Rebase { external: true });
    assert_eq!(header(&view, cx), "External rebase in progress");
    assert!(!painted(&view, "wip-commit", cx) && !painted(&view, "wip-counter", cx), "no continue, skip or message");
    click(&view, "wip-abort", cx);
    click(&view, "wip-confirm-yes", cx);
    assert_eq!((state(&f), wt(&f, &["rev-parse", "HEAD"])), (OperationState::None, before));
}

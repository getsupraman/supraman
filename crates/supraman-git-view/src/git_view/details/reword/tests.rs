//! The message box's amend and reword through the real tab, on fixture copies and their bare temp remote (C196).

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, MAIN_STASH, git_in};
use supraman_ui::hit::HitTargets;

use crate::git_view::details::reword::{ASK_KEY, CANCEL_KEY, CHECK_OUT_A_BRANCH, CHECK_OUT_TO_REWORD, CLICK_TO_AMEND, CLICK_TO_EDIT_STASH, CLICK_TO_REWORD, EDIT_KEY, REBASES_KEY, TYPE_A_SUMMARY, Target, UPDATE_KEY, offer, pushed_question, rebases_words, reword_question};
use crate::git_view::tests::redraw;
use crate::git_view::undo::UNDO_KEY;
use crate::git_view::{GitSizes, GitView, Selected, bind_keys};
use crate::occupancy;
use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1400.)).h(px(900.)).child(self.0.clone())
    }
}

fn open<'a>(folder: &std::path::Path, live: occupancy::Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        bind_keys(cx);
    });
    let folder = folder.to_path_buf();
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open_with(folder, None, HitTargets::default(), &GitSizes::default(), std::sync::Arc::new(live), cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    crate::git_view::test_support::assert_loaded(&view, cx);
    (view, cx)
}

fn select(view: &Entity<GitView>, sha: &str, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.select_sha(sha, cx));
    cx.run_until_parked();
    redraw(view, cx);
}

/// Whether `key` paints in a fresh frame.
fn painted(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) -> bool {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    hits.borrow().get(key).is_some()
}

fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    assert!(painted(view, key, cx), "{key} is painted");
    let at = view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds.center())).unwrap();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
    redraw(view, cx);
}

fn keys(view: &Entity<GitView>, keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    redraw(view, cx);
}

fn panel(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Entity<crate::git_view::details::CommitPanel> {
    view.read_with(cx, |v, _| v.panel().clone())
}

fn open_field(view: &Entity<GitView>, cx: &mut VisualTestContext) -> bool {
    panel(view, cx).read_with(cx, |p, _| p.field().is_some())
}

/// The open field's summary and description, and which of them has the keyboard.
fn field(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (String, String, bool, bool) {
    let p = panel(view, cx);
    cx.update(|window, cx| {
        let f = p.read(cx).field().expect("the field is open");
        let (s, d) = (f.summary.read(cx), f.description.read(cx));
        (s.text().to_string(), d.text().to_string(), s.is_focused(window), d.is_focused(window))
    })
}

fn type_in(view: &Entity<GitView>, summary: &str, description: &str, cx: &mut VisualTestContext) {
    let p = panel(view, cx);
    p.update(cx, |p, cx| {
        let f = p.field().expect("the field is open");
        let (s, d) = (f.summary.clone(), f.description.clone());
        s.update(cx, |i, cx| i.set_text(summary, cx));
        d.update(cx, |i, cx| i.set_text(description, cx));
        cx.notify();
    });
    redraw(view, cx);
}

fn focus_summary(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let summary = panel(view, cx).read_with(cx, |p, _| p.field().expect("the field is open").summary.clone());
    cx.update(|window, cx| summary.update(cx, |i, cx| i.focus(window, cx)));
    redraw(view, cx);
}

fn head(f: &Fixture) -> String {
    f.git(&["rev-parse", "HEAD"])
}

fn toast(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.toast.text().map(str::to_string))
}

#[gpui::test]
fn heads_message_box_opens_to_a_summary_and_description_and_amends_only_the_message(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (old, staged) = (head(&f), f.git(&["diff", "--cached"]));
    let who = |f: &Fixture| f.git(&["log", "-1", "--format=%an%ae%ad%T"]);
    let before = who(&f);
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);

    for read_only in [f.sha("merge"), f.sha("second")] {
        select(&view, read_only, cx);
        assert!(!painted(&view, EDIT_KEY, cx), "{read_only} is not HEAD: its box stays read-only");
    }

    select(&view, &old, cx);
    let shown = panel(&view, cx).read_with(cx, |p, _| p.shown().expect("HEAD shown"));
    assert_eq!(offer(&shown).map(|o| o.0), Some(CLICK_TO_AMEND));
    click(&view, EDIT_KEY, cx);
    assert_eq!(field(&view, cx), ("Ahead of origin".into(), String::new(), true, false), "the message in the field, the summary focused");
    assert!(painted(&view, "commit-amend-counter", cx), "the summary's count");
    keys(&view, "tab", cx);
    assert_eq!((field(&view, cx).2, field(&view, cx).3), (false, true), "Tab: the description");
    keys(&view, "shift-tab", cx);
    assert_eq!((field(&view, cx).2, field(&view, cx).3), (true, false), "Shift+Tab: the summary");
    keys(&view, "escape", cx);
    assert!(!open_field(&view, cx), "Esc cancels the amend");

    click(&view, EDIT_KEY, cx);
    type_in(&view, "  ", "", cx);
    assert_eq!(panel(&view, cx).read_with(cx, |p, cx| p.update_why_not(cx)), Some(TYPE_A_SUMMARY));
    click(&view, UPDATE_KEY, cx);
    assert_eq!(head(&f), old, "a blank summary amends nothing");

    std::fs::write(f.dir.join(".git/hooks/commit-msg"), "#!/bin/sh\necho 'no ticket in the message' >&2\nexit 1\n").unwrap();
    type_in(&view, "Ahead, amended", "Why it is ahead", cx);
    focus_summary(&view, cx);
    keys(&view, "ctrl-enter", cx);
    assert!(panel(&view, cx).read_with(cx, |p, _| p.amend_failure().is_some_and(|w| w.contains("no ticket in the message"))), "the hook's words in a banner");
    assert!(painted(&view, "commit-amend-failure", cx) && open_field(&view, cx));
    assert_eq!(head(&f), old);

    std::fs::remove_file(f.dir.join(".git/hooks/commit-msg")).unwrap();
    keys(&view, "ctrl-enter", cx);
    let new = head(&f);
    assert_ne!(new, old);
    assert_eq!(f.git(&["log", "-1", "--format=%s%n%b"]), "Ahead, amended\nWhy it is ahead");
    assert_eq!(who(&f), before, "author, date and tree kept");
    assert_eq!(f.git(&["diff", "--cached"]), staged, "the staged changes stay staged and out of the commit");
    assert_eq!(toast(&view, cx).as_deref(), Some("Amended \u{201c}Ahead, amended\u{201d}"));
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(new.into())), "the new HEAD is selected");
    assert!(!open_field(&view, cx));
}

#[gpui::test]
fn a_detached_heads_box_says_to_check_out_a_branch_and_opens_nothing(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    git_in(&f.git, &f.worktree, &["checkout", "-q", "--detach"], &[], None);
    let detached = git_in(&f.git, &f.worktree, &["rev-parse", "HEAD"], &[], None);
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    select(&view, &detached, cx);
    let shown = panel(&view, cx).read_with(cx, |p, _| p.shown().expect("HEAD shown"));
    assert_eq!(offer(&shown), Some((CHECK_OUT_A_BRANCH, None)));
    assert!(!painted(&view, EDIT_KEY, cx), "nothing to click");
}

#[gpui::test]
fn amending_a_pushed_head_asks_first_cancel_writes_nothing_and_undo_puts_the_old_head_back(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.git(&["push", "-q", "origin", "main"]);
    let old = head(&f);
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    select(&view, &old, cx);
    click(&view, EDIT_KEY, cx);
    type_in(&view, "Ahead, reworded", "", cx);
    click(&view, UPDATE_KEY, cx);
    assert_eq!(panel(&view, cx).read_with(cx, |p, _| p.amend_asking().map(str::to_string)), Some(pushed_question("main")));
    click(&view, &format!("{ASK_KEY}-cancel"), cx);
    assert_eq!(head(&f), old, "cancel writes nothing");
    assert!(open_field(&view, cx), "the field stays open");

    click(&view, UPDATE_KEY, cx);
    click(&view, &format!("{ASK_KEY}-yes"), cx);
    assert_eq!(f.git(&["log", "-1", "--format=%s"]), "Ahead, reworded");
    assert_ne!(head(&f), old);

    click(&view, UNDO_KEY, cx);
    assert_eq!(head(&f), old, "Undo puts the old HEAD back");
}

#[gpui::test]
fn a_stashs_box_edits_its_message_through_the_stash_writes_and_a_session_working_there_asks_nothing(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let live = occupancy::Live::default();
    let (view, cx) = open(&f.dir, live.clone(), cx);
    select(&view, f.sha("stash_message_index"), cx);
    let shown = panel(&view, cx).read_with(cx, |p, _| p.shown().expect("the stash shown"));
    assert_eq!(offer(&shown).map(|o| o.0), Some(CLICK_TO_EDIT_STASH));
    click(&view, EDIT_KEY, cx);
    assert_eq!(field(&view, cx).0, MAIN_STASH);
    assert!(painted(&view, CANCEL_KEY, cx));
    type_in(&view, "Better stash words", "", cx);
    click(&view, UPDATE_KEY, cx);
    let labels: Vec<String> = supraman_gitman::stashes(&f.repo()).unwrap().iter().map(|s| s.label()).collect();
    assert_eq!(labels[0], "Better stash words", "the stash keeps its commit and goes on top");

    live.set_sessions(vec![occupancy::LiveSession { name: "fm-alpha".into(), cwd: f.dir.clone(), activity: supraman_ui::marks::Activity::Working, attach: None }]);
    let top = head(&f);
    select(&view, &top, cx);
    click(&view, EDIT_KEY, cx);
    type_in(&view, "Ahead, not waiting", "", cx);
    click(&view, UPDATE_KEY, cx);
    assert_ne!(head(&f), top, "the amend ran without a question");
    assert_eq!(git_in(&f.git, &f.dir, &["log", "-1", "--format=%s"], &[], None), "Ahead, not waiting");
}

/// `side` (at `second`, above `root`) with three commits on it, W1 to W3, each changing c.txt.
fn three_above(f: &Fixture) -> [String; 3] {
    ["one", "two", "three"].map(|n| {
        std::fs::write(f.worktree.join("c.txt"), format!("{n}\n")).unwrap();
        git_in(&f.git, &f.worktree, &["add", "c.txt"], &[], None);
        git_in(&f.git, &f.worktree, &["commit", "-q", "--no-verify", "-m", &format!("W {n}")], &[], None);
        git_in(&f.git, &f.worktree, &["rev-parse", "HEAD"], &[], None)
    })
}

fn side_head(f: &Fixture) -> String {
    git_in(&f.git, &f.worktree, &["rev-parse", "HEAD"], &[], None)
}

fn subjects(f: &Fixture) -> Vec<String> {
    git_in(&f.git, &f.worktree, &["log", "--format=%s", "HEAD"], &[], None).lines().map(str::to_string).collect()
}

#[gpui::test]
fn an_older_commits_box_offers_reword_naming_the_commits_it_rebases_and_rewords_it(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let [w1, _, w3] = three_above(&f);
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    select(&view, f.sha("root"), cx);
    assert!(!painted(&view, EDIT_KEY, cx), "the root cannot be reworded");
    select(&view, &w3, cx);
    let shown = panel(&view, cx).read_with(cx, |p, _| p.shown().expect("HEAD shown"));
    assert_eq!(offer(&shown).map(|o| o.0), Some(CLICK_TO_AMEND), "HEAD is amended");

    select(&view, &w1, cx);
    let shown = panel(&view, cx).read_with(cx, |p, _| p.shown().expect("W1 shown"));
    assert_eq!(offer(&shown), Some((CLICK_TO_REWORD, Some(Target::Older { sha: w1.clone(), branch: "side".into(), above: 2, pushed: false }))));
    click(&view, EDIT_KEY, cx);
    assert_eq!(field(&view, cx).0, "W one");
    assert!(painted(&view, REBASES_KEY, cx), "{}", rebases_words(2));
    assert_eq!(rebases_words(2), "Rewords this commit and rebases the 2 commits above it");
    type_in(&view, "W one, reworded", "Why", cx);
    click(&view, UPDATE_KEY, cx);

    assert_eq!(subjects(&f), ["W three", "W two", "W one, reworded", "Second", "Root"]);
    assert_eq!(toast(&view, cx).as_deref(), Some("Reworded \u{201c}W one, reworded\u{201d}"));
    let new = side_head(&f);
    assert_ne!(new, w3);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(new.into())), "the new HEAD is selected");
    assert!(!open_field(&view, cx));
}

#[gpui::test]
fn a_pushed_or_dirty_reword_asks_first_cancel_writes_nothing_and_undo_puts_the_old_tip_back(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let [_, w2, w3] = three_above(&f);
    git_in(&f.git, &f.worktree, &["push", "-q", "origin", "side"], &[], None);
    std::fs::write(f.worktree.join("u.txt"), b"untracked\n").unwrap();
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    select(&view, &w2, cx);
    click(&view, EDIT_KEY, cx);
    type_in(&view, "W two, reworded", "", cx);
    click(&view, UPDATE_KEY, cx);
    assert_eq!(panel(&view, cx).read_with(cx, |p, _| p.amend_asking().map(str::to_string)), reword_question("side", 1, true, true));
    assert_eq!(reword_question("side", 1, true, true).unwrap(), "side was already pushed. Rewording rewrites 2 commits and the next push must force. Your changes are stashed and put back afterwards. Reword anyway?");
    click(&view, &format!("{ASK_KEY}-cancel"), cx);
    assert_eq!(side_head(&f), w3, "cancel writes nothing");

    click(&view, UPDATE_KEY, cx);
    click(&view, &format!("{ASK_KEY}-yes"), cx);
    assert_eq!(subjects(&f)[..3], ["W three", "W two, reworded", "W one"]);
    assert_eq!(std::fs::read(f.worktree.join("u.txt")).unwrap(), b"untracked\n", "the work in progress is back");

    click(&view, UNDO_KEY, cx);
    click(&view, "git-undo-stash-yes", cx);
    assert_eq!(side_head(&f), w3, "Undo puts the old tip back, the work in progress stashed first");
}

#[gpui::test]
fn a_detached_heads_older_commit_says_to_check_out_a_branch(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let [w1, ..] = three_above(&f);
    git_in(&f.git, &f.worktree, &["checkout", "-q", "--detach"], &[], None);
    let (view, cx) = open(&f.worktree, occupancy::Live::default(), cx);
    select(&view, &w1, cx);
    let shown = panel(&view, cx).read_with(cx, |p, _| p.shown().expect("W1 shown"));
    assert_eq!(offer(&shown), Some((CHECK_OUT_TO_REWORD, None)));
    assert!(!painted(&view, EDIT_KEY, cx));
}

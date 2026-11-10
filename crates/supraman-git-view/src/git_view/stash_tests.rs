//! The stash's writes through the real tab: the toolbar's Stash and Pop, the stash row's menus and the WIP's and
//! a stash's file buttons, on fixture copies (C196).

use gpui::{Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, MAIN_STASH};
use supraman_gitman::stashes;
use supraman_ui::hit::HitTargets;

use super::stash_bar::{NOTHING_TO_POP, NOTHING_TO_STASH, POP_KEY, STASH_KEY};
use super::tests::redraw;
use super::wip::tests::select_wip;
use super::*;
use crate::occupancy;

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
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open_with(folder, None, HitTargets::default(), &GitSizes::default(), Arc::new(live), cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    crate::git_view::test_support::assert_loaded(&view, cx);
    select_wip(&view, cx);
    (view, cx)
}

fn press(view: &Entity<GitView>, key: &str, button: MouseButton, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let position = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    if button == MouseButton::Left {
        cx.simulate_click(position, Modifiers::none());
    } else {
        cx.simulate_event(MouseDownEvent { position, button, modifiers: Modifiers::none(), click_count: 1, first_mouse: false });
        cx.simulate_event(MouseUpEvent { position, button, modifiers: Modifiers::none(), click_count: 1 });
    }
    cx.run_until_parked();
    redraw(view, cx);
}

fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    press(view, key, MouseButton::Left, cx);
}

fn labels(f: &Fixture) -> Vec<String> {
    stashes(&f.repo()).unwrap().iter().map(|s| s.label()).collect()
}

fn toast(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.toast.text().map(str::to_string))
}

fn asking(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, cx| v.stash_actions().read(cx).asking().map(str::to_string))
}

fn why_not(view: &Entity<GitView>, cx: &mut VisualTestContext) -> [Option<String>; 2] {
    view.read_with(cx, |v, cx| v.stash_why_not(cx))
}

fn summary(view: &Entity<GitView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, cx| v.wip_panel().read(cx).pending_summary(cx))
}

fn set_summary(view: &Entity<GitView>, text: &str, cx: &mut VisualTestContext) {
    let input = view.read_with(cx, |v, cx| v.wip_panel().read(cx).composer().expect("the commit area is made").summary.clone());
    input.update(cx, |i, cx| i.set_text(text, cx));
    redraw(view, cx);
}

fn status(f: &Fixture) -> String {
    f.git(&["status", "--porcelain", "--untracked-files=all"])
}

#[gpui::test]
fn toolbar_stash_takes_every_change_with_the_summary_and_pop_brings_it_back_each_dimmed_with_why(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    assert_eq!(why_not(&view, cx), [None, None]);
    set_summary(&view, "Toolbar stash", cx);
    click(&view, STASH_KEY, cx);
    assert_eq!(status(&f), "", "every change went, the untracked file too");
    assert_eq!(labels(&f)[0], "Toolbar stash", "the summary is the stash's message");
    assert_eq!((summary(&view, cx), toast(&view, cx).as_deref()), (String::new(), Some("Stashed all changes")), "the summary is spent");
    assert_eq!(why_not(&view, cx)[0].as_deref(), Some(NOTHING_TO_STASH), "a clean tree dims Stash");
    click(&view, STASH_KEY, cx);
    assert_eq!(labels(&f).len(), 4, "a dimmed Stash does nothing");

    click(&view, POP_KEY, cx);
    assert_eq!(asking(&view, cx), None, "a clean tree is not asked about");
    assert_eq!(labels(&f).len(), 3, "Pop drops the newest stash");
    assert_eq!(f.git(&["diff", "--cached", "--name-only"]), "", "everything is back unstaged");
    assert!(status(&f).contains("?? u.txt"));
    assert_eq!(summary(&view, cx), "Toolbar stash", "the popped stash's message fills the empty summary");
    assert_eq!(toast(&view, cx).as_deref(), Some("Popped \u{201c}Toolbar stash\u{201d}"));

    let top = labels(&f).remove(0);
    click(&view, POP_KEY, cx);
    assert_eq!(asking(&view, cx), Some(format!("Pop \u{201c}{top}\u{201d} over your unstaged changes?")), "local changes are asked about");
    click(&view, "git-stash-ask-cancel", cx);
    assert_eq!(labels(&f).len(), 3, "cancel pops nothing");

    f.git(&["stash", "clear"]);
    view.update(cx, |v, cx| v.refresh_after_write(cx));
    cx.run_until_parked();
    redraw(&view, cx);
    assert_eq!(why_not(&view, cx)[1].as_deref(), Some(NOTHING_TO_POP), "no stash dims Pop");
}

#[gpui::test]
fn a_pop_that_conflicts_says_so_and_keeps_the_stash(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.git(&["reset", "-q", "--hard"]);
    f.git(&["clean", "-fdq"]);
    f.write("b.txt", b"b\nb2\nmine\n");
    f.git(&["commit", "-q", "--no-verify", "-am", "Mine"]);
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    let top = labels(&f).remove(0);
    click(&view, POP_KEY, cx);
    assert_eq!(toast(&view, cx), Some(format!("Applying \u{201c}{top}\u{201d} left conflicts to resolve; the stash is kept")));
    assert_eq!(labels(&f).len(), 3, "the conflicted stash is not dropped");
    assert_eq!(view.read_with(cx, |v, cx| v.stash_actions().read(cx).conflicted().map(|s| s.sha.clone())), Some(f.sha("stash_wip").to_string()));
    assert!(status(&f).contains("UU b.txt"));
}

#[gpui::test]
fn toolbar_stash_runs_while_a_session_works_here(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let live = occupancy::Live::default();
    live.set_sessions(vec![occupancy::LiveSession { name: "fm-alpha".into(), cwd: f.dir.clone(), activity: supraman_ui::marks::Activity::Working, attach: None }]);
    let (view, cx) = open(&f.dir, live, cx);
    click(&view, STASH_KEY, cx);
    assert_eq!(labels(&f).len(), 4, "stashed without a question");
}

#[gpui::test]
fn the_stash_rows_menus_delete_after_asking_edit_the_message_and_apply_refuses_over_staged_changes(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    press(&view, "refpanel-name-stashes/1", MouseButton::Right, cx);
    click(&view, "menu-stash-delete", cx);
    assert_eq!(asking(&view, cx), Some(format!("Delete the stash \u{201c}{MAIN_STASH}\u{201d}? It cannot be undone.")));
    assert_eq!(labels(&f).len(), 3, "nothing goes before the answer");
    click(&view, "git-stash-ask-yes", cx);
    assert!(!labels(&f).contains(&MAIN_STASH.to_string()) && labels(&f).len() == 2);

    press(&view, "refpanel-name-stashes/1", MouseButton::Right, cx);
    click(&view, "menu-stash-edit", cx);
    view.update(cx, |v, cx| v.stash_actions().update(cx, |s, cx| s.type_message("Better words", cx)));
    click(&view, "git-stash-message-save", cx);
    assert_eq!(labels(&f)[0], "Better words", "the stash keeps its commit and goes on top");

    let row = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.stash().is_some()).expect("a stash row"));
    press(&view, &format!("git-row-{row}"), MouseButton::Right, cx);
    click(&view, "menu-stash-apply", cx);
    assert!(asking(&view, cx).is_some_and(|q| q.starts_with("Apply ")), "the work in progress is asked about");
    click(&view, "git-stash-ask-yes", cx);
    assert_eq!(toast(&view, cx).as_deref(), Some(supraman_gitman::write::stash::STAGED), "staged changes refuse an apply");
    assert_eq!(labels(&f).len(), 2);
}

#[gpui::test]
fn a_wip_files_stash_button_asks_and_a_stashs_file_applies_to_the_work_tree(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, occupancy::Live::default(), cx);
    click(&view, "wip-stash-u.txt", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("Stash 1 file?"));
    click(&view, "git-stash-ask-yes", cx);
    assert!(!f.dir.join("u.txt").exists() && labels(&f).len() == 4, "the file went to a new stash");
    assert_eq!(f.git(&["show", "stash@{0}^3:u.txt"]), "untracked");

    let feature = f.sha("stash_feature_index").to_string();
    view.update(cx, |v, cx| v.select_sha(&feature, cx));
    cx.run_until_parked();
    click(&view, "commit-apply-new.txt", cx);
    assert_eq!(std::fs::read_to_string(f.dir.join("new.txt")).unwrap(), "rename me\nline 2 on feature\nline 3\nline 4\n");
    assert_eq!(toast(&view, cx).as_deref(), Some("Applied 1 file from the stash"));
}

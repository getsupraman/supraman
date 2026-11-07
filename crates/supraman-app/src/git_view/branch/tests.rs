//! The ref writes through the real tab: the ref panel's and the pills' double-click checkout and menus, the name
//! dialog and the row's field, the questions, and a pill dropped on another, on fixture copies (C196).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Render, TestAppContext, VisualTestContext, Window, div, point, prelude::*, px};
use supraman_gitman::fixture::{Fixture, git_in, run_git};
use supraman_ui::hit::HitTargets;

use super::OpenGit;
use super::view::DraggedPill;
use crate::git_view::tests::redraw;
use crate::git_view::{GitSizes, GitView, bind_keys};
use crate::view::occupancy;

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1400.)).h(px(900.)).child(self.0.clone())
    }
}

fn open<'a>(folder: &Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        bind_keys(cx);
    });
    let folder = folder.to_path_buf();
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open_with(folder, None, HitTargets::default(), &GitSizes::default(), Arc::new(occupancy::Live::default()), cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    redraw(&view, cx);
    (view, cx)
}

fn press(view: &Entity<GitView>, key: &str, button: MouseButton, click_count: usize, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let position = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    let modifiers = Modifiers::none();
    cx.simulate_event(MouseDownEvent { position, button, modifiers, click_count, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position, button, modifiers, click_count });
    cx.run_until_parked();
    redraw(view, cx);
}

fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    press(view, key, MouseButton::Left, 1, cx);
}

fn painted(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.hits.borrow_mut().clear());
    redraw(view, cx);
    view.read_with(cx, |v, _| v.hits.borrow().contains_key(key))
}

fn head(f: &Fixture, dir: &Path) -> String {
    let out = run_git(&f.git, dir, &["symbolic-ref", "-q", "--short", "HEAD"], &[], None);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn sha(f: &Fixture, name: &str) -> String {
    git_in(&f.git, &f.dir, &["rev-parse", name], &[], None)
}

fn exists(f: &Fixture, full: &str) -> bool {
    run_git(&f.git, &f.dir, &["rev-parse", "-q", "--verify", full], &[], None).code == Some(0)
}

fn asking(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, cx| v.branch.read(cx).asking().map(str::to_string))
}

fn menu(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| v.branch.read(cx).menu_labels())
}

fn toast(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.toast.text().map(str::to_string))
}

fn opened(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<String>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let heard = seen.clone();
    cx.update(|_, cx| cx.subscribe(view, move |_, e: &OpenGit, _| heard.borrow_mut().push(supraman_core::paths::path_key(&e.0))).detach());
    seen
}

fn row_of(view: &Entity<GitView>, sha: &str, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.rows.iter().position(|r| r.sha.as_ref() == sha)).unwrap_or_else(|| panic!("a row of {sha}"))
}

fn group_of(view: &Entity<GitView>, ix: usize, name: &str, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.rows[ix].row.refs.iter().position(|g| g.name == name)).unwrap_or_else(|| panic!("a pill {name}"))
}

/// The menu of the pill `name` on the row of `at`.
fn pill_menu(view: &Entity<GitView>, at: &str, name: &str, cx: &mut VisualTestContext) {
    let ix = row_of(view, at, cx);
    let group = group_of(view, ix, name, cx);
    view.update(cx, |v, cx| v.pill_menu(ix, group, point(px(300.), px(300.)), cx));
    cx.run_until_parked();
    redraw(view, cx);
}

#[gpui::test]
fn a_double_click_checks_a_branch_out_opens_its_worktree_and_asks_to_stash_over_local_changes(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.worktree, cx);
    let opened = opened(&view, cx);
    press(&view, "refpanel-name-local/feature", MouseButton::Left, 2, cx);
    assert_eq!(head(&f, &f.worktree), "feature");
    assert_eq!(toast(&view, cx).as_deref(), Some("Checked out feature"));
    press(&view, "refpanel-name-local/main", MouseButton::Left, 2, cx);
    assert_eq!(*opened.borrow(), [supraman_core::paths::path_key(&f.dir)], "main lives in the main checkout: its Git tab opens");
    assert_eq!(head(&f, &f.worktree), "feature");

    std::fs::write(f.worktree.join("b.txt"), b"b\nmine\n").unwrap();
    press(&view, "refpanel-name-local/stale", MouseButton::Left, 2, cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("Checking out stale would overwrite your local changes. Stash them, check out and put them back?"));
    assert_eq!(head(&f, &f.worktree), "feature", "nothing before the answer");
    click(&view, "git-ref-ask-yes", cx);
    assert_eq!(head(&f, &f.worktree), "stale");
    let said = toast(&view, cx).unwrap_or_default();
    assert!(said.starts_with("Checked out stale; your changes stay in the stash \u{201c}GitMan: before checking out stale\u{201d}"), "a pop that conflicts keeps the stash and says so: {said}");
}

#[gpui::test]
fn a_remote_that_went_apart_from_its_local_asks_for_a_reset_or_a_new_branch(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    git_in(&f.git, &f.dir, &["push", "-q", "origin", &format!("{}:refs/heads/apart", f.sha("rename"))], &[], None);
    git_in(&f.git, &f.dir, &["fetch", "-q", "origin"], &[], None);
    git_in(&f.git, &f.dir, &["branch", "-q", "apart", f.sha("root")], &[], None);
    let (view, cx) = open(&f.worktree, cx);
    press(&view, "refpanel-name-remote/origin/apart", MouseButton::Left, 2, cx);
    assert_eq!(asking(&view, cx).as_deref(), Some(format!("apart and origin/apart went apart. Reset apart to origin/apart, or make a new branch at {}?", &f.sha("rename")[..7]).as_str()));
    click(&view, "git-ref-ask-alt", cx);
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.naming().expect("the name field in origin/apart's row").type_in("apart-2", "", cx)));
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.submit_name(cx)));
    cx.run_until_parked();
    assert_eq!(head(&f, &f.worktree), "apart-2");
    assert_eq!(sha(&f, "apart-2"), f.sha("rename"));
    assert_eq!(sha(&f, "apart"), f.sha("root"), "the local is left alone");
}

#[gpui::test]
fn a_pills_double_click_checks_out_and_its_menu_fast_forwards_merges_and_makes_a_branch_in_its_row(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.worktree, cx);
    pill_menu(&view, f.sha("main_work"), "stale", cx);
    assert_eq!(menu(&view, cx)[..3], ["Fast-forward side to stale", "Merge stale into side", "Rebase side onto stale"]);
    click(&view, "menu-ref-merge-stale", cx);
    assert_eq!(git_in(&f.git, &f.worktree, &["rev-parse", "HEAD^2"], &[], None), f.sha("main_work"), "a merge commit on side");
    assert_eq!(toast(&view, cx).as_deref(), Some("Merged stale into side"));

    pill_menu(&view, f.sha("rename"), "feature", cx);
    click(&view, "menu-ref-create-branch", cx);
    let ix = row_of(&view, f.sha("rename"), cx);
    assert!(view.read_with(cx, |v, cx| v.branch.read(cx).inline_field(&v.rows[ix].sha).is_some()), "the name field stands in the row");
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.naming().unwrap().type_in("bad name", "", cx)));
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.submit_name(cx)));
    cx.run_until_parked();
    assert!(!exists(&f, "refs/heads/bad name") && view.read_with(cx, |v, cx| v.branch.read(cx).naming().is_some()), "a bad name keeps the field");
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.naming().unwrap().type_in("from-feature", "", cx)));
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.submit_name(cx)));
    cx.run_until_parked();
    assert_eq!((head(&f, &f.worktree), sha(&f, "from-feature")), ("from-feature".to_string(), f.sha("rename").to_string()), "made in the row and checked out");

    let ix = row_of(&view, f.sha("rename"), cx);
    let group = group_of(&view, ix, "feature", cx);
    view.update(cx, |v, cx| v.pill_double_clicked(ix, group, point(px(0.), px(0.)), cx));
    cx.run_until_parked();
    assert_eq!(head(&f, &f.worktree), "feature");
}

#[gpui::test]
fn a_branch_with_commits_nowhere_else_is_asked_about_twice_and_the_checked_out_ones_delete_is_dimmed(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let commit = git_in(&f.git, &f.dir, &["commit-tree", "-p", f.sha("root"), "-m", "Lonely", &format!("{}^{{tree}}", f.sha("root"))], &[], None);
    git_in(&f.git, &f.dir, &["branch", "-q", "lonely", &commit], &[], None);
    let (view, cx) = open(&f.worktree, cx);
    press(&view, "refpanel-name-local/side", MouseButton::Right, 1, cx);
    assert!(menu(&view, cx).contains(&"Delete side".to_string()));
    click(&view, "menu-ref-delete-side", cx);
    assert!(asking(&view, cx).is_none() && exists(&f, "refs/heads/side"), "the checked-out branch's Delete is dimmed: a no-op");
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.show_menu(String::new(), Vec::new(), point(px(0.), px(0.)), cx)));

    press(&view, "refpanel-name-local/lonely", MouseButton::Right, 1, cx);
    click(&view, "menu-ref-delete-lonely", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("Delete the branch lonely? Undo can bring it back."));
    click(&view, "git-ref-ask-yes", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("lonely has commits on no other branch. Delete it anyway?"));
    assert!(exists(&f, "refs/heads/lonely"));
    click(&view, "git-ref-ask-yes", cx);
    assert!(!exists(&f, "refs/heads/lonely"));

    press(&view, "refpanel-name-local/feature", MouseButton::Right, 1, cx);
    click(&view, "menu-ref-delete-feature", cx);
    click(&view, "git-ref-ask-yes", cx);
    assert!(!exists(&f, "refs/heads/feature"), "a branch merged elsewhere is asked about once");
}

#[gpui::test]
fn the_dialog_makes_an_annotated_tag_and_renames_a_branch(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.worktree, cx);
    press(&view, "refpanel-name-local/feature", MouseButton::Right, 1, cx);
    click(&view, "menu-ref-create-annotated", cx);
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.naming().unwrap().type_in("t1", "Tag words", cx)));
    redraw(&view, cx);
    click(&view, "git-name-create", cx);
    assert_eq!(git_in(&f.git, &f.dir, &["cat-file", "-t", "t1"], &[], None), "tag");
    assert_eq!(git_in(&f.git, &f.dir, &["rev-parse", "t1^{commit}"], &[], None), f.sha("rename"));

    press(&view, "refpanel-name-local/stale", MouseButton::Right, 1, cx);
    click(&view, "menu-ref-rename-stale", cx);
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.naming().unwrap().type_in("fresh", "", cx)));
    redraw(&view, cx);
    click(&view, "git-name-create", cx);
    assert!(exists(&f, "refs/heads/fresh") && !exists(&f, "refs/heads/stale"));
}

#[gpui::test]
fn the_ref_menus_of_a_tag_a_worktree_a_detached_head_and_a_section_header(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, cx);
    press(&view, "refpanel-name-tags/light", MouseButton::Right, 1, cx);
    let labels = menu(&view, cx);
    assert!(labels.contains(&"Checkout the commit at light".to_string()) && labels.contains(&"Delete light locally".to_string()) && labels.contains(&"Annotate light".to_string()), "{labels:?}");
    press(&view, "refpanel-name-worktrees/1", MouseButton::Right, 1, cx);
    assert_eq!(menu(&view, cx), ["Open side", "Remove worktree", "Remove worktree and delete side", "Lock worktree", "Copy path"]);
    click(&view, "menu-worktree-lock", cx);
    assert!(supraman_gitman::refs(&f.repo()).unwrap().worktrees[1].locked);
    press(&view, "refpanel-row-local", MouseButton::Right, 1, cx);
    let section = view.read_with(cx, |v, cx| v.ref_panel.read(cx).section_items(crate::git_view::ref_panel::Section::Local).iter().map(|i| (i.label.clone(), i.dimmed.is_some())).collect::<Vec<_>>());
    assert_eq!(section, [("Hide all".to_string(), false), ("Maximize this section".to_string(), false)]);
    click(&view, "menu-section-maximize", cx);
    assert!(view.read_with(cx, |v, cx| v.ref_panel.read(cx).collapsed().iter().any(|c| c == "remote")), "the other sections close");
}

#[gpui::test]
fn a_detached_heads_menu_makes_a_branch_or_a_tag_and_copies_its_sha(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    git_in(&f.git, &f.worktree, &["checkout", "-q", "--detach", "HEAD"], &[], None);
    let (view, cx) = open(&f.worktree, cx);
    press(&view, "refpanel-name-local/HEAD", MouseButton::Right, 1, cx);
    assert_eq!(menu(&view, cx), ["Create branch here", "Copy commit sha", "Create tag here", "Create annotated tag here"]);
}

#[gpui::test]
fn a_pill_dropped_on_another_fast_forwards_that_branch_without_a_checkout(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, cx);
    let status = git_in(&f.git, &f.dir, &["status", "--porcelain"], &[], None);
    let (main_ix, stale_ix) = (row_of(&view, f.sha("ahead"), cx), row_of(&view, f.sha("main_work"), cx));
    let (main_group, stale_group) = (group_of(&view, main_ix, "main", cx), group_of(&view, stale_ix, "stale", cx));
    let pill = view.read_with(cx, |v, _| DraggedPill { group: v.rows[main_ix].row.refs[main_group].clone(), sha: v.rows[main_ix].sha.clone() });
    cx.update(|window, cx| view.update(cx, |v, cx| v.pill_dropped(&pill, stale_ix, stale_group, window, cx)));
    cx.run_until_parked();
    redraw(&view, cx);
    assert_eq!(menu(&view, cx), ["Fast-forward stale to main", "Merge main into stale"]);
    click(&view, "menu-drop-ff-stale", cx);
    assert_eq!(sha(&f, "stale"), f.sha("ahead"));
    assert_eq!(head(&f, &f.dir), "main", "nothing checked out");
    assert_eq!(git_in(&f.git, &f.dir, &["status", "--porcelain"], &[], None), status);
    assert!(!painted(&view, "menu-drop-ff-stale", cx), "a pick closes the menu");
}

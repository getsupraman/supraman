//! Create Worktree from the WORKTREES header's + through the real tab, on a fixture copy.

use std::path::Path;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, px};
use supraman_core::paths::path_key;
use supraman_gitman::fixture::Fixture;

use super::{KEY, NEEDS_REF, REFS};
use crate::git_view::GitView;
use crate::git_view::test_support::{self, Opts, click, keys, redraw, settle, toast};

fn open<'a>(folder: &Path, home: &Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    test_support::open(folder, Opts::default().home(home).size(1400., 900.), cx)
}

/// The form's picked reference, new branch, folder and why its button is dimmed; `None` while it is shut.
fn form(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<(Option<String>, String, String, Option<&'static str>)> {
    view.read_with(cx, |v, cx| v.branch.read(cx).adding().map(|a| (a.picked().map(str::to_string), a.new_branch(cx), a.folder(cx), a.why_not(cx))))
}

fn picking(view: &Entity<GitView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, cx| v.branch.read(cx).adding().is_some_and(|a| a.picker().is_some()))
}

fn focused(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<&'static str> {
    cx.update(|window, cx| view.read(cx).branch.read(cx).adding().and_then(|a| a.focused(window, cx)))
}

fn type_in(view: &Entity<GitView>, branch: Option<&str>, folder: Option<&str>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.adding_mut().unwrap().type_in(branch, folder, cx)));
    redraw(view, cx);
}

/// `query` typed in the open picker, then Enter.
fn pick(view: &Entity<GitView>, query: &str, cx: &mut VisualTestContext) {
    assert!(picking(view, cx), "the picker is open");
    cx.simulate_input(query);
    cx.run_until_parked();
    redraw(view, cx);
    keys(view, "enter", cx);
}

fn worktrees(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(String, Option<String>)> {
    view.read_with(cx, |v, cx| v.branch.read(cx).refs().worktrees.iter().map(|w| (path_key(&w.path), w.branch.clone())).collect())
}

#[gpui::test]
fn the_worktrees_headers_plus_opens_create_worktree_whose_enter_adds_one_and_esc_closes(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.git(&["update-ref", "refs/remotes/origin/topic", f.sha("root")]);
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = open(&f.dir, home.path(), cx);
    let hits = test_support::repaint(&view, cx);
    let at = |k: &str| hits.borrow().get(k).map(|h| h.bounds).unwrap_or_else(|| panic!("{k} is painted"));
    cx.simulate_mouse_move(at("refpanel-row-worktrees").center(), None, Modifiers::none());
    redraw(&view, cx);
    let (plus, count, header, refresh) = (at("refpanel-worktrees-add"), at("refpanel-worktrees-count"), at("refpanel-row-worktrees"), at("refpanel-worktrees-refresh"));
    assert!((plus.right() - count.right()).abs() < px(1.) && (plus.center().y - count.center().y).abs() < px(1.), "the + sits in the count's place: {plus:?} {count:?}");
    assert!(header.contains(&plus.center()) && header.contains(&refresh.center()), "the + and Refresh on the WORKTREES header");
    assert!(plus.right() <= refresh.left(), "Refresh to the +'s right");

    click(&view, "refpanel-worktrees-add", cx);
    assert_eq!(form(&view, cx), Some((None, String::new(), String::new(), Some(NEEDS_REF))), "Create Worktree opens, its button dimmed until a reference is picked");
    assert!(picking(&view, cx), "the reference's picker opens with it");
    for key in [KEY.to_string(), format!("{KEY}-ref"), format!("{KEY}-branch"), format!("{KEY}-folder"), format!("{KEY}-browse"), format!("{KEY}-ok"), format!("{KEY}-close"), REFS.to_string()] {
        assert!(test_support::painted(&view, &key, cx), "{key}");
    }
    let listed = view.read_with(cx, |v, cx| v.branch.read(cx).adding().unwrap().picker().unwrap().read(cx).shown().iter().map(|(e, _)| supraman_ui::palette::Entry::key(e).to_string()).collect::<Vec<_>>());
    assert!(["ref:Local:feature", "ref:Remote:origin/topic", "ref:Tag:v1"].iter().all(|k| listed.iter().any(|l| l == k)), "branches, remote branches and tags: {listed:?}");
    assert!(!listed.iter().any(|l| l.starts_with("ref:Stash")), "no stash: {listed:?}");

    pick(&view, "feature", cx);
    let main = f.dir.to_string_lossy().replace('\\', "/");
    let beside = |name: &str| format!("{}.worktrees/{name}", main.trim_end_matches('/'));
    let folder = |text: &str| text.replace('\\', "/");
    let (picked, branch, at_folder, why) = form(&view, cx).unwrap();
    assert_eq!((picked.as_deref(), branch.as_str(), folder(&at_folder), why), (Some("feature"), "", beside("feature"), None), "a local branch: no new branch, the folder beside the repository");
    assert!(!picking(&view, cx), "a pick closes the picker");
    assert_eq!(focused(&view, cx), Some("branch"), "the new branch's field takes the keyboard");

    click(&view, &format!("{KEY}-ref"), cx);
    pick(&view, "origin/topic", cx);
    let (picked, branch, at_folder, _) = form(&view, cx).unwrap();
    assert_eq!((picked.as_deref(), branch.as_str(), folder(&at_folder)), (Some("origin/topic"), "topic", beside("topic")), "a remote branch with no local one suggests its name");

    type_in(&view, Some("two words"), None, cx);
    assert_eq!(form(&view, cx).unwrap().3, Some("A name cannot hold a space"), "a bad name dims the button with why");
    click(&view, &format!("{KEY}-ok"), cx);
    assert!(form(&view, cx).is_some(), "the dimmed button does nothing");
    type_in(&view, Some("topic"), None, cx);
    let target = form(&view, cx).unwrap().2;
    click(&view, &format!("{KEY}-branch"), cx);
    keys(&view, "enter", cx);
    settle(&view, cx);
    assert_eq!(toast(&view, cx), Some(format!("Created worktree {target}")));
    assert_eq!(form(&view, cx), None, "the form closes");
    assert_eq!(f.at(Path::new(&target), &["symbolic-ref", "--short", "HEAD"]), "topic");
    assert!(worktrees(&view, cx).contains(&(path_key(Path::new(&target)), Some("topic".into()))), "the WORKTREES section lists it: {:?}", worktrees(&view, cx));
    assert!(test_support::painted(&view, "refpanel-row-worktrees/2", cx), "a third row in WORKTREES");

    click(&view, "refpanel-worktrees-add", cx);
    pick(&view, "side", cx);
    let held = f.dir.parent().unwrap().join("held");
    type_in(&view, None, Some(&held.to_string_lossy()), cx);
    keys(&view, "enter", cx);
    settle(&view, cx);
    let said = toast(&view, cx).unwrap_or_default();
    assert!(said.starts_with("'side' is already"), "git's refusal in its words: {said}");
    assert!(form(&view, cx).is_some(), "the form stays open");

    click(&view, &format!("{KEY}-ref"), cx);
    keys(&view, "escape", cx);
    assert!(!picking(&view, cx) && form(&view, cx).is_some(), "Esc in the picker closes the picker only");
    keys(&view, "escape", cx);
    assert_eq!(form(&view, cx), None, "Esc closes the form");
    click(&view, "refpanel-worktrees-add", cx);
    click(&view, &format!("{KEY}-close"), cx);
    assert_eq!(form(&view, cx), None, "× closes");
}

fn focus_at(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<usize> {
    cx.update(|window, cx| view.read(cx).branch.read(cx).adding().and_then(|a| a.focus_at(window, cx)))
}

#[gpui::test]
fn tab_walks_create_worktrees_reference_fields_and_buttons_and_wraps(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = open(&f.dir, home.path(), cx);
    click(&view, "refpanel-worktrees-add", cx);
    assert!(picking(&view, cx));
    assert_eq!(focus_at(&view, cx), Some(1), "the picker, after the reference's field");
    keys(&view, "tab", cx);
    assert_eq!((focused(&view, cx), picking(&view, cx)), (Some("branch"), false), "Tab from the picker: the new branch, the picker shut");
    let mut walked = vec![focus_at(&view, cx)];
    for _ in 0..4 {
        keys(&view, "tab", cx);
        walked.push(focus_at(&view, cx));
    }
    assert_eq!(walked, [Some(1), Some(2), Some(3), Some(4), Some(0)], "branch, folder, Browse, Create Worktree, then the reference");
    keys(&view, "shift-tab", cx);
    assert_eq!(focus_at(&view, cx), Some(4), "Shift+Tab from the reference wraps to Create Worktree");
    keys(&view, "shift-tab", cx);
    keys(&view, "shift-tab", cx);
    keys(&view, "shift-tab", cx);
    assert_eq!(focused(&view, cx), Some("branch"));
    keys(&view, "shift-tab", cx);
    keys(&view, "space", cx);
    assert!(picking(&view, cx), "Space on the focused reference opens its picker");
    keys(&view, "escape", cx);
    assert!(!picking(&view, cx) && form(&view, cx).is_some());
    keys(&view, "shift-tab", cx);
    assert_eq!(focus_at(&view, cx), Some(0));
    keys(&view, "escape", cx);
    assert_eq!(form(&view, cx), None, "Esc on a focused button closes the form");
}

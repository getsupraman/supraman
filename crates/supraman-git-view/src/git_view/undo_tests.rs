use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::write::journal::{self, Action};
use supraman_gitman::write::{Call, LockSet, Writer};
use supraman_ui::hit::HitTargets;

use super::tests::redraw;
use super::undo::{REDO_KEY, UNDO_KEY};
use super::*;
use crate::occupancy;

/// A Git tab of `folder` whose write gate reads `live`, as the fleet's poll keeps it.
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
    redraw(&view, cx);
    (view, cx)
}

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1200.)).h(px(800.)).child(self.0.clone())
    }
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
    git_in(&f.git, dir, &["rev-parse", "HEAD"], &[], None)
}

fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    redraw(view, cx);
    let at = view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds.center())).unwrap_or_else(|| panic!("{key} is painted"));
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
    redraw(view, cx);
}

fn keys(view: &Entity<GitView>, keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    redraw(view, cx);
}

fn toast(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.toast.text().map(str::to_string))
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
fn undo_where_a_live_session_works_asks_first(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let before = head(&f, &f.worktree);
    commit_recorded(&f.worktree, &f, "Watched");
    let live = occupancy::Live::default();
    live.set(vec![("fm-alpha".into(), f.worktree.join("src"))]);
    let (view, cx) = open(&f.worktree, live, cx);
    click(&view, UNDO_KEY, cx);
    let gate = view.read_with(cx, |v, _| v.gate().clone());
    assert!(gate.read_with(cx, |g, _| g.asking().is_some_and(|q| q.starts_with("fm-alpha is working in"))));
    assert_ne!(head(&f, &f.worktree), before, "nothing runs before the answer");
    click(&view, "write-guard-yes", cx);
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

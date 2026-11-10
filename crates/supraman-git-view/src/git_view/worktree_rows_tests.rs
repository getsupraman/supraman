//! The other worktrees' WIP rows through the real tab on a fixture copy with linked worktrees in its temp dir (C196):
//! where they show, what they say, and Open Worktree.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_gitman::BackgroundSettings;
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_ui::hit::HitTargets;

use super::background::{self, refresh_wips};
use super::branch::OpenGit;
use super::tests::redraw;
use super::{GitSizes, GitView, Selected, bind_keys};
use crate::occupancy;

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1400.)).h(px(900.)).child(self.0.clone())
    }
}

const QUIET: BackgroundSettings = BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), worktree_wip: None };

/// The fixture with a second linked worktree, `extra`, detached at `root`.
fn fixture() -> (Fixture, PathBuf) {
    let f = Fixture::copy();
    let extra = f.temp.path().join("extra");
    f.git(&["worktree", "add", "-q", "--detach", &extra.to_string_lossy(), f.sha("root")]);
    (f, extra)
}

fn open<'a>(f: &Fixture, live: occupancy::Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        bind_keys(cx);
        background::start_for_test(QUIET, cx);
    });
    let folder = f.dir.clone();
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open_with(folder, None, HitTargets::default(), &GitSizes::default(), Arc::new(live), cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    crate::git_view::test_support::assert_loaded(&view, cx);
    redraw(&view, cx);
    (view, cx)
}

fn reread(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let id = view.entity_id();
    cx.update(|_, cx| refresh_wips(id, cx));
    cx.run_until_parked();
    redraw(view, cx);
}

fn head(dir: &Path, f: &Fixture) -> String {
    git_in(&f.git, dir, &["rev-parse", "HEAD"], &[], None)
}

/// Each worktree row's folder and the sha of the row below it.
fn worktree_rows(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(String, String)> {
    view.read_with(cx, |v, _| {
        v.rows.iter().enumerate().filter_map(|(ix, r)| Some((path_key(r.worktree()?), v.rows.get(ix + 1).map(|b| b.sha.to_string()).unwrap_or_default()))).collect()
    })
}

fn row_of(view: &Entity<GitView>, dir: &Path, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.rows.iter().position(|r| r.worktree().is_some_and(|p| path_key(p) == path_key(dir)))).expect("a row of the worktree")
}

fn painted(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) -> bool {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    hits.borrow().contains_key(key)
}

fn press(view: &Entity<GitView>, key: &str, click_count: usize, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let position = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    let (button, modifiers) = (MouseButton::Left, Modifiers::none());
    cx.simulate_event(MouseDownEvent { position, button, modifiers, click_count, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position, button, modifiers, click_count });
    cx.run_until_parked();
    redraw(view, cx);
}

fn opened(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<String>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let heard = seen.clone();
    cx.update(|_, cx| cx.subscribe(view, move |_, e: &OpenGit, _| heard.borrow_mut().push(path_key(&e.0))).detach());
    seen
}

#[gpui::test]
fn a_worktree_row_shows_above_its_head_only_while_changed_or_worked_in_and_not_when_the_setting_is_off(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    let live = occupancy::Live::default();
    let (view, cx) = open(&f, live.clone(), cx);
    assert_eq!(worktree_rows(&view, cx), [], "both worktrees clean, no session in them");

    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    reread(&view, cx);
    assert_eq!(worktree_rows(&view, cx), [(path_key(&f.worktree), head(&f.worktree, &f))], "a change: its row right above its HEAD");

    live.set(vec![("fm-alpha".into(), extra.join("src"))]);
    reread(&view, cx);
    let mut rows = worktree_rows(&view, cx);
    rows.sort();
    let mut expected = vec![(path_key(&f.worktree), head(&f.worktree, &f)), (path_key(&extra), f.sha("root").to_string())];
    expected.sort();
    assert_eq!(rows, expected, "a live session below a clean worktree shows it too");
    let commits = view.read_with(cx, |v, _| (v.commits, v.rows.iter().filter(|r| r.commit()).count()));
    assert_eq!(commits.0, commits.1, "the worktree rows count as no commits");

    view.update(cx, |v, cx| {
        let ix = v.rows.iter().position(|r| r.worktree().is_some()).unwrap();
        v.table.select(Some(ix));
        v.selection_moved(cx);
    });
    assert!(matches!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Worktree(_))));

    let reads = cx.update(|_, cx| background::wips(view.entity_id(), cx).map(|w| w.read(cx).reads)).unwrap();
    cx.update(|_, cx| background::apply(BackgroundSettings { worktree_wip: Some(false), ..QUIET }, cx));
    cx.run_until_parked();
    assert_eq!(worktree_rows(&view, cx), [], "off: no worktree rows");
    reread(&view, cx);
    assert_eq!(cx.update(|_, cx| background::wips(view.entity_id(), cx).map(|w| w.read(cx).reads)), Some(reads), "off: nothing read");

    cx.update(|_, cx| background::apply(QUIET, cx));
    cx.run_until_parked();
    assert_eq!(worktree_rows(&view, cx).len(), 2, "on again: read and shown");
}

#[gpui::test]
fn a_worktree_row_says_its_counts_and_branch_or_detached_head_or_shows_the_sessions_pill(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    let live = occupancy::Live::default();
    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    std::fs::write(extra.join("a.txt"), b"changed\n").unwrap();
    let (view, cx) = open(&f, live.clone(), cx);
    reread(&view, cx);

    let side = row_of(&view, &f.worktree, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(side, cx)).expect("read");
    assert_eq!((said.label.as_str(), said.counts.as_str(), said.session.as_deref()), ("on side", "+1", None));
    assert!(painted(&view, &format!("git-worktree-wip-{side}"), cx), "muted WIP");
    assert!(painted(&view, &format!("git-worktree-ref-{side}"), cx));

    live.set(vec![("fm-alpha".into(), extra.clone())]);
    redraw(&view, cx);
    let detached = row_of(&view, &extra, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(detached, cx)).expect("read");
    assert_eq!((said.label.as_str(), said.counts.as_str(), said.session.as_deref()), ("Detached HEAD", "✎1", Some("fm-alpha")));
    assert!(painted(&view, &format!("git-worktree-session-{detached}"), cx), "the session's pill");
    assert!(!painted(&view, &format!("git-worktree-wip-{detached}"), cx), "in place of WIP");
}

#[gpui::test]
fn double_click_and_open_worktree_ask_for_the_worktrees_tab_and_a_detached_ones_double_click_does_nothing(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    std::fs::write(extra.join("a.txt"), b"changed\n").unwrap();
    let (view, cx) = open(&f, occupancy::Live::default(), cx);
    reread(&view, cx);
    let seen = opened(&view, cx);

    let side = row_of(&view, &f.worktree, cx);
    press(&view, &format!("git-worktree-ref-{side}"), 2, cx);
    assert_eq!(*seen.borrow(), [path_key(&f.worktree)], "double-click opens it");

    let detached = row_of(&view, &extra, cx);
    press(&view, &format!("git-worktree-ref-{detached}"), 2, cx);
    assert_eq!(seen.borrow().len(), 1, "a detached worktree's double-click does nothing");
    assert_eq!(view.read_with(cx, |v, _| v.table.selected()), Some(detached), "the click selects it");
    press(&view, &format!("git-worktree-open-{detached}"), 1, cx);
    assert_eq!(seen.borrow().last(), Some(&path_key(&extra)), "Open Worktree opens even a detached one");
}

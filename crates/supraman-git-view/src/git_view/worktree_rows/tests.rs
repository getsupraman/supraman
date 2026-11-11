//! The other worktrees' WIP rows through the real tab: where they show, what they say, and Open Worktree.

use std::path::{Path, PathBuf};

use gpui::{Entity, MouseButton, TestAppContext, VisualTestContext};
use supraman_core::paths::path_key;
use supraman_gitman::BackgroundSettings;
use supraman_gitman::fixture::Fixture;

use crate::git_view::background::{self, refresh_wips};
use crate::git_view::test_support::{self, Opts, QUIET, opened, painted, press, redraw, repaint};
use crate::git_view::{GitView, Selected};
use crate::occupancy;

/// The fixture with a second linked worktree, `extra`, detached at `root`.
fn fixture() -> (Fixture, PathBuf) {
    let f = Fixture::copy();
    let extra = f.temp.path().join("extra");
    f.git(&["worktree", "add", "-q", "--detach", &extra.to_string_lossy(), f.sha("root")]);
    (f, extra)
}

fn open<'a>(f: &Fixture, live: occupancy::Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    test_support::open(&f.dir, Opts::default().following(live).background().size(1400., 900.), cx)
}

fn reread(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let id = view.entity_id();
    cx.update(|_, cx| refresh_wips(id, cx));
    cx.run_until_parked();
    redraw(view, cx);
}

fn head(dir: &Path, f: &Fixture) -> String {
    f.at(dir, &["rev-parse", "HEAD"])
}

/// Each worktree row's folder and the sha of the row below it.
fn worktree_rows(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(String, String)> {
    view.read_with(cx, |v, cx| {
        v.pages.read(cx).rows.iter().enumerate().filter_map(|(ix, r)| Some((path_key(r.worktree()?), v.pages.read(cx).rows.get(ix + 1).map(|b| b.sha.to_string()).unwrap_or_default()))).collect()
    })
}

fn names(view: &crate::git_view::worktree_rows::WorktreeView) -> Vec<&str> {
    view.sessions.iter().map(|s| s.name.as_str()).collect()
}

fn session(name: &str, id: &str, cwd: PathBuf) -> occupancy::LiveSession {
    occupancy::LiveSession { name: name.into(), claude_session_id: id.into(), cwd, activity: supraman_ui::marks::Activity::Working, attach: None }
}

fn row_of(view: &Entity<GitView>, dir: &Path, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.worktree().is_some_and(|p| path_key(p) == path_key(dir)))).expect("a row of the worktree")
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

    live.set_sessions(vec![occupancy::LiveSession { name: "fm-alpha".into(), claude_session_id: String::new(), cwd: extra.join("src"), activity: supraman_ui::marks::Activity::Working, attach: None }]);
    reread(&view, cx);
    let mut rows = worktree_rows(&view, cx);
    rows.sort();
    let mut expected = vec![(path_key(&f.worktree), head(&f.worktree, &f)), (path_key(&extra), f.sha("root").to_string())];
    expected.sort();
    assert_eq!(rows, expected, "a live session below a clean worktree shows it too");
    let commits = view.read_with(cx, |v, cx| (v.pages.read(cx).commits, v.pages.read(cx).rows.iter().filter(|r| r.commit()).count()));
    assert_eq!(commits.0, commits.1, "the worktree rows count as no commits");

    view.update(cx, |v, cx| {
        let ix = v.pages.read(cx).rows.iter().position(|r| r.worktree().is_some()).unwrap();
        v.table.select(Some(ix));
        v.selection_moved(cx);
    });
    assert!(matches!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Worktree(_))));

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
fn a_worktree_row_says_its_counts_and_branch_or_detached_head_or_shows_the_sessions_lead(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    let live = occupancy::Live::default();
    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    std::fs::write(extra.join("a.txt"), b"changed\n").unwrap();
    let (view, cx) = open(&f, live.clone(), cx);
    reread(&view, cx);

    let side = row_of(&view, &f.worktree, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(side, cx)).expect("read");
    assert_eq!((said.label.as_str(), said.counts.as_str(), names(&said)), ("on side", "+1", vec![]));
    assert!(painted(&view, &format!("git-worktree-wip-{side}"), cx), "muted // WIP");
    assert!(painted(&view, &format!("git-worktree-ref-{side}"), cx));

    live.set_sessions(vec![occupancy::LiveSession { name: "fm-alpha".into(), claude_session_id: String::new(), cwd: extra.clone(), activity: supraman_ui::marks::Activity::Working, attach: None }]);
    redraw(&view, cx);
    let detached = row_of(&view, &extra, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(detached, cx)).expect("read");
    assert_eq!((said.label.as_str(), said.counts.as_str(), names(&said)), ("Detached HEAD", "✎1", vec!["fm-alpha"]));
    assert!(painted(&view, &format!("git-worktree-session-{detached}-0"), cx) && painted(&view, &format!("git-worktree-state-{detached}-0"), cx), "the session's lead with its state");
    assert!(!painted(&view, &format!("git-worktree-wip-{detached}"), cx), "in place of // WIP");
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
    press(&view, &format!("git-worktree-ref-{side}"), MouseButton::Left, 2, cx);
    assert_eq!(*seen.borrow(), [path_key(&f.worktree)], "double-click opens it");

    let detached = row_of(&view, &extra, cx);
    press(&view, &format!("git-worktree-ref-{detached}"), MouseButton::Left, 2, cx);
    assert_eq!(seen.borrow().len(), 1, "a detached worktree's double-click does nothing");
    assert_eq!(view.read_with(cx, |v, _| v.table.selected()), Some(detached), "the click selects it");
    press(&view, &format!("git-worktree-open-{detached}"), MouseButton::Left, 1, cx);
    assert_eq!(seen.borrow().last(), Some(&path_key(&extra)), "Open Worktree opens even a detached one");
}

#[gpui::test]
fn every_session_in_a_worktree_has_its_lead_on_its_row_its_word_in_its_tooltip_and_its_own_menu(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    let live = occupancy::Live::default();
    let (view, cx) = open(&f, live.clone(), cx);
    live.set_sessions(vec![session("fm-beta", "bb", extra.join("src")), session("fm-alpha", "aa", extra.clone()), session("fm-here", "hh", f.dir.clone())]);
    reread(&view, cx);

    let ix = row_of(&view, &extra, cx);
    assert_eq!(names(&view.update(cx, |v, cx| v.worktree_view(ix, cx)).expect("read")), ["fm-alpha", "fm-beta"]);
    for n in 0..2 {
        assert!(painted(&view, &format!("git-worktree-session-{ix}-{n}"), cx), "lead {n}");
        assert!(!painted(&view, &format!("git-worktree-state-{ix}-{n}"), cx), "lead {n}: its dot carries the state, the names get the room");
    }
    let hits = repaint(&view, cx);
    let (a, b) = (hits.borrow()[&format!("git-worktree-session-{ix}-0")].clone(), hits.borrow()[&format!("git-worktree-session-{ix}-1")].clone());
    assert!(a.bounds.right() <= b.bounds.left() && (a.bounds.center().y - b.bounds.center().y).abs() < gpui::px(1.), "side by side on one row: {a:?} {b:?}");
    let commit = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.commit())).expect("a commit row");
    let (agent, text) = (hits.borrow()["git-agent-0"], hits.borrow()[&format!("git-text-{commit}")]);
    assert_eq!((agent.bounds.left(), a.bounds.left()), (text.bounds.left(), text.bounds.left()), "the agent's lead, the worktree's and a commit's message start at one x");

    supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow_mut().take());
    cx.simulate_mouse_move(b.bounds.center(), None, gpui::Modifiers::none());
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow().clone()).as_deref(), Some("fm-beta · working"), "the word in the lead's tooltip with its name");

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s = seen.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &crate::git_view::agent_rows::AgentMenu, _| s.borrow_mut().push(e.session.name.clone())).detach());
    press(&view, &format!("git-worktree-session-{ix}-1"), MouseButton::Right, 1, cx);
    press(&view, &format!("git-worktree-ref-{ix}"), MouseButton::Right, 1, cx);
    assert_eq!(*seen.borrow(), ["fm-beta", "fm-alpha"], "a lead's right-click is its session's menu, the rest of the row the first's");
}

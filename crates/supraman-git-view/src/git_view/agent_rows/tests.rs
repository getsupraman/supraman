//! The agent rows through the real tab on a fixture copy in its temp dir and a fake [`Live`] (C196).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px};
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::write::{Call, LockSet, Resource, Writer};
use supraman_gitman::BackgroundSettings;
use supraman_ui::hit::HitTargets;
use supraman_ui::marks::Activity;

use super::super::background;
use super::super::tests::redraw;
use super::super::{GitSizes, GitView, Selected, bind_keys};
use super::AttachAgent;
use crate::occupancy::{Live, LiveSession};

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1400.)).h(px(900.)).child(self.0.clone())
    }
}

const QUIET: BackgroundSettings = BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), worktree_wip: None };

fn open<'a>(folder: PathBuf, live: Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        bind_keys(cx);
        background::start_for_test(QUIET, cx);
    });
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open_following(folder, None, HitTargets::default(), &GitSizes::default(), live, cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    crate::git_view::test_support::assert_loaded(&view, cx);
    redraw(&view, cx);
    (view, cx)
}

fn session(name: &str, cwd: PathBuf, activity: Activity, attach: Option<&str>) -> LiveSession {
    LiveSession { name: name.into(), cwd, activity, attach: attach.map(str::to_string) }
}

fn set(live: &Live, sessions: Vec<LiveSession>, view: &Entity<GitView>, cx: &mut VisualTestContext) {
    live.set_sessions(sessions);
    cx.run_until_parked();
    redraw(view, cx);
}

fn agents(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(usize, String)> {
    view.read_with(cx, |v, _| v.agent_rows())
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

/// Each row's kind, top down, for the first `n` rows.
fn kinds(view: &Entity<GitView>, n: usize, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.rows.iter().take(n).map(|r| match (&r.agent, r.wip()) {
            (Some(name), _) => format!("agent {name}"),
            (None, true) => "wip".into(),
            (None, false) => r.sha.to_string(),
        })
        .collect()
    })
}

#[gpui::test]
fn a_session_working_here_shows_as_a_row_over_the_wip_row_with_its_state_and_goes_when_it_ends(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    let before = view.read_with(cx, |v, _| (v.rows.len(), v.rows[0].row.clone()));
    assert_eq!(agents(&view, cx), []);

    set(&live, vec![session("fm-beta", f.dir.join("src"), Activity::Waiting, None), session("fm-alpha", f.dir.clone(), Activity::Working, Some("a1"))], &view, cx);
    assert_eq!(kinds(&view, 3, cx), ["agent fm-alpha", "agent fm-beta", "wip"], "one row per session, by name, over the WIP row");
    assert!(painted(&view, "git-agent-0", cx) && painted(&view, "git-agent-1", cx) && painted(&view, "git-graph-0", cx));
    assert_eq!(view.read_with(cx, |v, _| v.agents.shown.iter().map(|s| s.activity.word()).collect::<Vec<_>>()), ["working", "waiting"]);
    assert_eq!(view.read_with(cx, |v, _| v.wip_row()), Some(2));
    assert_eq!(view.read_with(cx, |v, _| v.commits), view.read_with(cx, |v, _| v.rows.iter().filter(|r| r.commit()).count()), "agent rows count as no commits");

    view.update(cx, |v, cx| v.select_wip(cx));
    set(&live, vec![session("fm-alpha", f.dir.clone(), Activity::Idle, Some("a1"))], &view, cx);
    assert_eq!(kinds(&view, 2, cx), ["agent fm-alpha", "wip"], "fm-beta ended");
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Wip), "the selection stays on its row");
    assert_eq!(view.read_with(cx, |v, _| v.agents.shown[0].activity), Activity::Idle, "the state follows the session");

    view.update(cx, |v, cx| v.refresh_graph(None, cx));
    cx.run_until_parked();
    assert_eq!(kinds(&view, 2, cx), ["agent fm-alpha", "wip"], "a read of the graph stacks it again");

    let elsewhere = f.temp.path().join("elsewhere");
    set(&live, vec![session("fm-alpha", elsewhere, Activity::Working, Some("a1"))], &view, cx);
    assert_eq!(agents(&view, cx), [], "the session left the worktree");
    let after = view.read_with(cx, |v, _| (v.rows.len(), v.rows[0].row.clone()));
    assert_eq!(after, before, "the rows as laid out without it");
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Wip));
}

#[gpui::test]
fn on_a_clean_tree_the_agent_row_stands_over_heads_row(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let live = Live::default();
    live.set_sessions(vec![session("fm-alpha", f.worktree.clone(), Activity::Working, None)]);
    let (view, cx) = open(f.worktree.clone(), live.clone(), cx);
    let head = git_in(&f.git, &f.worktree, &["rev-parse", "HEAD"], &[], None);
    let at = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.sha.as_ref() == head)).expect("HEAD's row");
    assert_eq!(view.read_with(cx, |v, _| v.wip_row()), None);
    assert_eq!(agents(&view, cx), [(at - 1, "fm-alpha".to_string())], "already working when the tab opened");
}

#[gpui::test]
fn selecting_an_agent_row_shows_the_session_and_attach_asks_for_it(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    let alpha = session("fm-alpha", f.dir.join("src"), Activity::Working, Some("a1"));
    set(&live, vec![alpha.clone(), session("fm-beta", f.dir.clone(), Activity::Idle, None)], &view, cx);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let seen = heard.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &AttachAgent, _| seen.borrow_mut().push(e.0.clone())).detach());

    press(&view, "git-agent-0", 1, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Agent("fm-alpha".into())));
    assert!(painted(&view, "git-agent-section-session", cx), "the session's page");
    press(&view, "git-agent-attach", 1, cx);
    assert_eq!(*heard.borrow(), [alpha.clone()], "Attach asks for the session");

    press(&view, "git-agent-1", 1, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Agent("fm-beta".into())));
    press(&view, "git-agent-attach", 1, cx);
    assert_eq!(heard.borrow().len(), 1, "a session that cannot be attached: Attach is dimmed");

    press(&view, "git-agent-0", 2, cx);
    assert_eq!(heard.borrow().len(), 2, "a double-click attaches too");

    set(&live, vec![session("fm-gamma", f.dir.clone(), Activity::Working, None), alpha.clone()], &view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Agent("fm-alpha".into())), "the selection follows its session's row");
    set(&live, vec![session("fm-gamma", f.dir.clone(), Activity::Working, None)], &view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), None, "the selected session ended");
}

#[gpui::test]
fn a_write_while_a_session_works_here_runs_without_a_question(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    set(&live, vec![session("fm-alpha", f.dir.join("src"), Activity::Working, None)], &view, cx);
    let gate = view.read_with(cx, |v, _| v.gate().clone());
    let writer = Writer::open(&f.repo()).unwrap();
    let asked = gate.update(cx, |g, cx| g.request(writer, LockSet::new().write(Resource::Index), |s| s.output(&Call::new(&["add"]).paths(["new.txt"])).map(drop), cx));
    assert!(asked.is_some());
    cx.run_until_parked();
    let staged = git_in(&f.git, &f.dir, &["diff", "--cached", "--name-only"], &[], None);
    assert!(staged.lines().any(|l| l == "new.txt"), "staged with fm-alpha working there: {staged}");
    assert!(!painted(&view, "write-guard-band", cx), "no question");
}

#[gpui::test]
fn a_session_starting_in_another_worktree_shows_its_row_with_the_pills_state_dot_at_once(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    let row = |view: &Entity<GitView>, cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.rows.iter().position(|r| r.worktree().is_some()));
    assert_eq!(row(&view, cx), None, "the other worktree is clean and nobody works there");

    set(&live, vec![session("fm-side", f.worktree.join("src"), Activity::Waiting, None)], &view, cx);
    let ix = row(&view, cx).expect("the session's start reads the worktrees again and lays its row out");
    assert!(painted(&view, &format!("git-worktree-session-{ix}"), cx) && painted(&view, &format!("git-worktree-state-{ix}"), cx), "the pill with its state's dot");
    assert_eq!(agents(&view, cx), [], "not a session of this worktree");

    set(&live, Vec::new(), &view, cx);
    assert_eq!(row(&view, cx), None, "its end takes the row away");
}

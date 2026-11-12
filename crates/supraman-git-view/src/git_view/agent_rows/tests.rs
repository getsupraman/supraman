//! The agent rows through the real tab on a fixture copy in its temp dir and a fake [`Live`].

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{Entity, MouseButton, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::{Call, LockSet, Resource};
use supraman_ui::marks::Activity;

use crate::git_view::test_support::{self, Opts, painted, press, redraw, repaint};
use super::super::{GitView, Selected};
use super::{AgentMenu, OpenAgent, PageAction, SessionPage, SessionPages};
use supraman_ui::icon::Icon;
use supraman_ui::session::Opening;
use crate::occupancy::{Live, LiveSession};

fn open<'a>(folder: PathBuf, live: Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    test_support::open(&folder, Opts::default().following(live).background().size(1400., 900.), cx)
}

/// A session whose claude session id is its name, so a row's key reads as its name.
fn session(name: &str, cwd: PathBuf, activity: Activity, attach: Option<&str>) -> LiveSession {
    LiveSession { name: name.into(), claude_session_id: name.into(), cwd, activity, attach: attach.map(str::to_string) }
}

fn set(live: &Live, sessions: Vec<LiveSession>, view: &Entity<GitView>, cx: &mut VisualTestContext) {
    live.set_sessions(sessions);
    cx.run_until_parked();
    redraw(view, cx);
}

fn agents(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(usize, String)> {
    view.read_with(cx, |v, cx| v.agent_rows(cx))
}

/// Each row's kind, top down, for the first `n` rows.
fn kinds(view: &Entity<GitView>, n: usize, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        v.pages.read(cx).rows.iter().take(n).map(|r| match (&r.agent, r.wip()) {
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
    let before = view.read_with(cx, |v, cx| (v.pages.read(cx).rows.len(), v.pages.read(cx).rows[0].row.clone()));
    assert_eq!(agents(&view, cx), []);

    set(&live, vec![session("fm-beta", f.dir.join("src"), Activity::Waiting, None), session("fm-alpha", f.dir.clone(), Activity::Working, Some("a1"))], &view, cx);
    assert_eq!(kinds(&view, 3, cx), ["agent fm-alpha", "agent fm-beta", "wip"], "one row per session, by name, over the WIP row");
    assert!(painted(&view, "git-agent-0", cx) && painted(&view, "git-agent-1", cx) && painted(&view, "git-graph-0", cx));
    assert_eq!(view.read_with(cx, |v, _| v.agents.shown().map(|s| s.activity.word()).collect::<Vec<_>>()), ["working", "waiting"]);
    assert_eq!(view.read_with(cx, |v, cx| v.wip_row(cx)), Some(2));
    assert_eq!(view.read_with(cx, |v, cx| v.pages.read(cx).commits), view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().filter(|r| r.commit()).count()), "agent rows count as no commits");

    view.update(cx, |v, cx| v.select_wip(cx));
    set(&live, vec![session("fm-alpha", f.dir.clone(), Activity::Idle, Some("a1"))], &view, cx);
    assert_eq!(kinds(&view, 2, cx), ["agent fm-alpha", "wip"], "fm-beta ended");
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Wip), "the selection stays on its row");
    assert_eq!(view.read_with(cx, |v, _| v.agents.shown().next().unwrap().activity), Activity::Idle, "the state follows the session");

    view.update(cx, |v, cx| v.refresh_graph(None, cx));
    cx.run_until_parked();
    assert_eq!(kinds(&view, 2, cx), ["agent fm-alpha", "wip"], "a read of the graph stacks it again");

    let elsewhere = f.temp.path().join("elsewhere");
    set(&live, vec![session("fm-alpha", elsewhere, Activity::Working, Some("a1"))], &view, cx);
    assert_eq!(agents(&view, cx), [], "the session left the worktree");
    let after = view.read_with(cx, |v, cx| (v.pages.read(cx).rows.len(), v.pages.read(cx).rows[0].row.clone()));
    assert_eq!(after, before, "the rows as laid out without it");
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Wip));
}

#[gpui::test]
fn on_a_clean_tree_the_agent_row_stands_over_heads_row(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let live = Live::default();
    live.set_sessions(vec![session("fm-alpha", f.worktree.clone(), Activity::Working, None)]);
    let (view, cx) = open(f.worktree.clone(), live.clone(), cx);
    let head = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    let at = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.sha.as_ref() == head)).expect("HEAD's row");
    assert_eq!(view.read_with(cx, |v, cx| v.wip_row(cx)), None);
    assert_eq!(agents(&view, cx), [(at - 1, "fm-alpha".to_string())], "already working when the tab opened");
}

/// A fleet with a page of `paged` only, whose Open runs for a session it can attach.
fn pages(paged: &'static str) -> SessionPages {
    let opening = |s: &LiveSession| Opening { icon: Icon::Attach, hint: if s.attach.is_some() { "Attach in a new tab" } else { "shown now" }.into(), runs: s.attach.is_some() };
    SessionPages {
        page: Rc::new(move |s, _| {
            (s.claude_session_id == paged).then(|| SessionPage {
                title: s.name.clone(),
                pill: ("Working".into(), gpui::red()),
                sections: vec![],
                open: opening(s),
                actions: vec![PageAction { key: "stop-session".into(), label: "Stop".into(), dimmed: None }],
            })
        }),
        act: Rc::new(|_, _, _, _| {}),
        opening: Rc::new(move |s, _| opening(s)),
    }
}

#[gpui::test]
fn selecting_an_agent_row_shows_the_session_and_open_asks_the_fleet_for_it(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new
").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    view.update(cx, |v, cx| v.set_session_pages(pages("fm-gamma"), cx));
    let alpha = session("fm-alpha", f.dir.join("src"), Activity::Working, Some("a1"));
    set(&live, vec![alpha.clone(), session("fm-beta", f.dir.clone(), Activity::Idle, None)], &view, cx);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let seen = heard.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &OpenAgent, _| seen.borrow_mut().push(e.0.clone())).detach());

    press(&view, "git-agent-0", MouseButton::Left, 1, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Agent("fm-alpha".into())));
    assert!(painted(&view, "git-agent-section-session", cx), "the session's folder and state");
    press(&view, "git-agent-open", MouseButton::Left, 1, cx);
    assert_eq!(*heard.borrow(), [alpha.clone()], "Open asks the fleet for the session");

    press(&view, "git-agent-1", MouseButton::Left, 1, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Agent("fm-beta".into())));
    press(&view, "git-agent-open", MouseButton::Left, 1, cx);
    assert_eq!(heard.borrow().len(), 1, "a session Open cannot open: dimmed with the fleet's why");

    press(&view, "git-agent-0", MouseButton::Left, 2, cx);
    assert_eq!(heard.borrow().len(), 2, "a double-click opens too");

    set(&live, vec![session("fm-gamma", f.dir.clone(), Activity::Working, None), alpha.clone()], &view, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Agent("fm-alpha".into())), "the selection follows its session's row");
    set(&live, vec![session("fm-gamma", f.dir.clone(), Activity::Working, None)], &view, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), None, "the selected session ended");
}

#[gpui::test]
fn the_fleets_page_shows_open_first_then_its_actions(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new
").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    view.update(cx, |v, cx| v.set_session_pages(pages("fm-alpha"), cx));
    let alpha = session("fm-alpha", f.dir.clone(), Activity::Working, Some("a1"));
    set(&live, vec![alpha.clone()], &view, cx);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let seen = heard.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &OpenAgent, _| seen.borrow_mut().push(e.0.clone())).detach());

    press(&view, "git-agent-0", MouseButton::Left, 1, cx);
    let hits = repaint(&view, cx);
    let (open, stop) = (hits.borrow()["git-agent-open"].bounds, hits.borrow()["git-agent-stop-session"].bounds);
    assert!(open.right() <= stop.left(), "Open first: {open:?} before {stop:?}");
    press(&view, "git-agent-open", MouseButton::Left, 1, cx);
    assert_eq!(*heard.borrow(), [alpha], "the page's Open asks the fleet");
}

#[gpui::test]
fn two_sessions_of_one_name_are_two_rows_each_selected_and_attached_by_its_own_id(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    let twin = |id: &str, attach: &str| LiveSession { claude_session_id: id.into(), ..session("77aa88bb", f.dir.clone(), Activity::Working, Some(attach)) };
    let (first, second) = (twin("uuid-1", "a1"), twin("uuid-2", "a2"));
    set(&live, vec![first.clone(), second.clone()], &view, cx);
    assert_eq!(agents(&view, cx), [(0, "uuid-1".to_string()), (1, "uuid-2".to_string())], "a row per session, keyed by its claude session id");
    let heard = Rc::new(RefCell::new(Vec::new()));
    let seen = heard.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &OpenAgent, _| seen.borrow_mut().push(e.0.clone())).detach());

    press(&view, "git-agent-1", MouseButton::Left, 2, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Agent("uuid-2".into())));
    assert_eq!(*heard.borrow(), [second.clone()], "the second row attaches the second session");

    set(&live, vec![second.clone()], &view, cx);
    assert_eq!(agents(&view, cx), [(0, "uuid-2".to_string())], "the first ended: its row goes, the other stays");
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Agent("uuid-2".into())), "the selection stays on its session");
}

#[gpui::test]
fn right_clicking_an_agent_row_asks_the_fleet_for_its_sessions_menu(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    let alpha = session("fm-alpha", f.dir.clone(), Activity::Idle, None);
    set(&live, vec![alpha.clone()], &view, cx);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let seen = heard.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &AgentMenu, _| seen.borrow_mut().push(e.session.clone())).detach());
    press(&view, "git-agent-0", MouseButton::Right, 1, cx);
    assert_eq!(*heard.borrow(), [alpha], "the session's menu is the fleet's");
}

#[gpui::test]
fn in_a_narrow_column_the_leads_name_is_cut_with_an_ellipsis_and_nothing_overlaps(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let live = Live::default();
    let (view, cx) = test_support::open(&f.dir, Opts::default().following(live.clone()).background().size(900., 600.), cx);
    let long = "fm-a-session-whose-name-is-far-too-long-for-the-message-column";
    set(&live, vec![session(long, f.dir.clone(), Activity::Working, None)], &view, cx);
    let hits = repaint(&view, cx);
    let name = supraman_ui::clipped::cut_at("git-agent-0-name").expect("the name painted cut, with an ellipsis and its tooltip");
    let hits = hits.borrow();
    let (lead, state) = (&hits["git-agent-0"], &hits["git-agent-state-0"]);
    assert!(lead.fully_visible(), "the lead inside its cell: {lead:?}");
    assert!(name.left() >= lead.bounds.left() && name.right() <= state.bounds.left(), "the name {name:?} ends before its state {state:?}");
    assert!(state.bounds.right() <= lead.bounds.right(), "its state {state:?} inside the lead {lead:?}, given up before the name");
    assert!(name.size.width >= state.bounds.size.width, "the name {name:?} keeps more room than its state {state:?}");
}

#[gpui::test]
fn a_write_while_a_session_works_here_runs_without_a_question(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    set(&live, vec![session("fm-alpha", f.dir.join("src"), Activity::Working, None)], &view, cx);
    let gate = view.read_with(cx, |v, _| v.gate().clone());
    let asked = gate.update(cx, |g, cx| g.request(LockSet::new().write(Resource::Index), "Staging".into(), |s| s.output(&Call::new(&["add"]).paths(["new.txt"])).map(drop), cx));
    assert!(asked.is_some());
    cx.run_until_parked();
    let staged = f.at(&f.dir, &["diff", "--cached", "--name-only"]);
    assert!(staged.lines().any(|l| l == "new.txt"), "staged with fm-alpha working there: {staged}");
    assert!(!painted(&view, "write-guard-band", cx), "no question");
}

#[gpui::test]
fn a_session_starting_in_another_worktree_shows_its_row_with_the_sessions_agent_row_on_it_at_once(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let live = Live::default();
    let (view, cx) = open(f.dir.clone(), live.clone(), cx);
    let row = |view: &Entity<GitView>, cx: &mut VisualTestContext| view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.worktree().is_some()));
    assert_eq!(row(&view, cx), None, "the other worktree is clean and nobody works there");

    set(&live, vec![session("fm-side", f.worktree.join("src"), Activity::Waiting, None)], &view, cx);
    let ix = row(&view, cx).expect("the session's start reads the worktrees again and lays its row out");
    assert_eq!(agents(&view, cx), [(ix - 1, "fm-side".to_string())], "its agent row stacked on the worktree's row");
    assert!(painted(&view, &format!("git-agent-state-{}", ix - 1), cx), "the session's lead with its state");
    assert!(!painted(&view, &format!("git-worktree-session-{ix}-0"), cx), "no lead on the worktree row");

    set(&live, Vec::new(), &view, cx);
    assert_eq!(row(&view, cx), None, "its end takes the row away");
}

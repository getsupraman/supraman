use std::time::Duration;

use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use supraman_core::testing::{eventually, polled};
use supraman_gitman::fixture::Fixture;
use supraman_term::terminal_settings::{AlternateScroll, CursorShape};
use supraman_term::{Terminal, TerminalBuilder};
use util::paths::PathStyle;

use super::Docks;
use supraman_ui::dock::layout::Layout;
use crate::git_view::GitView;
use crate::view::{FleetView, TabKind};

pub(in crate::view) const WITHIN: Duration = Duration::from_secs(30);
/// F12's escape sequence: a key the terminal writes itself and the Git view takes no action for.
const F12: &[u8] = b"\x1b[24~";

pub(in crate::view) fn window(cx: &mut TestAppContext) -> (Entity<FleetView>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        supraman_editor::init(supraman_ui::fonts::MONO, 13., cx);
        supraman_ui::init_globals(cx, "test");
        crate::view::bind_keys(cx);
        supraman_ui::text::input::bind_keys(cx);
        crate::git_view::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|window, cx| FleetView::inert(window, cx));
    cx.update(|window, _| window.activate_window());
    (view, cx)
}

pub(in crate::view) fn draw(view: &Entity<FleetView>, cx: &mut VisualTestContext) {
    for _ in 0..2 {
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.update(|_, _| ());
    }
}

/// A shell tab whose display-only terminal logs what it writes to its PTY.
pub(in crate::view) fn shell(view: &Entity<FleetView>, cx: &mut VisualTestContext) -> (u64, Entity<Terminal>) {
    view.update(cx, |v, cx| {
        let id = v.push_tab(format!("shell {}", v.next_tab_id), TabKind::Shell, None, cx);
        let terminal = cx.new(|cx| TerminalBuilder::new_display_only(CursorShape::Bar, AlternateScroll::On, None, cx.background_executor(), PathStyle::local()).subscribe(cx));
        v.tab_mut(id).expect("the shell tab").terminal = Some(terminal.clone());
        (id, terminal)
    })
}

pub(in crate::view) fn git_panel(view: &Entity<FleetView>, f: &Fixture, cx: &mut VisualTestContext) -> (u64, Entity<GitView>) {
    view.update(cx, |v, cx| {
        let id = v.open_git(f.dir.clone(), "Git · repo".into(), cx);
        let Some(TabKind::Git(git)) = v.tab(id).map(|t| &t.kind) else { panic!("a Git panel") };
        (id, git.clone())
    })
}

pub(in crate::view) fn keys(view: &Entity<FleetView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| v.hits.borrow().keys().cloned().collect())
}

fn click(view: &Entity<FleetView>, key: &str, cx: &mut VisualTestContext) {
    let bounds = view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds)).unwrap_or_else(|| panic!("{key} paints"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    draw(view, cx);
}

fn written(terminal: &Entity<Terminal>, cx: &mut VisualTestContext) -> Vec<u8> {
    terminal.update(cx, |t, _| t.take_pty_write_log()).concat()
}

pub(in crate::view) fn loaded(view: &Entity<FleetView>, git: &Entity<GitView>, cx: &mut VisualTestContext) {
    eventually(WITHIN, "the Git panel's first load", polled(Duration::from_millis(20)), || {
        draw(view, cx);
        git.read_with(cx, |g, cx| !g.probe(cx).rows.is_empty()).then_some(())
    });
}

#[gpui::test]
fn dock_of_a_terminal_and_a_git_panel_paints_both_keys_reach_the_clicked_one_and_git_follows_its_repo(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = window(cx);
    let (tab, terminal) = shell(&view, cx);
    let (panel, git) = git_panel(&view, &f, cx);
    assert!(view.update(cx, |v, cx| v.add_panel(tab, panel, cx)), "a Git panel joins the shell's tab");
    assert_eq!(view.read_with(cx, |v, _| (v.active, v.header_ids())), (Some(tab), vec![tab]), "one tab in the strip, the dock shown");
    loaded(&view, &git, cx);

    let painted = keys(&view, cx);
    let term = format!("p{tab}/terminal-{tab}");
    for key in [term.clone(), format!("dock-{tab}-slot-0"), format!("dock-{tab}-slot-1"), format!("dock-{tab}-title-0"), format!("dock-{tab}-title-1")] {
        assert!(painted.contains(&key), "{key} paints: {painted:?}");
    }

    click(&view, &format!("dock-{tab}-slot-1"), cx);
    written(&terminal, cx);
    cx.simulate_keystrokes("f12");
    assert_eq!(written(&terminal, cx), b"", "a key typed in the clicked Git panel never reaches the terminal");
    assert_eq!(view.read_with(cx, |v, _| v.docks.focused(tab)), panel, "the Git panel is the dock's focused one");

    click(&view, &term, cx);
    cx.simulate_keystrokes("f12");
    cx.simulate_input("x");
    assert_eq!(written(&terminal, cx), [F12, b"x"].concat(), "the clicked terminal takes keys and typed text");
    assert_eq!(view.read_with(cx, |v, _| v.docks.focused(tab)), tab);

    f.git(&["commit", "-q", "--no-verify", "--allow-empty", "-m", "Made beside the terminal"]);
    let head = f.git(&["rev-parse", "HEAD"]);
    eventually(WITHIN, "the outside commit in the Git panel", polled(Duration::from_millis(20)), || {
        draw(&view, cx);
        git.read_with(cx, |g, cx| g.probe(cx).rows.iter().any(|r| r.sha == head)).then_some(())
    });
    let holds = cx.update(|window, cx| view.read(cx).focused_panel(window, cx));
    assert_eq!(holds, Some(tab), "the terminal kept the keys while the Git panel refreshed");
}

#[gpui::test]
fn dock_rule_opening_git_for_a_repo_docked_in_another_tab_focuses_that_tab_and_panel(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = window(cx);
    let (tab, _) = shell(&view, cx);
    let (panel, _) = git_panel(&view, &f, cx);
    view.update(cx, |v, cx| v.add_panel(tab, panel, cx));
    let (other, _) = shell(&view, cx);
    view.update(cx, |v, _| v.docks.focus(tab));
    assert_eq!(view.read_with(cx, |v, _| (v.active, v.docks.focused(tab))), (Some(other), tab));

    let (again, _) = git_panel(&view, &f, cx);
    let gits = view.read_with(cx, |v, _| v.tabs.iter().filter(|t| matches!(t.kind, TabKind::Git(_))).count());
    assert_eq!((again, gits), (panel, 1), "the docked Git panel, not a second view");
    assert_eq!(view.read_with(cx, |v, _| (v.active, v.docks.focused(tab), v.header_ids())), (Some(tab), panel, vec![tab, other]), "its tab shown, the panel focused");
}

#[gpui::test]
fn dock_closing_one_panel_of_two_leaves_a_tab_of_one_that_paints_as_today(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = window(cx);
    let (tab, _) = shell(&view, cx);
    let (panel, git) = git_panel(&view, &f, cx);
    view.update(cx, |v, cx| v.add_panel(tab, panel, cx));
    loaded(&view, &git, cx);
    assert!(keys(&view, cx).contains(&format!("dock-{tab}-close-1")));

    click(&view, &format!("dock-{tab}-close-1"), cx);
    assert_eq!(view.read_with(cx, |v, _| (v.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), v.active, v.docks.of(tab).is_none())), (vec![tab], Some(tab), true), "the Git panel closed, its dock gone");
    let painted = keys(&view, cx);
    assert!(painted.contains(&format!("terminal-{tab}")), "the terminal paints under the window's own keys: {painted:?}");
    assert!(!painted.iter().any(|k| k.starts_with(&format!("p{tab}/")) || k.starts_with("dock-")), "no panel scope and no dock: {painted:?}");
    let order: Vec<String> = cx.update(|window, cx| supraman_ui::focus::order(window, cx).into_iter().map(|k| k.to_string()).collect());
    assert!(order.contains(&"terminal".to_string()), "the content pane as today: {order:?}");

    let (panel, _) = git_panel(&view, &f, cx);
    view.update(cx, |v, cx| v.add_panel(tab, panel, cx));
    view.update(cx, |v, cx| v.close_panel(tab, cx));
    assert_eq!(view.read_with(cx, |v, _| (v.header_ids(), v.active, v.docks.of(panel).is_none())), (vec![panel], Some(panel), true), "the tab's own panel closed: the tab passes to the Git panel");

    let (tab, _) = shell(&view, cx);
    view.update(cx, |v, cx| v.add_panel(panel, tab, cx));
    assert!(view.update(cx, |v, cx| v.remove_panel(tab, cx)));
    assert_eq!(view.read_with(cx, |v, _| v.header_ids()), vec![panel, tab], "a panel taken out is a tab after the one it left");
    view.update(cx, |v, cx| v.add_panel(panel, tab, cx));
    view.update(cx, |v, cx| v.close_tab(panel, cx));
    assert!(view.read_with(cx, |v, _| v.tabs.is_empty()), "closing the tab closes every panel");
}

#[gpui::test]
fn dock_model_adds_three_at_most_and_hands_the_tab_on_when_its_own_panel_leaves(cx: &mut TestAppContext) {
    let mut handle = || cx.update(|cx| cx.focus_handle());
    let mut docks = Docks::default();
    assert!(docks.add(1, 2, &mut handle));
    assert!(docks.add(1, 3, &mut handle));
    assert!(!docks.add(1, 4, &mut handle), "the menu adds three at most");
    assert!(!docks.add(2, 4, &mut handle), "a docked panel holds no dock of its own");
    assert_eq!((docks.panels(1), docks.focused(1), docks.tab_of(3), docks.is_inner(3), docks.is_inner(1)), (vec![1, 2, 3], 3, 1, true, false));
    assert_eq!(docks.remove(1), Some((1, 2)), "the tab's own panel left: the next first panel is the tab");
    assert_eq!((docks.panels(2), docks.tab_of(3)), (vec![2, 3], 2));
    assert_eq!(docks.remove(3), Some((2, 2)));
    assert!(docks.of(2).is_none() && !docks.is_inner(2), "a dock of one is no dock");
    assert_eq!(docks.remove(2), None);
}

const SIDS: [&str; 3] = ["0a0b0c0d-0000-4000-8000-00000000000a", "0a0b0c0d-0000-4000-8000-00000000000b", "0a0b0c0d-0000-4000-8000-00000000000c"];

fn hand(title: &str, sid: &str) -> crate::saved_tabs::SavedTab {
    crate::saved_tabs::SavedTab::Claude { title: title.into(), cwd: "C:/demo".into(), session_id: Some(sid.into()), name: title.into() }
}

fn placeholder(view: &Entity<FleetView>, saved: crate::saved_tabs::SavedTab, cx: &mut VisualTestContext) -> u64 {
    view.update(cx, |v, cx| v.push_tab(saved.title().to_string(), TabKind::Saved(saved), None, cx))
}

fn titled(view: &Entity<FleetView>, title: &str, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |v, _| v.tabs.iter().find(|t| t.title == title).map(|t| t.id)).unwrap_or_else(|| panic!("{title} is open"))
}

/// The tabs saved, closed, and reopened as a start plans them, each `failing` session without its conversation.
fn restart(view: &Entity<FleetView>, failing: &[&str], cx: &mut VisualTestContext) {
    let saved = view.read_with(cx, |v, cx| v.saved_tabs(cx));
    view.update(cx, |v, cx| v.header_ids().into_iter().for_each(|id| v.close_tab(id, cx)));
    assert!(view.read_with(cx, |v, _| v.tabs.is_empty()));
    let has = |sid: &str, _: &std::path::Path| !failing.contains(&sid);
    let world = crate::saved_tabs::World { sessions: &[], now: 0, targets: &[], resume_on_click: true, is_dir: &|_| true, is_file: &|_| true, has_transcript: &has };
    let plans = saved.tabs.iter().map(|t| (t.title().to_string(), crate::saved_tabs::plan(t, &world))).collect();
    view.update(cx, |v, cx| v.apply_reopen(Some((plans, saved.active, None)), cx));
    draw(view, cx);
}

/// Three placeholders `a`, `b`, `c` in one dock, `a` the tab, its splits at 0.3 and 0.6 and `b` focused, then a plain `d`.
fn three_docked(view: &Entity<FleetView>, cx: &mut VisualTestContext) -> (u64, supraman_ui::dock::layout::Layout<u64>) {
    let ids: Vec<u64> = ["a", "b", "c"].iter().zip(SIDS).map(|(t, sid)| placeholder(view, hand(t, sid), cx)).collect();
    placeholder(view, hand("d", "0a0b0c0d-0000-4000-8000-00000000000d"), cx);
    view.update(cx, |v, cx| {
        assert!(v.add_panel(ids[0], ids[1], cx) && v.add_panel(ids[0], ids[2], cx));
        let layout = v.docks.of(ids[0]).unwrap().layout.with_ratio(&[], 0.3).and_then(|l| l.with_ratio(&[1], 0.6)).expect("two splits");
        assert!(v.docks.set_layout(ids[0], layout.clone()));
        v.docks.focus(ids[1]);
        v.active = Some(ids[0]);
        (ids[0], layout)
    })
}

#[gpui::test]
fn saved_tabs_a_dock_reopens_with_its_panels_in_their_slots_its_ratios_and_its_focused_panel(cx: &mut TestAppContext) {
    let (view, cx) = window(cx);
    let (_, layout) = three_docked(&view, cx);
    let old: Vec<u64> = ["a", "b", "c"].iter().map(|t| titled(&view, t, cx)).collect();
    restart(&view, &[], cx);
    let new: Vec<u64> = ["a", "b", "c", "d"].iter().map(|t| titled(&view, t, cx)).collect();
    let moved = layout.map(&|p| new[old.iter().position(|o| *o == p).unwrap()]);
    view.read_with(cx, |v, _| {
        assert_eq!(v.header_ids(), vec![new[0], new[3]], "the dock's tab, then the plain one");
        let dock = v.docks.of(new[0]).expect("the dock is back");
        assert_eq!((&dock.layout, dock.focused, v.active), (&moved, new[1], Some(new[0])), "its slots, ratios and focused panel");
    });
}

#[gpui::test]
fn saved_tabs_a_dock_whose_panel_fails_reopens_with_the_rest(cx: &mut TestAppContext) {
    let (view, cx) = window(cx);
    three_docked(&view, cx);
    restart(&view, &[SIDS[1]], cx);
    let (a, c) = (titled(&view, "a", cx), titled(&view, "c", cx));
    view.read_with(cx, |v, _| {
        let dock = v.docks.of(a).expect("a and c stay docked");
        let closed_up = supraman_ui::dock::layout::Layout::Leaf(a).add(c).unwrap().with_ratio(&[], 0.3).unwrap();
        assert_eq!((&dock.layout, dock.focused), (&closed_up, a), "b closed up; its focus is the tab's");
        assert!(v.saving.tab_notices.iter().any(|n| n.contains("not reopened: b (")), "{:?}", v.saving.tab_notices);
    });
}

#[gpui::test]
fn saved_tabs_a_closed_dock_returns_whole_in_one_ctrl_alt_t_and_a_closed_panel_alone(cx: &mut TestAppContext) {
    let (f1, f2) = (Fixture::copy(), Fixture::copy());
    let (view, cx) = window(cx);
    let (g1, _) = git_panel(&view, &f1, cx);
    let (g2, _) = view.update(cx, |v, cx| (v.open_git(f2.dir.clone(), "Git · two".into(), cx), ()));
    let p = placeholder(&view, hand("lost", SIDS[0]), cx);
    view.update(cx, |v, cx| {
        assert!(v.add_panel(g1, g2, cx) && v.add_panel(g1, p, cx));
        let layout = v.docks.of(g1).unwrap().layout.with_ratio(&[], 0.3).unwrap();
        v.docks.set_layout(g1, layout);
        v.docks.focus(g2);
        v.close_tab(g1, cx);
    });
    assert!(view.read_with(cx, |v, _| v.tabs.is_empty()));

    view.update(cx, |v, cx| v.reopen_closed(cx));
    let tab = eventually(WITHIN, "the closed dock back", polled(Duration::from_millis(20)), || {
        draw(&view, cx);
        view.read_with(cx, |v, _| v.header_ids().first().copied().filter(|t| v.docks.of(*t).is_some()))
    });
    let two = titled(&view, "Git · two", cx);
    view.read_with(cx, |v, cx| {
        let dock = v.docks.of(tab).unwrap();
        let Layout::Split { ratio, .. } = dock.layout else { panic!("a split") };
        assert_eq!((v.header_ids().len(), dock.layout.len(), ratio, dock.focused, v.active), (1, 2, 0.3, two, Some(tab)), "both Git panels in one dock, the placeholder without its conversation left out");
        assert!(!v.tab_keys.read(cx).has_closed(), "one entry held the whole dock");
    });

    view.update(cx, |v, cx| v.close_panel(two, cx));
    view.update(cx, |v, cx| v.reopen_closed(cx));
    let back = eventually(WITHIN, "the closed panel back", polled(Duration::from_millis(20)), || {
        draw(&view, cx);
        view.read_with(cx, |v, _| v.tabs.iter().find(|t| t.title == "Git · two").map(|t| t.id))
    });
    assert_eq!(view.read_with(cx, |v, _| (v.header_ids(), v.docks.of(tab).is_none(), v.active)), (vec![tab, back], true, Some(back)), "the panel alone, its own tab");
}

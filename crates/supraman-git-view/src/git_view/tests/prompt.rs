//! A tab's prompt: modal over that tab alone, F6's way into it, and the keys given back as it closes; on a fixture copy.

use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_ui::dialog::PANE;

use super::*;
use crate::git_view::branch::menu::RefAct;
use crate::git_view::test_support::{self, Opts, click, pane_focused, redraw};

fn ask_delete(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.act(RefAct::DeleteBranch("feature".into()), cx)));
    cx.run_until_parked();
    redraw(view, cx);
    assert!(asking(view, cx), "the delete is asked");
}

fn asking(view: &Entity<GitView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, cx| v.branch.read(cx).asking().is_some())
}

fn keys(view: &Entity<GitView>, keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    redraw(view, cx);
}

#[gpui::test]
fn a_tabs_prompt_takes_that_tabs_keys_and_leaves_the_rest_of_the_window_its_own(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = test_support::open(&f.worktree, Opts::default().aside().size(1400., 900.), cx);
    ask_delete(&view, cx);
    assert_eq!(pane_focused(cx).as_deref(), Some(PANE), "the prompt takes the keys as it opens");

    keys(&view, "ctrl-p ctrl-j ctrl-l", cx);
    let (find, shut) = view.read_with(cx, |v, cx| (v.find.is_some(), v.ref_panel.read(cx).shut()));
    assert!(!find && !shut && asking(&view, cx), "no chord of the tab reaches it: Ctrl+P {find}, Ctrl+J {shut}");

    let (aside, typed) = test_support::aside(cx);
    cx.update(|window, cx| window.focus(&aside, cx));
    keys(&view, "x", cx);
    redraw(&view, cx);
    assert_eq!((typed.borrow().clone(), pane_focused(cx).as_deref()), (vec!["x".to_string()], Some("aside")), "the window's other pane keeps its keys, the prompt open beside it");

    let mut walked = Vec::new();
    for _ in 0..3 {
        keys(&view, "f6", cx);
        walked.push(pane_focused(cx).unwrap_or_default());
    }
    assert_eq!(walked, [PANE, "aside", PANE], "F6 lands in the prompt, never in the panes it covers");
    assert!(asking(&view, cx));
}

#[gpui::test]
fn closing_a_prompt_gives_the_keys_back_to_the_graph_row_that_held_them(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = test_support::open(&f.worktree, Opts::default().size(1400., 900.), cx);
    click(&view, "git-row-2", cx);
    let selected = view.read_with(cx, |v, cx| v.selected(cx));
    let graph = |cx: &mut VisualTestContext| cx.update(|window, cx| view.read(cx).table.focus_handle().is_focused(window));
    assert!(selected.is_some() && graph(cx), "a row selected, the graph holding the keys");

    ask_delete(&view, cx);
    keys(&view, "tab tab", cx);
    assert_eq!(pane_focused(cx).as_deref(), Some(PANE), "Tab walks the answers inside it");
    keys(&view, "escape", cx);
    assert!(!asking(&view, cx) && exists(&f), "Esc cancels");
    assert!(graph(cx) && view.read_with(cx, |v, cx| v.selected(cx)) == selected, "the keys back on the row that held them");

    ask_delete(&view, cx);
    keys(&view, "enter", cx);
    assert!(!asking(&view, cx) && exists(&f) && graph(cx), "Enter takes the default, Cancel for a delete, and gives the keys back");
    keys(&view, "ctrl-p", cx);
    assert!(view.read_with(cx, |v, _| v.find.is_some()), "the tab's chords reach it again");
}

fn exists(f: &Fixture) -> bool {
    supraman_gitman::fixture::run_git(&f.git, &f.dir, &["rev-parse", "-q", "--verify", "refs/heads/feature"], &[], None).code == Some(0)
}

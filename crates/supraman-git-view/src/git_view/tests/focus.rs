//! The graph's selection shows whether the graph has the keys; F6 steps through the tab's panes; a closed menu gives the keys back;
//! a tab shown gives the graph the keys on HEAD's commit.

use gpui::{Entity, Hsla, TestAppContext, VisualTestContext};
use theme::ActiveTheme as _;
use supraman_gitman::fixture::Fixture;
use supraman_ui::themes::selection;

use crate::git_view::test_support::{Opts, click, keys, menu, open, redraw, repaint, right_click, row_of, show};
use crate::git_view::*;

fn shown(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (Option<usize>, Option<Hsla>) {
    view.read_with(cx, |v, _| (v.table.selected(), v.table.selection_shown()))
}

#[gpui::test]
fn the_graph_selection_is_inactive_while_another_list_has_the_keys(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let colors = cx.update(|_, cx| cx.theme().colors().clone());
    let (active, inactive) = (selection(&colors, true), selection(&colors, false));
    let second = row_of(&view, f.sha("second"), cx);
    click(&view, &format!("git-row-{second}"), cx);
    assert_eq!(shown(&view, cx), (Some(second), Some(active)), "a click gives the graph the keys");

    click(&view, "refpanel-row-local/feature", cx);
    let rename = row_of(&view, f.sha("rename"), cx);
    assert_eq!(shown(&view, cx), (Some(rename), Some(inactive)), "the ref panel has the keys; the graph shows its commit inactive");
    keys(&view, "down", cx);
    let (next, fill) = shown(&view, cx);
    assert_ne!(next, Some(rename), "Down moves the ref panel to its next ref, whose commit the graph shows");
    assert_eq!(fill, Some(inactive), "the ref panel keeps the keys");

    let next = next.expect("a commit is selected");
    click(&view, &format!("git-row-{next}"), cx);
    assert_eq!(shown(&view, cx), (Some(next), Some(active)));
    keys(&view, "down", cx);
    assert_eq!(shown(&view, cx), (Some(next + 1), Some(active)), "Down moves the graph again");
}

fn holding(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, cx| supraman_ui::focus::focused(window, cx).map(|k| k.to_string()))
}

fn order(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, cx| supraman_ui::focus::order(window, cx).into_iter().map(|k| k.to_string()).collect())
}

/// The panes whose mark a frame drawn now paints.
fn marked(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<String> {
    let mut keys: Vec<String> = repaint(view, cx).borrow().keys().filter(|k| k.starts_with("focus-mark-")).cloned().collect();
    keys.sort();
    keys
}

fn mark(pane: &str) -> Vec<String> {
    vec![supraman_ui::focus::mark_key(pane)]
}

#[gpui::test]
fn f6_steps_through_the_ref_panel_graph_right_panel_and_diff_viewer_and_only_the_focused_pane_is_marked(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("loose.txt"), "loose\n").unwrap();
    let (view, cx) = open(&f.dir, Opts::default().editor(), cx);
    let colors = cx.update(|_, cx| cx.theme().colors().clone());
    let second = row_of(&view, f.sha("second"), cx);
    click(&view, &format!("git-row-{second}"), cx);
    assert_eq!(order(cx), [FOCUS_REFS, FOCUS_GRAPH, FOCUS_PANEL]);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_GRAPH));
    assert_eq!(marked(&view, cx), mark(FOCUS_GRAPH), "the graph a click focused wears the mark");

    keys(&view, "f6", cx);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_PANEL), "F6 from the graph: the commit panel");
    assert_eq!(marked(&view, cx), mark(FOCUS_PANEL), "the mark moves with the focus");
    assert_eq!(shown(&view, cx), (Some(second), Some(selection(&colors, false))), "the graph's selection turns inactive");
    keys(&view, "f6", cx);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_REFS), "wraps to the ref panel");
    keys(&view, "f6", cx);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_GRAPH));
    assert_eq!(shown(&view, cx), (Some(second), Some(selection(&colors, true))), "back on the graph, on its row");
    keys(&view, "shift-f6", cx);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_REFS), "Shift+F6 goes back");

    view.update(cx, |v, cx| v.open_diff(diff::Target { of: diff::Of::Untracked, path: "loose.txt".into(), change: None }, cx));
    cx.run_until_parked();
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_DIFF), "the viewer takes the keys as it opens");
    assert_eq!(order(cx), [FOCUS_REFS, FOCUS_PANEL, FOCUS_DIFF], "the viewer in the graph's place comes after the right panel");
    assert_eq!(marked(&view, cx), mark(FOCUS_DIFF));
    keys(&view, "shift-f6", cx);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_PANEL));
    keys(&view, "f6", cx);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_DIFF), "F6 from the right panel: the diff viewer");
    keys(&view, "f6", cx);
    assert_eq!(holding(cx).as_deref(), Some(FOCUS_REFS), "wraps");
}

#[gpui::test]
fn a_menu_opened_from_the_graph_and_closed_by_esc_gives_the_graph_the_keys_back(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let second = row_of(&view, f.sha("second"), cx);
    click(&view, &format!("git-row-{second}"), cx);
    right_click(&view, &format!("git-row-{second}"), cx);
    assert!(!menu(&view, cx).is_empty(), "the row's menu is open");
    assert_eq!(holding(cx), None, "the menu holds the keys");
    assert!(marked(&view, cx).is_empty(), "no pane is marked while the menu has the keys");

    keys(&view, "escape", cx);
    assert!(menu(&view, cx).is_empty(), "Esc closes the menu");
    let graph = view.read_with(cx, |v, _| v.table.focus_handle().clone());
    assert!(cx.update(|window, _| graph.is_focused(window)), "the graph has the keys again");
    assert_eq!(marked(&view, cx), mark(FOCUS_GRAPH));
}

fn graph_has_keys(view: &Entity<GitView>, cx: &mut VisualTestContext) -> bool {
    let graph = view.read_with(cx, |v, _| v.table.focus_handle().clone());
    cx.update(|window, cx| graph.contains_focused(window, cx))
}

fn head(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<Selected> {
    view.read_with(cx, |v, _| v.heads.head.clone()).map(Selected::Commit)
}

#[gpui::test]
fn a_tab_shown_gives_the_graph_the_keys_on_heads_commit_and_ctrl_p_opens_the_palette(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default().hidden(), cx);
    assert!(!graph_has_keys(&view, cx), "the window's own element holds the keys before the tab is shown");
    show(cx);
    redraw(&view, cx);
    assert!(graph_has_keys(&view, cx), "the shown tab's graph has the keys");
    assert!(head(&view, cx).is_some());
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), head(&view, cx), "HEAD's commit is selected");
    keys(&view, "ctrl-p", cx);
    assert!(view.read_with(cx, |v, _| v.find().is_some()), "Ctrl+P opens the palette with no click");
    assert!(repaint(&view, cx).borrow().contains_key(crate::git_view::find::ID), "the palette is painted");
}

#[gpui::test]
fn a_tab_shown_while_loading_selects_heads_commit_once_the_rows_land(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default().hidden(), cx);
    view.update(cx, |v, cx| v.reload(cx));
    show(cx);
    assert!(view.read_with(cx, |v, _| matches!(v.load, Load::Loading)), "still reading the history");
    let root = view.read_with(cx, |v, _| v.focus.clone());
    assert!(cx.update(|window, _| root.is_focused(window)), "the tab holds the keys while it loads");
    redraw(&view, cx);
    assert!(graph_has_keys(&view, cx), "the graph has the keys once its rows land");
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), head(&view, cx), "HEAD's commit is selected");
}

#[gpui::test]
fn a_selection_already_there_stays_when_the_graph_takes_the_keys(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default().hidden(), cx);
    let second = row_of(&view, f.sha("second"), cx);
    view.read_with(cx, |v, _| v.table.select(Some(second)));
    show(cx);
    redraw(&view, cx);
    assert!(graph_has_keys(&view, cx));
    assert_eq!(view.read_with(cx, |v, _| v.table.selected()), Some(second));
}

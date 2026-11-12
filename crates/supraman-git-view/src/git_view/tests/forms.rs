//! The tab's form slot between the ref panel and the graph: Add Remote, Edit Remote and Create Worktree, on a fixture copy and its bare temp remote.

use gpui::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::remotes;

use super::super::remote::add;
use super::*;
use crate::git_view::test_support::{self, Opts, click, keys, redraw, right_click, settle, toast};

const WORKTREE: &str = "git-add-worktree";

/// The ref panel's, the open form's (when `form`), the graph's and the commit panel's bounds in a frame drawn now.
fn places(view: &Entity<GitView>, form: Option<&str>, cx: &mut VisualTestContext) -> (Bounds<Pixels>, Option<Bounds<Pixels>>, Bounds<Pixels>, Bounds<Pixels>) {
    let hits = test_support::repaint(view, cx);
    let at = |k: &str| hits.borrow().get(k).map(|h| h.bounds).unwrap_or_else(|| panic!("{k} is painted"));
    (at(REFS_PANE), form.map(at), at(GRAPH_PANE), at(PANEL_PANE))
}

/// The form lies between the ref panel and the graph, `FORM_WIDTH` wide, the commit panel as it was.
fn beside(view: &Entity<GitView>, form: &str, before: Bounds<Pixels>, cx: &mut VisualTestContext) -> (Bounds<Pixels>, Bounds<Pixels>) {
    let (refs, at, graph, panel) = places(view, Some(form), cx);
    let at = at.unwrap();
    assert!(refs.right() <= at.left() && at.left() - refs.right() < px(8.), "{form} right of the ref panel: {refs:?} {at:?}");
    assert!(at.right() <= graph.left() && graph.left() - at.right() < px(2.), "{form} left of the graph: {at:?} {graph:?}");
    assert!((at.size.width - px(FORM_WIDTH)).abs() < px(2.), "{form} is {FORM_WIDTH} wide: {at:?}");
    assert_eq!(panel, before, "{form}: the commit panel stays as it was");
    (refs, graph)
}

fn open_form(view: &Entity<GitView>, plus: &str, cx: &mut VisualTestContext) {
    if plus == "refpanel-worktrees-add" {
        let hits = test_support::repaint(view, cx);
        let header = hits.borrow().get("refpanel-row-worktrees").map(|h| h.bounds.center()).expect("the WORKTREES header");
        cx.simulate_mouse_move(header, None, gpui::Modifiers::none());
        redraw(view, cx);
    }
    click(view, plus, cx);
    settle(view, cx);
}

#[gpui::test]
fn a_form_opens_between_the_ref_panel_and_the_graph_and_takes_its_width_from_the_graph(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    let (refs_before, _, graph_before, panel) = places(&view, None, cx);

    open_form(&view, "refpanel-remote-add", cx);
    let (refs, graph) = beside(&view, add::KEY, panel, cx);
    assert_eq!(refs, refs_before, "a wide tab keeps the ref panel's width");
    assert!((graph_before.size.width - graph.size.width - px(FORM_WIDTH)).abs() < px(2.), "the graph gives the form its width: {graph_before:?} {graph:?}");

    open_form(&view, "refpanel-worktrees-add", cx);
    beside(&view, WORKTREE, panel, cx);
    keys(&view, "escape", cx);
    keys(&view, "escape", cx);
    assert_eq!(places(&view, None, cx), (refs_before, None, graph_before, panel), "closed, the tab is as it was");
}

#[gpui::test]
fn in_a_narrow_tab_the_form_takes_the_graphs_width_and_then_the_ref_panels(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1100., 700.), cx);
    let (refs_before, _, graph_before, panel) = places(&view, None, cx);

    open_form(&view, "refpanel-remote-add", cx);
    let (refs, graph) = beside(&view, add::KEY, panel, cx);
    assert!(graph.size.width < graph_before.size.width && graph.size.width >= px(GRAPH_MIN) - px(2.), "the graph gives down to its least: {graph_before:?} {graph:?}");
    assert!(refs.size.width < refs_before.size.width, "then the ref panel gives the rest: {refs_before:?} {refs:?}");
}

#[gpui::test]
fn one_form_at_a_time(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    let open = |view: &Entity<GitView>, cx: &mut VisualTestContext| {
        let hits = test_support::repaint(view, cx);
        [add::KEY, add::EDIT_KEY, WORKTREE].into_iter().filter(|k| hits.borrow().contains_key(*k)).collect::<Vec<_>>()
    };

    open_form(&view, "refpanel-worktrees-add", cx);
    assert_eq!(open(&view, cx), [WORKTREE]);
    open_form(&view, "refpanel-remote-add", cx);
    assert_eq!(open(&view, cx), [add::KEY], "Add Remote closes Create Worktree");
    open_form(&view, "refpanel-worktrees-add", cx);
    assert_eq!(open(&view, cx), [WORKTREE], "Create Worktree closes Add Remote");
    right_click(&view, "refpanel-row-remote/origin", cx);
    click(&view, "menu-remote-edit-origin", cx);
    settle(&view, cx);
    assert_eq!(open(&view, cx), [add::EDIT_KEY], "Edit Remote closes Create Worktree");
}

fn edit(view: &Entity<GitView>, name: &str, cx: &mut VisualTestContext) -> [String; 3] {
    right_click(view, &format!("refpanel-row-remote/{name}"), cx);
    click(view, &format!("menu-remote-edit-{name}"), cx);
    settle(view, cx);
    view.read_with(cx, |v, cx| v.remote.read(cx).adding().expect("Edit Remote is open").texts(cx))
}

fn type_in(view: &Entity<GitView>, at: usize, text: &str, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.adding().unwrap().type_in(at, text, cx)));
    redraw(view, cx);
}

#[gpui::test]
fn edit_remote_opens_filled_and_saves_its_url_push_url_and_name(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    let was = remotes(&f.repo()).unwrap().into_iter().find(|r| r.name == "origin").unwrap();

    assert_eq!(edit(&view, "origin", cx), ["origin".to_string(), was.url.clone(), String::new()], "the form holds the remote as it is");
    for key in [add::EDIT_KEY, "git-edit-remote-name", "git-edit-remote-pull", "git-edit-remote-push", "git-edit-remote-ok", "git-edit-remote-close"] {
        assert!(test_support::painted(&view, key, cx), "{key}");
    }
    assert!(!test_support::painted(&view, "git-edit-remote-band", cx), "no band");
    type_in(&view, 2, "C:/push-here.git", cx);
    keys(&view, "enter", cx);
    settle(&view, cx);
    assert_eq!(toast(&view, cx).as_deref(), Some("Saved remote origin"));
    assert!(!test_support::painted(&view, add::EDIT_KEY, cx), "saved, the form closes");
    let read = remotes(&f.repo()).unwrap();
    assert_eq!((read[0].name.as_str(), read[0].url.as_str(), read[0].push_url.as_deref()), ("origin", was.url.as_str(), Some("C:/push-here.git")));

    let elsewhere = f.temp.path().join("elsewhere.git").to_string_lossy().replace('\\', "/");
    assert_eq!(edit(&view, "origin", cx)[2], "C:/push-here.git");
    type_in(&view, 0, "upstream", cx);
    type_in(&view, 1, &elsewhere, cx);
    type_in(&view, 2, "", cx);
    click(&view, "git-edit-remote-ok", cx);
    settle(&view, cx);
    assert_eq!(toast(&view, cx).as_deref(), Some("Saved remote upstream"));
    let read = remotes(&f.repo()).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!((read[0].name.as_str(), read[0].url.as_str(), read[0].push_url.as_deref()), ("upstream", elsewhere.as_str(), None), "renamed, its URL set and an empty push URL removed");
    assert!(test_support::painted(&view, "refpanel-row-remote/upstream", cx), "the REMOTE section lists it by its new name");

    edit(&view, "upstream", cx);
    type_in(&view, 1, "", cx);
    assert_eq!(view.read_with(cx, |v, cx| v.remote.read(cx).adding().unwrap().why_not(cx)), Some(add::NEEDS_URL), "an empty URL dims Save");
    keys(&view, "escape", cx);
    assert!(!test_support::painted(&view, add::EDIT_KEY, cx), "Esc closes");
}

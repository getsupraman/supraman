//! The open form laid over the graph at the ref panel's right edge: Add Remote, Edit Remote and Create Worktree, on a fixture copy and its bare temp remote.

use gpui::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::remotes;

use super::super::remote::add;
use super::*;
use crate::git_view::test_support::{self, Opts, click, keys, redraw, right_click, settle, toast};

const WORKTREE: &str = "git-add-worktree";

/// The ref panel's, the open form's (when `form`), the graph's and the commit panel's bounds in a frame drawn now.
fn places(view: &Entity<GitView>, form: Option<&str>, cx: &mut VisualTestContext) -> (Option<Bounds<Pixels>>, Option<Bounds<Pixels>>, Bounds<Pixels>, Bounds<Pixels>) {
    let hits = test_support::repaint(view, cx);
    let at = |k: &str| hits.borrow().get(k).map(|h| h.bounds).unwrap_or_else(|| panic!("{k} is painted"));
    (hits.borrow().get(REFS_PANE).map(|h| h.bounds), form.map(at), at(GRAPH_PANE), at(PANEL_PANE))
}

/// The form lies over the graph from its left edge, `FORM_WIDTH` wide (or the graph's width when narrower) and the column's height; nothing else moves.
fn over(view: &Entity<GitView>, form: &str, closed: (Option<Bounds<Pixels>>, Bounds<Pixels>, Bounds<Pixels>), cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let (refs, at, graph, panel) = places(view, Some(form), cx);
    let at = at.unwrap();
    assert_eq!((refs, graph, panel), closed, "{form}: the ref panel, the graph and the commit panel stay where they were");
    if let Some(refs) = refs {
        assert!(refs.right() <= at.left() && at.left() - refs.right() < px(8.), "{form} at the ref panel's right edge: {refs:?} {at:?}");
        assert_eq!((at.top(), at.bottom()), (refs.top(), refs.bottom()), "{form} spans the ref panel's column");
    }
    assert_eq!(at.left(), graph.left(), "{form} starts at the graph's left edge: {at:?} {graph:?}");
    assert_eq!((at.top(), at.bottom()), (graph.top(), graph.bottom()), "{form} is the graph's height");
    assert!((at.size.width - px(FORM_WIDTH).min(graph.size.width)).abs() < px(2.), "{form} is {FORM_WIDTH} wide, within the graph: {at:?} {graph:?}");
    at
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
fn a_form_covers_the_graph_at_the_ref_panels_edge_and_nothing_moves(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    let (refs, _, graph, panel) = places(&view, None, cx);

    open_form(&view, "refpanel-remote-add", cx);
    over(&view, add::KEY, (refs, graph, panel), cx);
    open_form(&view, "refpanel-worktrees-add", cx);
    over(&view, WORKTREE, (refs, graph, panel), cx);
    keys(&view, "escape", cx);
    keys(&view, "escape", cx);
    assert_eq!(places(&view, None, cx), (refs, None, graph, panel), "closed, the tab is as it was");
}

#[gpui::test]
fn a_click_on_a_pill_under_the_form_lands_on_the_form(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    open_form(&view, "refpanel-remote-add", cx);
    let hits = test_support::repaint(&view, cx);
    let form = hits.borrow().get(add::KEY).unwrap().bounds;
    let selected = view.read_with(cx, |v, cx| v.selected(cx));
    let under = hits.borrow().iter().filter(|(k, h)| k.starts_with("git-pill-") && form.contains(&h.bounds.center())).map(|(k, _)| k.clone()).collect::<Vec<_>>();
    assert!(under.len() > 1, "pills lie under the form: {under:?}");
    for pill in under {
        click(&view, &pill, cx);
        assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), selected, "{pill}: the click stays on the form");
        assert!(test_support::painted(&view, add::KEY, cx), "{pill}: the form stays open");
    }
    let outside = hits.borrow().get(PANEL_PANE).unwrap().bounds;
    assert!(!form.intersects(&outside), "the commit panel lies outside the form");
}

#[gpui::test]
fn with_the_ref_panel_shut_the_form_starts_at_the_strip(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    view.update(cx, |v, cx| v.ref_panel.update(cx, |p, cx| p.toggle_shut(cx)));
    let (refs, _, graph, panel) = places(&view, None, cx);
    assert_eq!(refs, None, "shut, the ref panel is its strip");
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.open_add(cx)));
    settle(&view, cx);
    let at = over(&view, add::KEY, (refs, graph, panel), cx);
    assert!(at.left() > px(0.) && at.left() < px(60.), "the form starts at the strip's right edge: {at:?}");
}

#[gpui::test]
fn in_a_narrow_tab_the_form_stays_within_the_graph_and_nothing_moves(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    for width in [1300., 1100., 950., 700.] {
        let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(width, 700.), cx);
        let (refs, _, graph, panel) = places(&view, None, cx);
        open_form(&view, "refpanel-remote-add", cx);
        let at = over(&view, add::KEY, (refs, graph, panel), cx);
        assert!(at.right() <= graph.right(), "{width}: the form ends within the graph: {at:?} {graph:?}");
    }
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

    f.git(&["remote", "add", "mirror", "C:/mirror.git"]);
    edit(&view, "upstream", cx);
    type_in(&view, 0, "mirror", cx);
    type_in(&view, 1, "C:/other.git", cx);
    click(&view, "git-edit-remote-ok", cx);
    settle(&view, cx);
    assert_eq!(toast(&view, cx).as_deref(), Some("Remote mirror already exists."), "git's refusal in its words");
    assert!(test_support::painted(&view, add::EDIT_KEY, cx), "the form stays open");
    let read = remotes(&f.repo()).unwrap();
    let of = |name: &str| read.iter().find(|r| r.name == name).map(|r| r.url.as_str());
    assert_eq!((of("upstream"), of("mirror")), (Some(elsewhere.as_str()), Some("C:/mirror.git")), "a refused rename writes nothing, the URL included");

    type_in(&view, 1, "", cx);
    assert_eq!(view.read_with(cx, |v, cx| v.remote.read(cx).adding().unwrap().why_not(cx)), Some(add::NEEDS_URL), "an empty URL dims Save");
    keys(&view, "escape", cx);
    assert!(!test_support::painted(&view, add::EDIT_KEY, cx), "Esc closes");
}

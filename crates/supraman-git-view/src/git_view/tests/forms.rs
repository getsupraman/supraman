//! The open form, the tab's prompt over it: Add Remote, Edit Remote and Create Worktree, on a fixture copy and its bare temp remote.

use gpui::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::remotes;

use super::super::remote::add;
use super::*;
use crate::git_view::test_support::{self, Opts, click, keys, redraw, right_click, settle, toast};

const WORKTREE: &str = "git-add-worktree";

/// The form's scrim and panel in a frame drawn now: the scrim over the whole tab, the panel centred on it within its margin.
fn prompt(view: &Entity<GitView>, form: &str, cx: &mut VisualTestContext) -> (Bounds<Pixels>, Bounds<Pixels>) {
    let hits = test_support::repaint(view, cx);
    let at = |k: &str| hits.borrow().get(k).map(|h| h.bounds).unwrap_or_else(|| panic!("{k} is painted"));
    let (toolbar, graph, scrim, panel) = (at("git-toolbar"), at(GRAPH_PANE), at(&format!("{form}-scrim")), at(&format!("{form}-dialog")));
    assert_eq!((scrim.origin, scrim.size.width), (toolbar.origin, toolbar.size.width), "{form}: the scrim from the tab's top across it");
    assert!(scrim.bottom() >= graph.bottom(), "{form}: down to the tab's bottom: {scrim:?} {graph:?}");
    assert!((panel.center() - scrim.center()).x.abs() < px(1.) && (panel.center() - scrim.center()).y.abs() < px(1.), "{form}: centred: {panel:?} {scrim:?}");
    let margin = supraman_ui::dialog::MARGIN - px(1.);
    assert!(panel.left() >= scrim.left() + margin && panel.right() <= scrim.right() - margin && panel.size.width <= supraman_ui::dialog::WIDTH, "{form}: within the tab: {panel:?} {scrim:?}");
    (scrim, panel)
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
fn a_form_is_a_prompt_centred_over_the_tab_and_closing_it_leaves_the_tab_as_it_was(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    let before = test_support::repaint(&view, cx).borrow().get(GRAPH_PANE).map(|h| h.bounds);
    open_form(&view, "refpanel-remote-add", cx);
    prompt(&view, add::KEY, cx);
    keys(&view, "escape", cx);
    open_form(&view, "refpanel-worktrees-add", cx);
    prompt(&view, WORKTREE, cx);
    keys(&view, "escape", cx);
    keys(&view, "escape", cx);
    let hits = test_support::repaint(&view, cx);
    assert!(!hits.borrow().contains_key(&format!("{WORKTREE}-dialog")), "closed");
    assert_eq!(hits.borrow().get(GRAPH_PANE).map(|h| h.bounds), before, "the graph where it was");
}

#[gpui::test]
fn in_a_narrow_tab_the_form_stays_within_it(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    for width in [1300., 950., 500.] {
        let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(width, 700.), cx);
        view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.open_add(cx)));
        settle(&view, cx);
        prompt(&view, add::KEY, cx);
    }
}

#[gpui::test]
fn a_click_on_a_pill_under_the_scrim_does_nothing_and_the_form_stays(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().home(home.path()).size(1400., 900.), cx);
    open_form(&view, "refpanel-remote-add", cx);
    let (scrim, _) = prompt(&view, add::KEY, cx);
    let hits = test_support::repaint(&view, cx);
    let selected = view.read_with(cx, |v, cx| v.selected(cx));
    let under = hits.borrow().iter().filter(|(k, h)| (k.starts_with("git-pill-") || k.starts_with("refpanel-row-")) && scrim.contains(&h.bounds.center())).map(|(k, _)| k.clone()).collect::<Vec<_>>();
    assert!(under.len() > 1, "pills and ref rows lie under the scrim: {under:?}");
    for key in under {
        click(&view, &key, cx);
        assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), selected, "{key}: the click stays on the scrim");
        assert!(test_support::painted(&view, add::KEY, cx), "{key}: the form stays open");
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

    let event = |view: &Entity<GitView>, e: ref_panel::RefEvent, cx: &mut VisualTestContext| {
        view.update(cx, |v, cx| {
            let panel = v.ref_panel.clone();
            v.ref_event(panel, &e, cx);
        });
        settle(view, cx);
    };

    open_form(&view, "refpanel-worktrees-add", cx);
    assert_eq!(open(&view, cx), [WORKTREE]);
    click(&view, "refpanel-remote-add", cx);
    settle(&view, cx);
    assert_eq!(open(&view, cx), [WORKTREE], "the ref panel's + under the prompt's scrim opens nothing");
    event(&view, ref_panel::RefEvent::AddRemote, cx);
    assert_eq!(open(&view, cx), [add::KEY], "Add Remote closes Create Worktree");
    event(&view, ref_panel::RefEvent::AddWorktree, cx);
    assert_eq!(open(&view, cx), [WORKTREE], "Create Worktree closes Add Remote");
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.act(remote::RemoteAct::EditRemote("origin".into()), cx)));
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

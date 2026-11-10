use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_ui::palette::Entry as _;

use super::entries::{Command, GitEntry};
use super::*;
use crate::git_view::diff::Of;
use crate::git_view::test_support::{keys, redraw};
use crate::git_view::{GitView, Selected};

fn open<'a>(folder: &std::path::Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    let (view, cx) = crate::git_view::test_support::open(folder, crate::git_view::test_support::Opts::default().editor(), cx);
    view.update_in(cx, |v, window, cx| window.focus(&v.focus, cx));
    (view, cx)
}

fn typed(view: &Entity<GitView>, text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    cx.run_until_parked();
    redraw(view, cx);
}

fn palette(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Entity<Palette<GitEntry>> {
    view.read_with(cx, |v, _| v.find().cloned()).expect("the palette is open")
}

/// The mode shown, the query and the keys of the rows listed.
fn listed(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (Option<String>, String, Vec<String>) {
    let p = palette(view, cx);
    p.read_with(cx, |p, cx| (p.mode().map(|m| m.to_string()), p.query(cx), p.shown().iter().map(|(e, _)| e.key().to_string()).collect()))
}

fn is_open(view: &Entity<GitView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.find().is_some())
}

fn commit(sha: &str) -> Option<Selected> {
    Some(Selected::Commit(sha.to_string().into()))
}

#[gpui::test]
fn c1_ctrl_p_finds_a_branch_a_file_and_a_commit_by_its_sha(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    keys(&view, "ctrl-p", cx);
    let (mode, _, top) = listed(&view, cx);
    assert_eq!(mode, None, "the top list");
    assert!(top.contains(&"cmd:refresh".to_string()) && top.contains(&"mode:refs".to_string()), "commands and modes: {top:?}");
    view.read_with(cx, |v, _| v.hits.borrow().get(ID).cloned()).expect("the palette is painted");

    typed(&view, "@", cx);
    assert_eq!(listed(&view, cx).0.as_deref(), Some(REFS), "@ enters Refs, the prefix taken off the query");
    assert_eq!(listed(&view, cx).1, "");
    view.read_with(cx, |v, _| v.hits.borrow().get(&format!("{ID}-chip")).cloned()).expect("the mode's chip");
    typed(&view, "featur", cx);
    assert_eq!(listed(&view, cx).2.first().map(String::as_str), Some("ref:Local:feature"));
    keys(&view, "enter", cx);
    assert!(!is_open(&view, cx), "Enter closes it");
    assert_eq!(view.read_with(cx, |v, _| v.selected()), commit(f.sha("rename")), "the branch's commit selected in the graph");

    keys(&view, "ctrl-p", cx);
    typed(&view, "/new", cx);
    let (mode, query, files) = listed(&view, cx);
    assert_eq!((mode.as_deref(), query.as_str()), (Some(FILES), "new"));
    assert_eq!(files.first().map(String::as_str), Some("file:new.txt"), "the files at HEAD, read when it opened: {files:?}");
    keys(&view, "enter", cx);
    let target = view.read_with(cx, |v, cx| v.diff(cx).map(|d| d.read(cx).target.clone())).expect("the viewer shows the file");
    assert_eq!((target.path.as_str(), target.of, target.change), ("new.txt", Of::Commit { sha: f.sha("ahead").to_string(), parent: None }, None), "the file at HEAD");

    keys(&view, "ctrl-p", cx);
    typed(&view, &format!("#{}", &f.sha("second")[..6]), cx);
    assert_eq!(listed(&view, cx).2.first().cloned(), Some(format!("commit:{}", f.sha("second"))));
    keys(&view, "enter", cx);
    assert_eq!(view.read_with(cx, |v, cx| (v.selected(), v.diff(cx).is_some())), (commit(f.sha("second")), false), "the commit selected, the viewer closed");
}

#[gpui::test]
fn ctrl_shift_p_opens_the_same_palette_as_ctrl_p(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    for key in ["ctrl-p", "ctrl-shift-p"] {
        keys(&view, key, cx);
        assert!(is_open(&view, cx), "{key} opens it");
        let (mode, _, top) = listed(&view, cx);
        assert_eq!(mode, None, "{key}: the top list");
        assert!(top.contains(&"cmd:refresh".to_string()) && top.contains(&"cmd:push".to_string()), "{key}: {top:?}");
        keys(&view, "escape", cx);
        assert!(!is_open(&view, cx));
    }
}

#[gpui::test]
fn backspace_on_an_empty_query_leaves_a_mode_and_ctrl_shift_h_opens_history(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    keys(&view, "ctrl-p", cx);
    typed(&view, "#ab", cx);
    keys(&view, "backspace", cx);
    assert_eq!(listed(&view, cx).0.as_deref(), Some(COMMITS), "a query left: Backspace edits it");
    keys(&view, "backspace", cx);
    assert_eq!(listed(&view, cx).0.as_deref(), Some(COMMITS));
    keys(&view, "backspace", cx);
    assert_eq!(listed(&view, cx).0, None, "on an empty query it leaves the mode");
    keys(&view, "backspace", cx);
    assert!(is_open(&view, cx), "and in the top list does nothing");
    typed(&view, "files", cx);
    keys(&view, "enter", cx);
    assert_eq!(listed(&view, cx).0.as_deref(), Some(FILES), "a mode row enters its mode");
    keys(&view, "escape", cx);
    assert!(!is_open(&view, cx), "Esc closes");

    keys(&view, "ctrl-shift-h", cx);
    let (mode, _, files) = listed(&view, cx);
    assert_eq!(mode.as_deref(), Some(HISTORY));
    assert!(files.contains(&"history:a.txt".to_string()), "{files:?}");
}

#[gpui::test]
fn a_command_that_cannot_act_is_listed_dimmed_and_its_pick_does_nothing(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    keys(&view, "ctrl-p", cx);
    typed(&view, "close the file", cx);
    let p = palette(&view, cx);
    let first = p.read_with(cx, |p, _| p.selected().cloned());
    assert_eq!(first, Some(GitEntry::Command { command: Command::CloseViewer, why_not: Some("No file is open in the viewer.".into()) }));
    view.read_with(cx, |v, _| v.hits.borrow().get(&format!("{ID}-row-cmd:close-viewer")).cloned()).expect("listed, not hidden");
    keys(&view, "enter", cx);
    assert!(is_open(&view, cx), "a dimmed command's Enter does nothing");
    keys(&view, "escape", cx);

    view.update(cx, |v, cx| v.open_diff(Target::commit(f.sha("second").into(), None, "a.txt".into(), None), cx));
    keys(&view, "ctrl-p", cx);
    typed(&view, "close the file", cx);
    let first = palette(&view, cx).read_with(cx, |p, _| p.selected().cloned());
    assert_eq!(first, Some(GitEntry::Command { command: Command::CloseViewer, why_not: None }));
    keys(&view, "enter", cx);
    assert_eq!(view.read_with(cx, |v, cx| (v.find().is_some(), v.diff(cx).is_some())), (false, false), "it acts once it can");
}

#[gpui::test]
fn a_command_picked_ranks_first_next_time_among_equal_matches(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    keys(&view, "ctrl-p", cx);
    let top = listed(&view, cx).2;
    assert_eq!(top.first().map(String::as_str), Some("cmd:refresh"), "the commands in their order at first");
    typed(&view, "go to", cx);
    let shown = listed(&view, cx).2;
    assert_eq!(shown[..2], ["cmd:go-to-head", "cmd:go-to-upstream"], "equal matches in their order: {shown:?}");
    keys(&view, "down enter", cx);
    assert!(!is_open(&view, cx));

    keys(&view, "ctrl-p", cx);
    typed(&view, "go to", cx);
    assert_eq!(listed(&view, cx).2.first().map(String::as_str), Some("cmd:go-to-upstream"), "the pick ranks first for an equal match");
    keys(&view, "escape", cx);
    keys(&view, "ctrl-p", cx);
    assert_eq!(listed(&view, cx).2.first().map(String::as_str), Some("cmd:go-to-upstream"), "and for the empty query");
    typed(&view, "go to h", cx);
    assert_eq!(listed(&view, cx).2.first().map(String::as_str), Some("cmd:go-to-head"), "a better match beats it");
}

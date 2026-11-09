use gpui::{Entity, KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, Generated};

use super::diff::{FileStep, Of, Target};
use super::keys::SETTLE;
use super::tests::redraw;
use super::*;
use crate::git_view::wip::Section;

fn open<'a>(folder: &std::path::Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| supraman_editor::init(supraman_ui::fonts::MONO, 13., cx));
    super::tests::open(folder, cx)
}

/// Clicks row `ix`, which gives the graph the keyboard and reads the row at once.
fn click_row(view: &Entity<GitView>, ix: usize, cx: &mut VisualTestContext) {
    let at = view.read_with(cx, |v, _| v.hits.borrow()[&format!("git-row-{ix}")].bounds.center());
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
    redraw(view, cx);
}

fn keys(view: &Entity<GitView>, keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    redraw(view, cx);
}

fn key_up(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    cx.simulate_event(KeyUpEvent { keystroke: Keystroke::parse(key).unwrap() });
    cx.run_until_parked();
    redraw(view, cx);
}

fn selected_sha(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<Selected> {
    view.read_with(cx, |v, _| v.selected())
}

fn commit(sha: &str) -> Option<Selected> {
    Some(Selected::Commit(sha.to_string().into()))
}

fn open_target(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<Target> {
    view.read_with(cx, |v, cx| v.diff().map(|d| d.read(cx).target.clone()))
}

fn paths(targets: &[Target]) -> Vec<&str> {
    targets.iter().map(|t| t.path.as_str()).collect()
}

#[test]
fn a_file_step_wraps_at_both_ends() {
    use FileStep::*;
    assert_eq!([Next, Previous, First, Last].map(|s| s.index(0, None)), [None; 4], "an empty list leads nowhere");
    assert_eq!([Next, Previous].map(|s| s.index(3, None)), [Some(0), Some(2)]);
    assert_eq!([Next, Previous].map(|s| s.index(3, Some(1))), [Some(2), Some(0)]);
    assert_eq!(Next.index(3, Some(2)), Some(0), "from the last to the first");
    assert_eq!(Previous.index(3, Some(0)), Some(2), "from the first to the last");
    assert_eq!([First, Last].map(|s| s.index(3, Some(1))), [Some(0), Some(2)]);
}

#[gpui::test]
fn graph_keys_walk_parents_children_head_and_upstream(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    let upstream = f.git(&["rev-parse", "origin/main"]);
    assert_eq!(upstream, f.sha("merge"), "the fixture's origin/main");
    click_row(&view, 3, cx);
    let steps: [(&str, Option<Selected>, &str); 8] = [
        ("ctrl-home", Some(Selected::Wip), "Ctrl+Home: the WIP row"),
        ("shift-down", commit(f.sha("ahead")), "the WIP row's parent is HEAD's commit"),
        ("shift-down", commit(f.sha("merge")), "Shift+Down: the first parent"),
        ("shift-down", commit(f.sha("main_work")), "a merge's first parent, not its second"),
        ("shift-up", commit(f.sha("merge")), "Shift+Up: the child"),
        ("shift-up", commit(f.sha("ahead")), "and its child, not a stash made on the merge"),
        ("shift-up", Some(Selected::Wip), "HEAD's child is the WIP row"),
        ("h", commit(f.sha("ahead")), "H: HEAD's commit"),
    ];
    for (key, want, what) in steps {
        keys(&view, key, cx);
        assert_eq!(selected_sha(&view, cx), want, "{what}");
    }
    keys(&view, "shift-h", cx);
    assert_eq!(selected_sha(&view, cx), commit(&upstream), "Shift+H: HEAD's upstream");
    let rename = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.sha.as_ref() == f.sha("rename")).unwrap());
    click_row(&view, rename, cx);
    keys(&view, "shift-up", cx);
    assert_eq!(selected_sha(&view, cx), commit(f.sha("merge")), "a merged commit's child is the merge");
    let stash = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.sha.as_ref() == f.sha("stash_message_index")).unwrap());
    click_row(&view, stash, cx);
    keys(&view, "shift-down", cx);
    assert_eq!(selected_sha(&view, cx), commit(f.sha("merge")), "Shift+Down from a stash: its base");
    keys(&view, "ctrl-end", cx);
    let last = view.read_with(cx, |v, _| v.rows.last().map(|r| r.sha.to_string()));
    assert_eq!(selected_sha(&view, cx), last.as_deref().and_then(commit), "Ctrl+End: the last loaded commit");
    keys(&view, "ctrl-up k j", cx);
    assert_eq!(selected_sha(&view, cx), commit(f.sha("ahead")), "Ctrl+Up the WIP row, K up (stays), J down");
    keys(&view, "left", cx);
    assert!(selected_sha(&view, cx) == commit(f.sha("ahead")) && open_target(&view, cx).is_none(), "Left in the graph does nothing");
}

#[gpui::test]
fn page_keys_move_a_page_and_keep_the_selection_in_view(cx: &mut TestAppContext) {
    let g = Generated::shared(1_000);
    let (view, cx) = open(&g.dir, cx);
    click_row(&view, 0, cx);
    let page = view.read_with(cx, |v, _| v.table.page());
    assert!(page > 10, "a page of rows: {page}");
    keys(&view, "pagedown pagedown", cx);
    assert_eq!(view.read_with(cx, |v, _| v.table.selected()), Some(2 * page));
    let painted = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.hits.borrow().contains_key(&format!("git-row-{}", 2 * page)));
    view.read_with(cx, |v, _| v.hits.borrow_mut().clear());
    redraw(&view, cx);
    assert!(painted(cx), "the selected row is in view");
    keys(&view, "pageup", cx);
    assert_eq!(view.read_with(cx, |v, _| v.table.selected()), Some(page));
}

#[gpui::test]
fn a_held_key_reads_the_commit_once_it_comes_up_or_settles(cx: &mut TestAppContext) {
    let g = Generated::shared(200);
    let (view, cx) = open(&g.dir, cx);
    click_row(&view, 0, cx);
    let token = |cx: &mut VisualTestContext| view.read_with(cx, |v, cx| v.panel.read(cx).token());
    let shown = |cx: &mut VisualTestContext| view.read_with(cx, |v, cx| v.probe(cx).commit.map(|c| c.0));
    let start = token(cx);
    keys(&view, "down down down down down", cx);
    assert_eq!(token(cx), start, "five rows moved and not one read");
    let fifth = view.read_with(cx, |v, _| v.rows[5].sha.to_string());
    assert_eq!(selected_sha(&view, cx), commit(&fifth), "the selection moved at once");
    key_up(&view, "down", cx);
    assert_eq!(token(cx), start + 1, "one read when the key comes up");
    assert_eq!(shown(cx).as_deref(), Some(fifth.as_str()));
    key_up(&view, "down", cx);
    assert_eq!(token(cx), start + 1, "a key up with nothing waiting reads nothing");

    keys(&view, "down down", cx);
    cx.executor().advance_clock(SETTLE / 2);
    cx.run_until_parked();
    assert_eq!(token(cx), start + 1, "not yet settled");
    cx.executor().advance_clock(SETTLE);
    cx.run_until_parked();
    assert_eq!(token(cx), start + 2, "read once it settles");
    let seventh = view.read_with(cx, |v, _| v.rows[7].sha.to_string());
    assert_eq!(shown(cx).as_deref(), Some(seventh.as_str()));
}

#[gpui::test]
fn right_opens_the_first_file_and_left_or_escape_go_back_to_the_row(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    let root = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.sha.as_ref() == f.sha("root")).unwrap());
    click_row(&view, root, cx);
    let files = view.read_with(cx, |v, cx| v.panel.read(cx).file_order(f.sha("root")));
    assert_eq!(paths(&files), ["a.txt", "b.txt", "bin.dat", "old.txt"]);
    keys(&view, "right", cx);
    assert_eq!(open_target(&view, cx).map(|t| t.path), Some("a.txt".into()), "Right: the first file's diff");
    let picked = view.read_with(cx, |v, cx| v.panel.read(cx).selected_file());
    assert_eq!(picked.as_deref(), Some("a.txt"), "and the file selected in the list");
    let d = view.read_with(cx, |v, _| v.diff().cloned()).unwrap();
    assert_eq!(d.read_with(cx, |d, _| d.current()), Some(0), "scrolled to its first change");
    assert!(cx.update(|window, cx| d.read(cx).focus_handle().is_focused(window)), "the viewer has the keyboard");
    keys(&view, "right", cx);
    assert_eq!(open_target(&view, cx).map(|t| t.path), Some("a.txt".into()), "Right in the viewer does nothing");

    keys(&view, "left", cx);
    assert!(open_target(&view, cx).is_none(), "Left closes the viewer");
    assert_eq!(selected_sha(&view, cx), commit(f.sha("root")));
    assert!(view.read_with(cx, |v, cx| v.panel.read(cx).selected_file().is_none()), "the file deselected");
    assert!(cx.update(|window, cx| view.read(cx).table.focus_handle().is_focused(window)), "the graph's row has the keyboard");
    keys(&view, "up right", cx);
    let above = view.read_with(cx, |v, _| v.rows[root - 1].sha.to_string());
    assert_eq!(selected_sha(&view, cx), commit(&above), "keys walk the graph again");
    key_up(&view, "up", cx);
    keys(&view, "right", cx);
    assert!(open_target(&view, cx).is_some(), "Right once the panel shows the row");
    keys(&view, "escape", cx);
    assert!(open_target(&view, cx).is_none(), "Escape closes it too");
}

#[gpui::test]
fn up_and_down_in_the_viewer_walk_the_files_and_wrap(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    let root = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.sha.as_ref() == f.sha("root")).unwrap());
    click_row(&view, root, cx);
    keys(&view, "right", cx);
    let path = |cx: &mut VisualTestContext| open_target(&view, cx).map(|t| t.path).unwrap_or_default();
    let mut seen = Vec::new();
    for key in ["down", "j", "down", "down", "up", "k", "ctrl-end", "ctrl-home", "ctrl-down", "ctrl-up"] {
        keys(&view, key, cx);
        seen.push(path(cx));
    }
    assert_eq!(seen, ["b.txt", "bin.dat", "old.txt", "a.txt", "old.txt", "bin.dat", "old.txt", "a.txt", "old.txt", "a.txt"], "down wraps from the last, up from the first");
    let picked = view.read_with(cx, |v, cx| v.panel.read(cx).selected_file());
    assert_eq!(picked.as_deref(), Some("a.txt"), "the list follows");
    let d = view.read_with(cx, |v, _| v.diff().cloned()).unwrap();
    assert_eq!(d.read_with(cx, |d, _| d.current()), Some(0), "each file opens at its first change");
}

#[gpui::test]
fn on_the_wip_row_right_opens_an_unstaged_file_first_and_the_walk_goes_on_into_the_staged(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    click_row(&view, 0, cx);
    let files = view.read_with(cx, |v, cx| v.wip_panel.read(cx).file_order());
    let listed: Vec<(&str, bool)> = files.iter().map(|t| (t.path.as_str(), t.of == Of::Staged)).collect();
    assert_eq!(listed, [("a.txt", false), ("b.txt", false), ("u.txt", false), ("a.txt", true), ("renamed.txt", true), ("s.txt", true)]);
    keys(&view, "right", cx);
    assert_eq!(open_target(&view, cx), Some(files[0].clone()));
    keys(&view, "ctrl-end", cx);
    keys(&view, "up", cx);
    assert_eq!(open_target(&view, cx), Some(files[4].clone()), "renamed.txt, staged");
    view.read_with(cx, |v, cx| {
        let p = v.wip_panel.read(cx);
        assert_eq!((p.selected_row(Section::Unstaged), p.selected_row(Section::Staged)), (None, Some(1)), "one row selected, in the staged list");
    });
    keys(&view, "down", cx);
    keys(&view, "down", cx);
    assert_eq!(open_target(&view, cx), Some(files[0].clone()), "from the last staged file to the first unstaged");
}

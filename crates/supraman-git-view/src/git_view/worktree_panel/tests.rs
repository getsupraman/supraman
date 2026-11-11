//! The panel of another worktree's WIP row through the real tab: its header and lists, a file's diff, nothing that writes.

use std::path::Path;

use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use supraman_core::paths::path_key;
use supraman_gitman::fixture::Fixture;
use supraman_gitman::{ChangeKind, FileChange};

use crate::git_view::background::refresh_wips;
use crate::git_view::branch::OpenGit;
use crate::git_view::diff::{Content, Of};
use crate::git_view::test_support::{self, Opts, redraw, settle};
use crate::git_view::wip::Section;
use crate::git_view::worktree_panel::list_id;
use crate::git_view::{GitView, Selected};
use crate::occupancy;

const SIDE_A: &str = "side's own a\n";

/// The fixture's worktree `wt` (on `side`) with `a.txt` changed, `s.txt` staged and `u.txt` untracked.
fn fixture() -> Fixture {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("a.txt"), SIDE_A).unwrap();
    std::fs::write(f.worktree.join("s.txt"), b"staged on side\n").unwrap();
    std::fs::create_dir_all(f.worktree.join("dir")).unwrap();
    std::fs::write(f.worktree.join("dir").join("u.txt"), b"untracked\n").unwrap();
    f.at(&f.worktree, &["add", "s.txt"]);
    f
}

fn open<'a>(f: &Fixture, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    let (view, cx) = test_support::open(&f.dir, Opts::default().live(occupancy::Live::default()).background().editor().size(1400., 900.), cx);
    reread(&view, cx);
    (view, cx)
}

fn reread(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let id = view.entity_id();
    cx.update(|_, cx| refresh_wips(id, cx));
    settle(view, cx);
}

fn select_worktree(view: &Entity<GitView>, dir: &Path, cx: &mut VisualTestContext) {
    let ix = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.worktree().is_some_and(|p| path_key(p) == path_key(dir)))).expect("a row of the worktree");
    view.update(cx, |v, cx| {
        v.table.select(Some(ix));
        v.selection_moved(cx);
    });
    settle(view, cx);
}

fn painted(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<String> {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let keys = hits.borrow().keys().cloned().collect();
    keys
}

fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    test_support::click(view, key, cx);
    settle(view, cx);
}

fn listed(files: &[FileChange]) -> Vec<(&str, ChangeKind)> {
    files.iter().map(|f| (f.path.as_str(), f.kind)).collect()
}

fn index_of(f: &Fixture, dir: &Path) -> String {
    f.at(dir, &["diff", "--cached", "--name-status"])
}

#[gpui::test]
fn worktree_panel_shows_the_header_and_the_worktrees_files_in_path_and_tree(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    select_worktree(&view, &f.worktree, cx);
    assert!(matches!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Worktree(_))));

    let shown = view.read_with(cx, |v, cx| v.worktree_panel.read(cx).shown()).expect("the worktree is shown");
    assert_eq!(path_key(&shown.path), path_key(&f.worktree));
    assert_eq!((shown.said.label.as_str(), shown.said.counts.as_str()), ("side", "+2 ✎1"));
    assert_eq!(listed(&shown.unstaged), [("a.txt", ChangeKind::Modified), ("dir/u.txt", ChangeKind::Untracked)], "the worktree's files, not the tab's");
    assert_eq!(listed(&shown.staged), [("s.txt", ChangeKind::Added)]);

    let keys = painted(&view, cx);
    for key in ["worktree-path", "worktree-branch", "worktree-counts", "worktree-open", "worktree-view-path", "worktree-view-tree", "worktree-unstaged-header", "worktree-staged-header"] {
        assert!(keys.iter().any(|k| k == key), "{key} paints");
    }
    for (section, path) in [(Section::Unstaged, "a.txt"), (Section::Unstaged, "dir/u.txt"), (Section::Staged, "s.txt")] {
        let row = format!("{}-files-row-{path}", list_id(section));
        assert!(keys.contains(&row), "{row} paints in Path");
    }
    assert!(!keys.iter().any(|k| k.starts_with("wip-")), "the tab's WIP panel is not shown: {keys:?}");

    click(&view, "worktree-view-tree", cx);
    let keys = painted(&view, cx);
    for (section, path) in [(Section::Unstaged, "a.txt"), (Section::Unstaged, "dir"), (Section::Unstaged, "dir/u.txt"), (Section::Staged, "s.txt")] {
        let row = format!("{}-tree-row-{path}", list_id(section));
        assert!(keys.contains(&row), "{row} paints in Tree");
    }

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let heard = seen.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &OpenGit, _| heard.borrow_mut().push(path_key(&e.0))).detach());
    click(&view, "worktree-open", cx);
    assert_eq!(*seen.borrow(), [path_key(&f.worktree)], "Open Worktree asks for its tab");
}

#[gpui::test]
fn worktree_panel_follows_a_new_read_and_clears_when_the_row_goes(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    select_worktree(&view, &f.worktree, cx);

    std::fs::write(f.worktree.join("n.txt"), b"new\n").unwrap();
    reread(&view, cx);
    let shown = view.read_with(cx, |v, cx| v.worktree_panel.read(cx).shown()).expect("shown");
    assert!(shown.unstaged.iter().any(|c| c.path == "n.txt"), "a new read updates the panel while the row stays selected");

    f.at(&f.worktree, &["reset", "-q", "--hard"]);
    f.at(&f.worktree, &["clean", "-q", "-fd"]);
    reread(&view, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), None, "a clean worktree's row goes, and its selection with it");
}

#[gpui::test]
fn worktree_file_opens_its_diff_from_that_worktree_and_nothing_writes(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    select_worktree(&view, &f.worktree, cx);
    let index_before = index_of(&f, &f.worktree);

    click(&view, &format!("{}-files-row-a.txt", list_id(Section::Unstaged)), cx);
    let d = view.read_with(cx, |v, cx| v.diff(cx).cloned()).expect("the viewer is open");
    let (of, worktree, new, old) = d.read_with(cx, |d, cx| match d.content() {
        Content::Text(t) => (d.target.of.clone(), d.worktree.clone(), t.diff.new.read(cx).text(), t.diff.old.read(cx).text()),
        _ => panic!("a.txt is not a text diff"),
    });
    assert_eq!(of, Of::Unstaged);
    assert_eq!(worktree.map(|w| path_key(&w)), Some(path_key(&f.worktree)));
    assert_eq!(new, SIDE_A, "the other worktree's file");
    let side_index = f.at(&f.worktree, &["cat-file", "blob", ":a.txt"]);
    assert_eq!(old.trim_end(), side_index.trim_end(), "against that worktree's index");
    assert_ne!(new, std::fs::read_to_string(f.dir.join("a.txt")).unwrap(), "not the tab's own a.txt");

    let keys = painted(&view, cx);
    let writes: Vec<&String> = keys.iter().filter(|k| k.starts_with("diff-hunk-") || k.starts_with("diff-line-") || k.starts_with("wip-")).collect();
    assert!(writes.is_empty(), "no hunk, line or WIP button: {writes:?}");

    let at = view.read_with(cx, |v, _| v.hits.borrow().get(&format!("{}-files-row-a.txt", list_id(Section::Unstaged))).map(|h| h.bounds.center())).expect("a.txt's row is painted");
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    settle(&view, cx);
    assert!(!painted(&view, cx).iter().any(|k| k.starts_with("menu")), "no file menu");

    for keys in ["s", "u"] {
        cx.simulate_keystrokes(keys);
        settle(&view, cx);
    }
    assert_eq!(index_of(&f, &f.worktree), index_before, "no key stages or unstages in the other worktree");
    assert_eq!(std::fs::read_to_string(f.worktree.join("a.txt")).unwrap(), SIDE_A);

    cx.simulate_keystrokes("down");
    settle(&view, cx);
    let stepped = view.read_with(cx, |v, cx| v.diff(cx).map(|d| (d.read(cx).target.path.clone(), d.read(cx).worktree.as_deref().map(path_key))));
    assert_eq!(stepped, Some(("dir/u.txt".to_string(), Some(path_key(&f.worktree)))), "Down opens the worktree's next file");

    click(&view, &format!("{}-files-row-s.txt", list_id(Section::Staged)), cx);
    let (of, new) = view.read_with(cx, |v, cx| v.diff(cx).cloned()).expect("open").read_with(cx, |d, cx| match d.content() {
        Content::Text(t) => (d.target.of.clone(), t.diff.new.read(cx).text()),
        _ => panic!("s.txt is not a text diff"),
    });
    assert_eq!((of, new.as_str()), (Of::Staged, "staged on side\n"), "a staged file from that worktree's index");
}

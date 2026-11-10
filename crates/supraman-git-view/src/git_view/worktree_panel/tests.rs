//! The panel of another worktree's WIP row through the real tab, on a fixture copy whose linked worktree is in its
//! temp dir (C196): its header and lists, a file's diff read from that worktree, and nothing that writes.

use std::path::Path;
use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Modifiers, MouseButton, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::{BackgroundSettings, ChangeKind, FileChange};
use supraman_ui::hit::HitTargets;

use crate::git_view::background::{self, refresh_wips};
use crate::git_view::branch::OpenGit;
use crate::git_view::diff::{Content, Of};
use crate::git_view::tests::redraw;
use crate::git_view::wip::Section;
use crate::git_view::worktree_panel::list_id;
use crate::git_view::{GitSizes, GitView, Selected, bind_keys};
use crate::occupancy;

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1400.)).h(px(900.)).child(self.0.clone())
    }
}

const QUIET: BackgroundSettings = BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), worktree_wip: None };
const SIDE_A: &str = "side's own a\n";

/// The fixture's worktree `wt` (on `side`) with `a.txt` changed, `s.txt` staged and `u.txt` untracked.
fn fixture() -> Fixture {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("a.txt"), SIDE_A).unwrap();
    std::fs::write(f.worktree.join("s.txt"), b"staged on side\n").unwrap();
    std::fs::create_dir_all(f.worktree.join("dir")).unwrap();
    std::fs::write(f.worktree.join("dir").join("u.txt"), b"untracked\n").unwrap();
    git_in(&f.git, &f.worktree, &["add", "s.txt"], &[], None);
    f
}

fn open<'a>(f: &Fixture, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        supraman_editor::init(supraman_ui::fonts::MONO, 13., cx);
        bind_keys(cx);
        background::start_for_test(QUIET, cx);
    });
    let folder = f.dir.clone();
    let live: Arc<dyn supraman_gitman::write::Occupancy> = Arc::new(occupancy::Live::default());
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open_with(folder, None, HitTargets::default(), &GitSizes::default(), live, cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    crate::git_view::test_support::assert_loaded(&view, cx);
    reread(&view, cx);
    (view, cx)
}

fn reread(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let id = view.entity_id();
    cx.update(|_, cx| refresh_wips(id, cx));
    settle(view, cx);
}

fn settle(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        redraw(view, cx);
    }
}

fn select_worktree(view: &Entity<GitView>, dir: &Path, cx: &mut VisualTestContext) {
    let ix = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.worktree().is_some_and(|p| path_key(p) == path_key(dir)))).expect("a row of the worktree");
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
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let at = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    cx.simulate_click(at, Modifiers::none());
    settle(view, cx);
}

fn listed(files: &[FileChange]) -> Vec<(&str, ChangeKind)> {
    files.iter().map(|f| (f.path.as_str(), f.kind)).collect()
}

fn index_of(f: &Fixture, dir: &Path) -> String {
    git_in(&f.git, dir, &["diff", "--cached", "--name-status"], &[], None)
}

#[gpui::test]
fn worktree_panel_shows_the_header_and_the_worktrees_files_in_path_and_tree(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    select_worktree(&view, &f.worktree, cx);
    assert!(matches!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Worktree(_))));

    let shown = view.read_with(cx, |v, cx| v.worktree_panel.read(cx).shown()).expect("the worktree is shown");
    assert_eq!(path_key(&shown.path), path_key(&f.worktree));
    assert_eq!((shown.said.label.as_str(), shown.said.counts.as_str()), ("on side", "+2 ✎1"));
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

    git_in(&f.git, &f.worktree, &["reset", "-q", "--hard"], &[], None);
    git_in(&f.git, &f.worktree, &["clean", "-q", "-fd"], &[], None);
    reread(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), None, "a clean worktree's row goes, and its selection with it");
}

#[gpui::test]
fn worktree_file_opens_its_diff_from_that_worktree_and_nothing_writes(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    select_worktree(&view, &f.worktree, cx);
    let index_before = index_of(&f, &f.worktree);

    click(&view, &format!("{}-files-row-a.txt", list_id(Section::Unstaged)), cx);
    let d = view.read_with(cx, |v, _| v.diff().cloned()).expect("the viewer is open");
    let (of, worktree, new, old) = d.read_with(cx, |d, cx| match d.content() {
        Content::Text(t) => (d.target.of.clone(), d.worktree.clone(), t.diff.new.read(cx).text(), t.diff.old.read(cx).text()),
        _ => panic!("a.txt is not a text diff"),
    });
    assert_eq!(of, Of::Unstaged);
    assert_eq!(worktree.map(|w| path_key(&w)), Some(path_key(&f.worktree)));
    assert_eq!(new, SIDE_A, "the other worktree's file");
    let side_index = git_in(&f.git, &f.worktree, &["cat-file", "blob", ":a.txt"], &[], None);
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
    let stepped = view.read_with(cx, |v, cx| v.diff().map(|d| (d.read(cx).target.path.clone(), d.read(cx).worktree.as_deref().map(path_key))));
    assert_eq!(stepped, Some(("dir/u.txt".to_string(), Some(path_key(&f.worktree)))), "Down opens the worktree's next file");

    click(&view, &format!("{}-files-row-s.txt", list_id(Section::Staged)), cx);
    let (of, new) = view.read_with(cx, |v, _| v.diff().cloned()).expect("open").read_with(cx, |d, cx| match d.content() {
        Content::Text(t) => (d.target.of.clone(), t.diff.new.read(cx).text()),
        _ => panic!("s.txt is not a text diff"),
    });
    assert_eq!((of, new.as_str()), (Of::Staged, "staged on side\n"), "a staged file from that worktree's index");
}

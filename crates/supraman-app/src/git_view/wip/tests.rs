use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::{ChangeKind, FileChange};
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;

use super::*;
use crate::git_view::details::mark_icon;
use crate::git_view::diff::{Content, DiffView, Mode, Of};
use crate::git_view::tests::{open, redraw};
use crate::git_view::{GitView, Selected};

pub(crate) fn open_with_editor<'a>(folder: &std::path::Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_editor::init(supraman_ui::fonts::MONO, 13., cx);
        crate::git_view::diff::bind_keys(cx);
    });
    open(folder, cx)
}

pub(crate) fn select_wip(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.select_wip(cx));
    cx.run_until_parked();
    redraw(view, cx);
}

fn shown(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Rc<Shown> {
    view.read_with(cx, |v, cx| v.wip_panel().read(cx).shown().expect("the working directory is shown"))
}

fn listed(files: &[FileChange]) -> Vec<(&str, ChangeKind)> {
    files.iter().map(|f| (f.path.as_str(), f.kind)).collect()
}

/// Hit keys of the next frame only.
pub(crate) fn repaint(view: &Entity<GitView>, cx: &mut VisualTestContext) -> HitTargets {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    hits
}

pub(crate) fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    let at = repaint(view, cx).borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
    redraw(view, cx);
}

/// Clicks `path`'s row in `section` and reads the diff it opens: its target and its two sides.
pub(crate) fn open_file(view: &Entity<GitView>, section: Section, path: &str, cx: &mut VisualTestContext) -> (Entity<DiffView>, String, String) {
    click(view, &format!("{}-files-row-{path}", section.list()), cx);
    let d = view.read_with(cx, |v, _| v.diff().cloned()).expect("the viewer is open");
    let (old, new) = d.read_with(cx, |d, cx| match d.content() {
        Content::Text(t) => (t.diff.old.read(cx).text(), t.diff.new.read(cx).text()),
        _ => panic!("{path} is not a text diff"),
    });
    (d, old, new)
}

#[gpui::test]
fn wip_view_lists_each_file_in_its_section_with_its_mark(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    select_wip(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Wip));

    let s = shown(&view, cx);
    assert_eq!(listed(&s.unstaged), [("a.txt", ChangeKind::Modified), ("b.txt", ChangeKind::Deleted), ("u.txt", ChangeKind::Untracked)]);
    assert_eq!(listed(&s.staged), [("a.txt", ChangeKind::Modified), ("renamed.txt", ChangeKind::Renamed), ("s.txt", ChangeKind::Added)]);
    assert_eq!(s.staged[1].old_path.as_deref(), Some("new.txt"));
    assert_eq!((s.total, &s.branch), (5, &Branch::Named("main".into())), "a.txt counts once though in both lists");
    assert_eq!(WipPanel::header_words(&s).as_ref(), "5 file changes on");

    let painted = repaint(&view, cx);
    for (section, path) in [(Section::Unstaged, "a.txt"), (Section::Unstaged, "b.txt"), (Section::Unstaged, "u.txt"), (Section::Staged, "a.txt"), (Section::Staged, "renamed.txt"), (Section::Staged, "s.txt")] {
        let list = section.list();
        assert!(painted.borrow().contains_key(&format!("{list}-files-row-{path}")) && painted.borrow().contains_key(&format!("{list}-mark-{path}")), "{path} is a row of {list} with its mark");
    }
    for key in ["wip-sort", "wip-view-path", "wip-view-tree", "wip-unstaged-header", "wip-staged-header"] {
        assert!(painted.borrow().contains_key(key), "{key} paints");
    }
    assert!(!painted.borrow().contains_key("commit-sort") && !painted.borrow().contains_key("wip-banner-view-change"), "the WIP panel in the commit panel's place, no banner");
    cx.update(|_, cx| {
        let status = theme::ActiveTheme::theme(cx).status().clone();
        assert_eq!(mark_icon(ChangeKind::Untracked, cx), (Icon::Untracked, status.created));
        assert_eq!(mark_icon(ChangeKind::Unmerged, cx), (Icon::Conflict, status.conflict));
        assert_eq!(mark_icon(ChangeKind::Deleted, cx), (Icon::Minus, status.deleted));
    });

    view.update(cx, |v, cx| v.wip_panel().update(cx, |p, cx| p.set_view(FileView::Tree, cx)));
    let painted = repaint(&view, cx);
    assert!(painted.borrow().contains_key("wip-unstaged-tree-row-u.txt") && painted.borrow().contains_key("wip-staged-tree-row-renamed.txt"));
    assert!(painted.borrow().contains_key("wip-staged-mark-a.txt") && painted.borrow().contains_key("wip-unstaged-mark-a.txt"), "the partly staged file in both trees");

    click(&view, "wip-unstaged-header", cx);
    let painted = repaint(&view, cx);
    assert!(!painted.borrow().contains_key("wip-unstaged-tree-row-u.txt"), "Unstaged Files folds");
    assert!(painted.borrow().contains_key("wip-staged-tree-row-s.txt"), "Staged Files stays open");
}

#[gpui::test]
fn wip_view_diffs_show_the_right_sides(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open_with_editor(&f.dir, cx);
    select_wip(&view, cx);

    let (d, old, new) = open_file(&view, Section::Unstaged, "a.txt", cx);
    assert_eq!((old.as_str(), new.as_str()), ("staged\n", "staged\nunstaged\n"), "unstaged: the index against the worktree");
    assert_eq!(d.read_with(cx, |d, _| (d.target.of.clone(), d.target.side_word())), (Of::Unstaged, Some("Unstaged")));
    let (d, old, new) = open_file(&view, Section::Staged, "a.txt", cx);
    assert_eq!((old.as_str(), new.as_str()), ("one\ntwo\nthree\n", "staged\n"), "staged: HEAD against the index");
    assert_eq!(d.read_with(cx, |d, _| d.target.side_word()), Some("Staged"));
    let (d, old, new) = open_file(&view, Section::Unstaged, "u.txt", cx);
    assert_eq!((old.as_str(), new.as_str()), ("", "untracked\n"), "untracked: nothing against the worktree");
    assert_eq!(d.read_with(cx, |d, _| d.target.of.clone()), Of::Untracked);
    let (_, old, new) = open_file(&view, Section::Staged, "s.txt", cx);
    assert_eq!((old.as_str(), new.as_str()), ("", "new\n"));
    let (_, old, new) = open_file(&view, Section::Staged, "renamed.txt", cx);
    assert_eq!((old.as_str(), new.as_str()), ("rename me\nline 2\nline 3\nline 4\n", "rename me\nline 2\nline 3\nline 4\n"), "a staged rename read at its old path in HEAD");
    let (_, old, new) = open_file(&view, Section::Unstaged, "b.txt", cx);
    assert_eq!((old.as_str(), new.as_str()), ("b\nb2\n", ""), "a deletion in the worktree");
    let painted = repaint(&view, cx);
    assert!(painted.borrow().contains_key("diff-close"), "the viewer in the graph's place");
}

#[gpui::test]
fn wip_view_shows_a_conflicted_file_in_file_view(cx: &mut TestAppContext) {
    let f = Fixture::new();
    f.conflict();
    let (view, cx) = open_with_editor(&f.worktree, cx);
    select_wip(&view, cx);
    let s = shown(&view, cx);
    assert_eq!(listed(&s.unstaged), [("a.txt", ChangeKind::Unmerged)]);
    assert!(!s.staged.is_empty() && s.staged.iter().all(|f| f.path != "a.txt"), "the merge's clean files are staged, the conflicted one is not: {:?}", s.staged);
    assert_eq!(s.branch, Branch::Named("side".into()));

    let (d, old, new) = open_file(&view, Section::Unstaged, "a.txt", cx);
    assert!(new.contains("<<<<<<<") && new.contains(">>>>>>>") && old == new, "the worktree file with its markers: {new:?}");
    assert_eq!(d.read_with(cx, |d, _| (d.mode(), d.target.of.clone())), (Mode::File, Of::Conflicted));
}

#[gpui::test]
fn wip_view_banner_view_change_selects_the_wip_row(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, vcx) = open(&f.worktree, cx);
    view.update(vcx, |v, cx| v.select_sha(f.sha("second"), cx));
    vcx.run_until_parked();
    let painted = repaint(&view, vcx);
    assert!(painted.borrow().contains_key("commit-sort"), "a commit is shown");
    assert!(!painted.borrow().contains_key("wip-banner-view-change"), "no banner on a clean tree");

    let (view, cx) = open(&f.dir, cx);
    view.update(cx, |v, cx| v.select_sha(f.sha("ahead"), cx));
    cx.run_until_parked();
    assert!(repaint(&view, cx).borrow().contains_key("wip-banner-view-change"), "the banner over a commit of a dirty tree");
    click(&view, "wip-banner-view-change", cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Wip), "View Change selects the WIP row");
    assert_eq!(shown(&view, cx).total, 5);
    assert!(!repaint(&view, cx).borrow().contains_key("wip-banner-view-change"), "no banner over the WIP panel");
    assert_eq!(file_changes(1), "1 file change");
}

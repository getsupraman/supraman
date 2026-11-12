//! One Path | Tree for every file list of every Git tab, kept in the layout file.

use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;

use super::*;
use crate::git_view::details::FileView;
use crate::git_view::test_support::{Opts, click, open, painted, select_wip};

fn select_second(view: &Entity<GitView>, f: &Fixture, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.select_sha(f.sha("second"), cx));
    cx.run_until_parked();
}

fn views(view: &Entity<GitView>, cx: &mut VisualTestContext) -> (FileView, FileView, FileView) {
    view.read_with(cx, |v, cx| (v.wip_panel.read(cx).view(), v.panel.read(cx).view(), v.worktree_panel.read(cx).view()))
}

fn kept(home: &std::path::Path) -> FileView {
    let (file, notice) = layout::load(Some(home));
    assert!(notice.is_none());
    file.get().file_view
}

#[gpui::test]
fn path_or_tree_chosen_in_one_panel_is_every_panels_and_every_later_tabs(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let home = tempfile::tempdir().unwrap();
    let (view, vcx) = open(&f.dir, Opts::default().home(home.path()), cx);
    assert_eq!(views(&view, vcx), (FileView::Path, FileView::Path, FileView::Path), "Path by default");

    select_wip(&view, vcx);
    click(&view, "wip-view-tree", vcx);
    assert_eq!(views(&view, vcx), (FileView::Tree, FileView::Tree, FileView::Tree), "the WIP panel's Tree is every list's");
    select_second(&view, &f, vcx);
    assert!(painted(&view, "commit-tree-row-a.txt", vcx) && !painted(&view, "commit-files-row-a.txt", vcx), "the commit's files as a tree");
    assert_eq!(kept(home.path()), FileView::Tree, "the layout file holds it");

    click(&view, "commit-view-path", vcx);
    assert_eq!(views(&view, vcx), (FileView::Path, FileView::Path, FileView::Path), "the commit panel's Path is every list's");
    select_wip(&view, vcx);
    assert!(painted(&view, "wip-unstaged-files-row-a.txt", vcx) && !painted(&view, "wip-unstaged-tree-row-a.txt", vcx), "the WIP files as paths");
    assert_eq!(kept(home.path()), FileView::Path);

    click(&view, "wip-view-tree", vcx);
    drop(view);
    let (again, vcx) = open(&f.dir, Opts::default().home(home.path()), cx);
    assert_eq!(views(&again, vcx), (FileView::Tree, FileView::Tree, FileView::Tree), "a tab opened afterwards starts with it");
    select_wip(&again, vcx);
    assert!(painted(&again, "wip-unstaged-tree-row-a.txt", vcx));
}

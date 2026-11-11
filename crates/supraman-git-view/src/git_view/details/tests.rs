use std::rc::Rc;

use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_ui::tree::Order;

use super::*;
use crate::git_view::GitView;
use crate::git_view::test_support::{Opts, open, redraw, repaint};
use crate::git_view::test_support::click;

fn panel_of(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Entity<CommitPanel> {
    view.read_with(cx, |v, _| v.panel().clone())
}

fn shown(panel: &Entity<CommitPanel>, cx: &mut VisualTestContext) -> Rc<Shown> {
    panel.read_with(cx, |p, _| p.shown().expect("a commit is shown"))
}

fn select(view: &Entity<GitView>, sha: &str, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.select_sha(sha, cx));
    cx.run_until_parked();
    redraw(view, cx);
}

fn hits(view: &Entity<GitView>, cx: &mut VisualTestContext) -> supraman_ui::hit::HitTargets {
    view.read_with(cx, |v, _| v.hits.clone())
}

fn names(people: &[supraman_gitman::Person]) -> Vec<&str> {
    people.iter().map(|p| p.name.as_str()).collect()
}

#[gpui::test]
fn commit_details_show_a_commit_a_merge_and_the_root(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);

    select(&view, f.sha("rename"), cx);
    let s = shown(&panel, cx);
    assert_eq!((s.sha.as_ref(), s.summary.as_ref(), s.author.as_ref()), (f.sha("rename"), "Move old.txt to new.txt", "Ada Fixture"));
    assert!(s.body.starts_with("The old name no longer fits."), "{}", s.body);
    assert_eq!(s.parents, [SharedString::from(f.sha("second").to_string())]);
    assert_eq!(names(&s.co_authors), ["Bea Two", "Cy Three"]);
    assert_eq!(s.counts, WipCounts { renamed: 1, ..WipCounts::default() });
    assert_eq!(s.authored_ms, 1_700_000_000_000 + 3 * 60_000, "the third commit's time");
    let authored = render::authored(&s);
    assert_eq!(authored, supraman_core::time::stamp(s.authored_ms), "the author's date reads as the graph's date column");
    assert!(authored.contains(" @ "), "{authored}");
    let painted = repaint(&view, cx);
    for key in ["commit-sha", "commit-message-copy", "commit-parent-0", "commit-coauthor-0", "commit-coauthor-1"] {
        assert!(painted.borrow().contains_key(key), "{key} paints");
    }
    assert!(!painted.borrow().contains_key("commit-parent-1"));

    select(&view, f.sha("merge"), cx);
    let s = shown(&panel, cx);
    assert_eq!(s.summary.as_ref(), "Merge feature");
    assert_eq!(s.parents.iter().map(|p| p.as_ref()).collect::<Vec<_>>(), [f.sha("main_work"), f.sha("rename")]);
    assert!(s.co_authors.is_empty());
    assert_eq!((s.counts, s.changes[0].path.as_str()), (WipCounts { renamed: 1, ..WipCounts::default() }, "new.txt"), "against its first parent");
    let painted = repaint(&view, cx);
    assert!(painted.borrow().contains_key("commit-parent-1") && !painted.borrow().contains_key("commit-coauthor-0"));
    let second = painted.borrow()["commit-parent-1"].bounds.center();
    cx.simulate_click(second, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Commit(f.sha("rename").to_string().into())), "the second parent's click selects it");
    select(&view, f.sha("merge"), cx);
    let painted = repaint(&view, cx);

    let parent = painted.borrow()["commit-parent-0"].bounds.center();
    cx.simulate_click(parent, Modifiers::none());
    cx.run_until_parked();
    let main_work = Selected::Commit(f.sha("main_work").to_string().into());
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(main_work), "a parent's click selects it in the graph");
    assert_eq!(shown(&panel, cx).summary.as_ref(), "Main work");

    select(&view, f.sha("root"), cx);
    let s = shown(&panel, cx);
    assert_eq!((s.summary.as_ref(), s.body.as_ref(), s.parents.len()), ("Root", "", 0));
    assert_eq!(s.counts, WipCounts { added: 4, ..WipCounts::default() });
    assert!(!repaint(&view, cx).borrow().contains_key("commit-parent-0"), "the root has no parent");

    view.update(cx, |v, cx| {
        v.table.select(Some(0));
        v.selection_moved(cx);
    });
    cx.run_until_parked();
    assert!(panel.read_with(cx, |p, _| matches!(p.state(), State::Empty)), "the WIP row leaves the commit panel to the WIP panel");
}

/// A commit on `side` (the fixture's worktree) adding files in folders, changing `a.txt` and deleting `b.txt`.
fn nested_commit(f: &Fixture) -> String {
    for (path, text) in [("src/app/main.rs", "fn main() {}\n"), ("src/app/lib.rs", "\n"), ("src/util.rs", "\n"), ("docs/guide.md", "# Guide\n"), ("a.txt", "side\n")] {
        let file = f.worktree.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    std::fs::remove_file(f.worktree.join("b.txt")).unwrap();
    let date = "@1700100000 +0000".to_string();
    let env = [("GIT_AUTHOR_DATE", date.clone()), ("GIT_COMMITTER_DATE", date)];
    f.at(&f.worktree, &["add", "-A"]);
    git_in(&f.git, &f.worktree, &["commit", "-q", "--no-verify", "-m", "Folders"], &env, None);
    f.at(&f.worktree, &["rev-parse", "HEAD"])
}

/// A test's own copy of the fixture with [`nested_commit`].
fn nested() -> (Fixture, String) {
    let f = Fixture::copy();
    let sha = nested_commit(&f);
    (f, sha)
}

fn visible(panel: &Entity<CommitPanel>, cx: &mut VisualTestContext) -> Vec<String> {
    panel.read_with(cx, |p, _| p.tree().0.visible().iter().map(|s| s.to_string()).collect())
}

#[gpui::test]
fn commit_details_list_path_tree_and_all_files_with_marks_badges_and_the_filter(cx: &mut TestAppContext) {
    let nested = nested();
    let (f, sha) = (&nested.0, nested.1.clone());
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    select(&view, &sha, cx);
    let s = shown(&panel, cx);
    assert_eq!(s.counts, WipCounts { modified: 1, added: 4, deleted: 1, ..WipCounts::default() });

    panel.update(cx, |p, cx| p.set_view(FileView::Path, cx));
    let painted = repaint(&view, cx);
    let changed = ["a.txt", "b.txt", "docs/guide.md", "src/app/lib.rs", "src/app/main.rs", "src/util.rs"];
    assert_eq!(s.changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), changed);
    for path in changed {
        assert!(painted.borrow().contains_key(&format!("commit-files-row-{path}")) && painted.borrow().contains_key(&format!("commit-mark-{path}")), "{path} is a row with its mark");
    }
    panel.update(cx, |p, cx| p.set_order(Order::Descending, cx));
    let reversed: Vec<&str> = changed.iter().rev().copied().collect();
    assert_eq!(shown(&panel, cx).changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), reversed, "Z–A");
    panel.update(cx, |p, cx| p.set_order(Order::Ascending, cx));

    panel.update(cx, |p, cx| p.set_view(FileView::Tree, cx));
    redraw(&view, cx);
    assert_eq!(visible(&panel, cx), ["docs", "docs/guide.md", "src", "src/app", "src/app/lib.rs", "src/app/main.rs", "src/util.rs", "a.txt", "b.txt"]);
    let badge = |path: &str, cx: &mut VisualTestContext| panel.read_with(cx, |p, _| p.tree().0.aggregate(path));
    assert_eq!(badge("src", cx), Some(WipCounts { added: 3, ..WipCounts::default() }), "a folder counts its whole subtree");
    assert_eq!(badge("src/app", cx), Some(WipCounts { added: 2, ..WipCounts::default() }));
    let painted = repaint(&view, cx);
    for folder in ["docs", "src", "src/app"] {
        assert!(!painted.borrow().contains_key(&format!("commit-badges-{folder}")), "open {folder} shows no counters");
    }
    panel.update(cx, |p, cx| p.expand_or_collapse(cx));
    assert_eq!(visible(&panel, cx), ["docs", "src", "a.txt", "b.txt"], "Collapse All");
    let painted = repaint(&view, cx);
    for folder in ["docs", "src"] {
        assert!(painted.borrow().contains_key(&format!("commit-badges-{folder}")), "closed {folder} shows its counters");
    }

    panel.update(cx, |p, cx| p.set_all_files(true, cx));
    panel.update(cx, |p, cx| p.expand_or_collapse(cx));
    let painted = repaint(&view, cx);
    let all = ["docs", "docs/guide.md", "src", "src/app", "src/app/lib.rs", "src/app/main.rs", "src/util.rs", "a.txt", "b.txt", "bin.dat", "old.txt"];
    assert_eq!(visible(&panel, cx), all, "the whole tree, the deleted file kept");
    assert!(!painted.borrow().contains_key("commit-view-path"), "no Path toggle with View all files");
    for path in changed {
        assert!(painted.borrow().contains_key(&format!("commit-mark-{path}")), "{path} is marked");
    }
    for path in ["bin.dat", "old.txt"] {
        assert!(painted.borrow().contains_key(&format!("commit-tree-row-{path}")) && !painted.borrow().contains_key(&format!("commit-mark-{path}")), "unchanged {path} has no mark");
    }
    assert_eq!(badge("src", cx), Some(WipCounts { added: 3, ..WipCounts::default() }));
    let x = |path: &str| painted.borrow().get(&format!("commit-name-{path}")).map(|h| h.bounds.left()).unwrap_or_else(|| panic!("{path}'s name paints"));
    for path in ["src", "a.txt", "bin.dat"] {
        assert_eq!(x(path), x("docs"), "{path}'s name at the top-level folders' names, marked or not");
    }
    assert_eq!(x("src/util.rs"), x("src/app"), "a file's name at its sibling folder's");

    panel.update(cx, |p, cx| p.set_filter("APP", cx));
    assert_eq!(visible(&panel, cx), ["src", "src/app", "src/app/lib.rs", "src/app/main.rs"], "the filter matches the whole path, ignoring case");
    panel.update(cx, |p, cx| p.set_filter("", cx));

    let bar = repaint(&view, cx).borrow()["commit-message-bar"].bounds.center();
    let before = panel.read_with(cx, |p, _| p.message_height());
    cx.simulate_mouse_down(bar, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(bar + point(px(0.), px(20.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(bar + point(px(0.), px(40.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(bar + point(px(0.), px(40.)), MouseButton::Left, Modifiers::none());
    let after = panel.read_with(cx, |p, _| p.message_height());
    assert!(after > before + px(35.) && after < before + px(45.), "the bar drags the box taller: {before:?} to {after:?}");
    let kept = cx.update(|_, cx| view.read(cx).sizes(cx).message);
    assert_eq!(kept, Some(f32::from(after).round() as u32), "the tab keeps the height");
}

#[gpui::test]
fn commit_details_toolbar_clicks_switch_the_list_and_the_filter_takes_typing(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    select(&view, f.sha("ahead"), cx);
    for (key, shows) in [("commit-view-path", "commit-files-row-a.txt"), ("commit-view-tree", "commit-tree-row-a.txt"), ("commit-all-files", "commit-tree-row-bin.dat"), ("commit-filter", "commit-filter")] {
        click(&view, key, cx);
        assert!(repaint(&view, cx).borrow().contains_key(shows), "after {key}, {shows} paints");
    }
    cx.simulate_input("new");
    let painted = repaint(&view, cx);
    assert!(painted.borrow().contains_key("commit-tree-row-new.txt") && !painted.borrow().contains_key("commit-tree-row-a.txt"), "the typed filter leaves new.txt");
    click(&view, "commit-all-files", cx);
    let painted = repaint(&view, cx);
    assert!(painted.borrow().contains_key("commit-tree-row-a.txt") && !painted.borrow().contains_key("commit-filter"), "the changed files back, the filter gone");
}

#[gpui::test]
fn commit_details_drop_a_load_that_finishes_after_a_newer_selection(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    let earlier = panel.update(cx, |p, cx| p.show(Some(Selected::Commit(f.sha("root").to_string().into())), 0, cx));
    panel.update(cx, |p, cx| p.show(Some(Selected::Commit(f.sha("second").to_string().into())), 0, cx));
    cx.run_until_parked();
    assert_eq!(shown(&panel, cx).summary.as_ref(), "Second");

    let late = load::load(&f.repo(), f.sha("root"), Order::Ascending);
    assert!(late.is_ok());
    panel.update(cx, |p, cx| p.apply(earlier, late, cx));
    let s = shown(&panel, cx);
    assert_eq!((s.sha.as_ref(), s.summary.as_ref()), (f.sha("second"), "Second"), "the earlier selection's late result is dropped");
}

#[gpui::test]
fn commit_details_a_second_click_on_the_open_file_closes_its_diff(cx: &mut TestAppContext) {
    let nested = nested();
    let (f, sha) = (&nested.0, nested.1.clone());
    let (view, cx) = crate::git_view::test_support::open(&f.dir, crate::git_view::test_support::Opts::default().editor(), cx);
    let panel = panel_of(&view, cx);
    select(&view, &sha, cx);
    let open_path = |cx: &mut VisualTestContext| view.read_with(cx, |v, cx| v.diff(cx).map(|d| d.read(cx).target.path.clone()));
    for (list, row) in [(FileView::Path, "commit-files-row"), (FileView::Tree, "commit-tree-row")] {
        panel.update(cx, |p, cx| p.set_view(list, cx));
        click(&view, &format!("{row}-a.txt"), cx);
        assert_eq!(open_path(cx).as_deref(), Some("a.txt"), "{list:?}: a click opens the file");
        click(&view, &format!("{row}-src/util.rs"), cx);
        assert_eq!(open_path(cx).as_deref(), Some("src/util.rs"), "{list:?}: another file switches the viewer to it");
        click(&view, &format!("{row}-src/util.rs"), cx);
        assert_eq!(open_path(cx), None, "{list:?}: a second click on the open file closes the viewer");
        assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Commit(sha.clone().into())), "the commit stays selected");
        assert!(repaint(&view, cx).borrow().contains_key("git-col-sha"), "the graph is back");
        assert_eq!(panel.read_with(cx, |p, _| (p.paths.selected(), p.tree().0.selected())), (None, None), "the file is deselected");
        assert_eq!(shown(&panel, cx).sha.as_ref(), sha, "the panel keeps the commit");
    }
    select(&view, f.sha("second"), cx);
    assert_eq!(shown(&panel, cx).summary.as_ref(), "Second", "the next commit selects as usual");
}

#[gpui::test]
fn commit_details_show_a_stash_its_branch_time_and_files_against_its_base(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    select(&view, f.sha("stash_feature_index"), cx);
    let Some(Selected::Stash(stash)) = view.read_with(cx, |v, cx| v.selected(cx)) else { panic!("the stash row selects its stash") };
    assert_eq!(stash.sha, f.sha("stash_feature"));
    let s = shown(&panel, cx);
    assert_eq!((s.sha.as_ref(), s.summary.as_ref(), s.body.as_ref()), (f.sha("stash_feature"), supraman_gitman::fixture::FEATURE_STASH, ""));
    assert_eq!(s.stash.as_ref().map(|s| s.branch.as_str()), Some("feature"));
    assert_eq!(s.parents, [SharedString::from(f.sha("rename").to_string())], "only the base, not the index commit");
    assert_eq!(s.authored_ms, stash.time * 1000);
    assert_eq!((s.counts, s.changes[0].path.as_str()), (WipCounts { modified: 1, ..WipCounts::default() }, "new.txt"));
    let painted = repaint(&view, cx);
    assert!(painted.borrow().contains_key("commit-sha") && painted.borrow().contains_key("commit-parent-0"));
    assert!(!painted.borrow().contains_key("commit-parent-1"));

    let parent = painted.borrow()["commit-parent-0"].bounds.center();
    cx.simulate_click(parent, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Commit(f.sha("rename").to_string().into())), "the base's click selects it");
}

/// Whether `key` painted wider than `min`.
fn wide(hits: &supraman_ui::hit::HitTargets, key: &str, min: f32) -> bool {
    hits.borrow().get(key).is_some_and(|h| h.bounds.size.width > px(min))
}

/// The hit keys of the one frame `act` leads to, drawn here if the update drew none.
pub(crate) fn first_frame(view: &Entity<GitView>, cx: &mut VisualTestContext, act: impl FnOnce(&mut VisualTestContext)) -> supraman_ui::hit::HitTargets {
    let hits = hits(view, cx);
    hits.borrow_mut().clear();
    act(cx);
    if hits.borrow().is_empty() {
        cx.update(|window, cx| {
            let drawn = window.draw(cx);
            drawn.clear(cx);
        });
    }
    hits
}

#[gpui::test]
fn commit_details_paint_a_list_whole_in_the_first_frame_it_shows(cx: &mut TestAppContext) {
    let nested = nested();
    let (f, sha) = (&nested.0, nested.1.clone());
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    select(&view, &sha, cx);
    let painted = first_frame(&view, cx, |cx| {
        panel.update(cx, |p, cx| p.set_all_files(true, cx));
        cx.run_until_parked();
    });
    assert!(wide(&painted, "commit-name-docs", 20.) && wide(&painted, "commit-name-bin.dat", 30.), "View all files: whole names at once, never cut to nothing");
    let painted = first_frame(&view, cx, |cx| {
        panel.update(cx, |p, cx| {
            p.set_all_files(false, cx);
            p.set_view(FileView::Path, cx);
        })
    });
    assert!(wide(&painted, "commit-files-row-src/app/main.rs", 100.));
    assert_eq!(supraman_ui::clipped::cut_at("commit-dir-src/app/main.rs"), None, "Path: the folder part fits in the first frame");
}

#[gpui::test]
fn commit_details_keep_the_last_counts_and_files_unpainted_while_the_next_commit_loads(cx: &mut TestAppContext) {
    let nested = nested();
    let (f, sha) = (&nested.0, nested.1.clone());
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    select(&view, &sha, cx);
    let painted = repaint(&view, cx);
    assert!(painted.borrow().contains_key("commit-tree-row-docs"));
    view.update(cx, |v, cx| v.select_sha(f.sha("merge"), cx));
    assert!(panel.read_with(cx, |p, _| p.stale()), "the next commit loads: the shown one's files are stale");
    hits(&view, cx).borrow_mut().clear();
    cx.update(|window, cx| {
        let drawn = window.draw(cx);
        drawn.clear(cx);
    });
    assert!(!hits(&view, cx).borrow().keys().any(|k| k.starts_with("commit-tree-row-")), "no row of the earlier commit paints meanwhile");
    cx.run_until_parked();
    assert!(!panel.read_with(cx, |p, _| p.stale()) && shown(&panel, cx).sha.as_ref() == f.sha("merge"), "then the new one shows in one go");
}

#[gpui::test]
fn commit_details_read_the_whole_tree_only_while_view_all_files_is_on(cx: &mut TestAppContext) {
    let nested = nested();
    let (f, sha) = (&nested.0, nested.1.clone());
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    select(&view, &sha, cx);
    assert_eq!(panel.read_with(cx, |p, _| (p.whole, p.all.row_count())), (Whole::Unread, 0), "a selection with View all files off reads no whole tree");

    let painted = first_frame(&view, cx, |cx| panel.update(cx, |p, cx| p.set_all_files(true, cx)));
    assert!(painted.borrow().contains_key("commit-files-reading"), "the frame after the click says the tree is read");
    assert!(!painted.borrow().keys().any(|k| k.starts_with("commit-tree-row-")), "and paints no partial tree");
    cx.run_until_parked();
    panel.update(cx, |p, cx| p.expand_or_collapse(cx));
    assert!(visible(&panel, cx).iter().any(|p| p == "bin.dat"), "then the whole tree shows, an unchanged file in it");
    assert!(shown(&panel, cx).files.contains("bin.dat"), "and its unchanged files open");

    select(&view, f.sha("merge"), cx);
    assert_eq!(panel.read_with(cx, |p, _| p.whole), Whole::Read, "with View all files on, the next selection reads its tree with it");
}

#[gpui::test]
fn commit_details_file_order_follows_the_list_on_show_with_each_files_change(cx: &mut TestAppContext) {
    let nested = nested();
    let (f, sha) = (&nested.0, nested.1.clone());
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    select(&view, &sha, cx);
    let order = |cx: &mut VisualTestContext| -> Vec<(String, Option<supraman_gitman::ChangeKind>)> { panel.read_with(cx, |p, _| p.file_order(&sha).into_iter().map(|t| (t.path.clone(), t.kind())).collect()) };
    use supraman_gitman::ChangeKind::{Added, Deleted, Modified};
    let changed = [("a.txt", Some(Modified)), ("b.txt", Some(Deleted)), ("docs/guide.md", Some(Added)), ("src/app/lib.rs", Some(Added)), ("src/app/main.rs", Some(Added)), ("src/util.rs", Some(Added))];
    let owned = |l: &[(&str, Option<supraman_gitman::ChangeKind>)]| l.iter().map(|(p, k)| (p.to_string(), *k)).collect::<Vec<_>>();
    panel.update(cx, |p, cx| p.set_view(FileView::Path, cx));
    assert_eq!(order(cx), owned(&changed), "Path: the changes A–Z");
    panel.update(cx, |p, cx| p.set_order(Order::Descending, cx));
    assert_eq!(order(cx), owned(&changed.iter().rev().copied().collect::<Vec<_>>()), "Path Z–A");
    panel.update(cx, |p, cx| p.set_order(Order::Ascending, cx));
    panel.update(cx, |p, cx| p.set_view(FileView::Tree, cx));
    let tree = [changed[2], changed[3], changed[4], changed[5], changed[0], changed[1]];
    assert_eq!(order(cx), owned(&tree), "Tree: folders first");
    panel.update(cx, |p, cx| p.set_all_files(true, cx));
    cx.run_until_parked();
    let all = [changed[2], changed[3], changed[4], changed[5], changed[0], changed[1], ("bin.dat", None), ("old.txt", None)];
    assert_eq!(order(cx), owned(&all), "View all files: every file, the unchanged ones without a change");
}

#[gpui::test]
fn a_commit_selection_shows_its_details_after_two_git_reads_and_reads_whether_it_amends_after(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let head = f.git(&["rev-parse", "HEAD"]);
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let panel = panel_of(&view, cx);
    let before = supraman_core::process::spawned_here();
    view.update(cx, |v, cx| v.select_sha(&head, cx));
    while panel.read_with(cx, |p, _| p.shown().is_none_or(|s| s.sha != head.as_str() || p.stale())) {
        assert!(cx.executor().tick(), "the details land");
    }
    assert_eq!(supraman_core::process::spawned_here() - before, 2, "details and changed_files, nothing else before they show");
    assert_eq!(shown(&panel, cx).amendable, None, "the message box is read-only until the second read lands");
    cx.run_until_parked();
    assert_eq!(shown(&panel, cx).amendable, Some(Amendable::Branch { name: "main".into() }));
}

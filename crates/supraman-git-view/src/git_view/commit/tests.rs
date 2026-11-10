//! The commit, WIP row and file menus through the real tab, on fixture copies.


use gpui::{Entity, VisualTestContext, point, px};
use supraman_gitman::fixture::{Fixture, run_git};

use crate::git_view::GitView;
use crate::git_view::test_support::{Opts, open, redraw};
use crate::git_view::test_support::{click, menu, repaint, right_click, row_of, select_wip, toast};

/// The menu of `sha`'s row, opened by a right-click on the row.
fn row_menu(view: &Entity<GitView>, sha: &str, cx: &mut VisualTestContext) -> Vec<String> {
    view.update(cx, |v, cx| v.branch.update(cx, |b, cx| b.show_menu(String::new(), Vec::new(), point(px(0.), px(0.)), cx)));
    let ix = row_of(view, sha, cx);
    right_click(view, &format!("git-row-{ix}"), cx);
    menu(view, cx)
}

fn asking(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, cx| v.commit.read(cx).asking().map(str::to_string))
}

const RESETS: [&str; 3] = ["Soft: keep all changes", "Mixed: keep the working copy, reset the index", "Hard: discard all changes"];

#[gpui::test]
fn a_commit_rows_menu_follows_its_kind_and_its_branch_groups(cx: &mut gpui::TestAppContext) {
    let f = Fixture::copy();
    f.at(&f.dir, &["branch", "-q", "one", f.sha("root")]);
    f.at(&f.dir, &["branch", "-q", "two", f.sha("root")]);
    f.at(&f.dir, &["tag", "plain", f.sha("rename")]);
    let (view, cx) = open(&f.worktree, Opts::default(), cx);

    let mut b1 = vec!["Checkout this commit", "Create branch here", "Cherry-pick commit", "Rebase side onto this commit", "Edit commit message", "Revert commit"];
    b1.extend(RESETS);
    b1.extend(["Copy commit sha", "Create patch from commit", "Create tag here", "Create annotated tag here"]);
    let rename = row_menu(&view, f.sha("rename"), cx);
    for item in ["Checkout feature", "Rename feature", "Checkout this commit", "Cherry-pick commit"] {
        assert!(rename.contains(&item.to_string()), "one branch group: its menu with the commit's items, {item} too: {rename:?}");
    }
    assert!(!rename.iter().any(|l| l.contains("plain")), "a tag is no group of the commit menu: {rename:?}");

    let head = row_menu(&view, f.sha("second"), cx);
    assert!(!head.iter().any(|l| l.starts_with("Cherry-pick") || l.starts_with("Rebase side onto")), "HEAD: no cherry-pick, no rebase onto: {head:?}");
    assert!(head.contains(&"Edit commit message".to_string()) && head.contains(&"Revert commit".to_string()));

    let merge = row_menu(&view, f.sha("merge"), cx);
    assert!(merge.contains(&"Cherry-pick commit".to_string()) && !merge.contains(&"Edit commit message".to_string()), "a merge's message is not edited: {merge:?}");

    let root = row_menu(&view, f.sha("root"), cx);
    let expected: Vec<String> = b1.iter().filter(|l| **l != "Edit commit message").map(|s| s.to_string()).collect();
    assert_eq!(root[..expected.len()], expected[..], "several groups: B1's items first, a root's message not edited");
    assert!(root[expected.len()..].contains(&"Checkout one".to_string()) && root[expected.len()..].contains(&"Checkout two".to_string()), "then each group's: {root:?}");
}

#[gpui::test]
fn the_commit_items_cherry_pick_revert_reset_and_edit_the_message(cx: &mut gpui::TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.worktree, Opts::default(), cx);
    let reload = |view: &Entity<GitView>, cx: &mut VisualTestContext| {
        f.at(&f.worktree, &["reset", "-q", "--hard", f.sha("second")]);
        view.update(cx, |v, cx| v.reload(cx));
        cx.run_until_parked();
    };
    row_menu(&view, f.sha("root"), cx);
    click(&view, "menu-commit-reset-mixed", cx);
    assert!(asking(&view, cx).is_some_and(|q| q.starts_with("side was already pushed")), "origin/main holds HEAD's second: asked first");
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), f.sha("second"), "nothing before the answer");
    click(&view, "git-commit-ask-yes", cx);
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), f.sha("root"));
    assert_eq!(f.at(&f.worktree, &["diff", "--cached", "--name-only"]), "", "mixed: the index reset");
    assert_eq!(f.at(&f.worktree, &["diff", "--name-only"]), "a.txt", "the working copy kept");

    reload(&view, cx);
    row_menu(&view, f.sha("rename"), cx);
    click(&view, "menu-commit-cherry-pick", cx);
    assert!(asking(&view, cx).is_some_and(|q| q.ends_with("commit the changes now?")));
    click(&view, "git-commit-ask-yes", cx);
    assert_eq!(f.at(&f.worktree, &["log", "-1", "--format=%s"]), "Move old.txt to new.txt");
    assert!(toast(&view, cx).is_some_and(|t| t.starts_with("Cherry-picked")));

    let head = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    row_menu(&view, f.sha("second"), cx);
    click(&view, "menu-commit-revert", cx);
    click(&view, "git-commit-ask-alt", cx);
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), head, "Don't commit: nothing committed");
    assert_eq!(f.at(&f.worktree, &["diff", "--cached", "--name-only"]), "a.txt", "the revert waits staged");

    reload(&view, cx);
    row_menu(&view, f.sha("second"), cx);
    click(&view, "menu-commit-edit-message", cx);
    cx.run_until_parked();
    redraw(&view, cx);
    assert!(view.read_with(cx, |v, cx| v.panel().read(cx).field().is_some()), "HEAD's message field is open");
}

#[gpui::test]
fn the_wip_rows_menu_lists_stage_stash_and_discard_all_and_stage_all_stages_everything(cx: &mut gpui::TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    view.update(cx, |v, cx| v.row_menu(0, point(px(200.), px(200.)), cx));
    cx.run_until_parked();
    assert_eq!(menu(&view, cx), ["Stage all changes", "Stash all changes", "Discard all changes"]);
    click(&view, "menu-wip-menu-stage-all", cx);
    assert_eq!(f.at(&f.dir, &["diff", "--name-only"]), "", "nothing left unstaged");
}

#[gpui::test]
fn a_wip_files_menu_ignores_untracks_and_deletes(cx: &mut gpui::TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    select_wip(&view, cx);
    right_click(&view, "wip-unstaged-files-row-u.txt", cx);
    assert_eq!(menu(&view, cx), ["Stage file", "Discard changes", "Ignore this file", "Ignore all *.txt files", "Stash file", "Open with the default program", "Show in folder", "Copy path", "Delete file"]);
    click(&view, "menu-file-ignore-0-u.txt", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("Add /u.txt to .gitignore?"));
    assert!(!repaint(&view, cx).borrow().contains_key("git-commit-ask-alt"), "an untracked file has nothing to stop tracking");
    click(&view, "git-commit-ask-yes", cx);
    assert_eq!(std::fs::read_to_string(f.dir.join(".gitignore")).unwrap(), "/u.txt\n");

    right_click(&view, "wip-unstaged-files-row-a.txt", cx);
    click(&view, "menu-file-ignore-0-a.txt", cx);
    click(&view, "git-commit-ask-alt", cx);
    assert_eq!(f.at(&f.dir, &["ls-files", "a.txt"]), "", "Ignore and stop tracking");
    assert!(f.dir.join("a.txt").exists());

    right_click(&view, "wip-staged-files-row-s.txt", cx);
    assert_eq!(menu(&view, cx)[..2], ["Unstage file", "Discard changes"]);
    click(&view, "menu-file-delete-s.txt", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("Delete s.txt? A file git does not track cannot be brought back."));
    click(&view, "git-commit-ask-yes", cx);
    assert!(!f.dir.join("s.txt").exists());
}

#[gpui::test]
fn a_commits_file_is_restored_from_it_asked_first_over_local_changes(cx: &mut gpui::TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    view.update(cx, |v, cx| v.select_sha(f.sha("second"), cx));
    cx.run_until_parked();
    right_click(&view, "commit-tree-row-a.txt", cx);
    assert_eq!(menu(&view, cx), ["Restore file from this commit", "Open with the default program", "Show in folder", "Copy path", "Delete file"]);
    click(&view, "menu-file-restore-a.txt", cx);
    assert!(asking(&view, cx).is_some_and(|q| q.starts_with("Restoring 1 file from")), "the work tree changed a.txt");
    click(&view, "git-commit-ask-yes", cx);
    assert_eq!(std::fs::read_to_string(f.dir.join("a.txt")).unwrap(), "one\ntwo\n");
    let staged = run_git(&f.git, &f.dir, &["diff", "--cached", "--quiet", f.sha("second"), "--", "a.txt"], &[], None);
    assert!(staged.success(), "the index has second's a.txt too");
}

#[test]
fn a_copied_path_is_in_the_platforms_own_separators() {
    let copied = super::copied_path(std::path::Path::new("C:/x/repo"), "a/b.txt");
    assert_eq!(copied, if cfg!(windows) { r"C:\x\repo\a\b.txt" } else { "C:/x/repo/a/b.txt" });
}

use std::path::{Path, PathBuf};

use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::*;

/// `wt` stopped cherry-picking on a conflict with an add, a delete and a rename; `det` detached with changes.
fn two_linked_worktrees() -> (Fixture, PathBuf) {
    let f = Fixture::copy();
    f.cherry_pick_conflict();
    let wt = |args: &[&str]| git_in(&f.git, &f.worktree, args, &[], None);
    std::fs::write(f.worktree.join("added.txt"), b"added\n").unwrap();
    wt(&["add", "added.txt"]);
    wt(&["rm", "-q", "b.txt"]);
    wt(&["mv", "old.txt", "moved.txt"]);
    wt(&["branch", "-q", "-u", "origin/main"]);
    let det = f.temp.path().join("det");
    f.git(&["worktree", "add", "-q", "--detach", &det.to_string_lossy(), f.sha("second")]);
    std::fs::write(det.join("a.txt"), b"one\ntwo\ndetached\n").unwrap();
    std::fs::write(det.join("new.md"), b"untracked\n").unwrap();
    (f, det)
}

fn top(f: &Fixture) -> PathBuf {
    dirs(&f.repo()).unwrap().top
}

fn of<'a>(wips: &'a [WorktreeWip], path: &Path) -> &'a WorktreeWip {
    let key = supraman_core::paths::path_key(path);
    wips.iter().find(|w| supraman_core::paths::path_key(w.path()) == key).unwrap_or_else(|| panic!("no {} in {wips:?}", path.display()))
}

#[test]
fn each_other_worktree_has_its_head_branch_counts_upstream_and_files() {
    let (f, det) = two_linked_worktrees();
    let wips = wips(&f.repo(), &[top(&f)]).unwrap();
    assert_eq!(wips.len(), 2, "the main checkout the tab shows is left out: {wips:?}");

    let wt = of(&wips, &f.worktree);
    let wt_head = git_in(&f.git, &f.worktree, &["rev-parse", "HEAD"], &[], None);
    assert_eq!((wt.head(), wt.branch(), wt.detached()), (Some(wt_head.as_str()), Some("side"), false));
    assert_eq!(wt.counts(), WipCounts { added: 1, deleted: 1, modified: 1, renamed: 1 });
    let counts = git_in(&f.git, &f.worktree, &["rev-list", "--left-right", "--count", "HEAD...origin/main"], &[], None);
    let (ahead, behind) = counts.split_once('\t').unwrap();
    assert_eq!(wt.upstream(), Some(("origin/main", ahead.parse().unwrap(), behind.parse().unwrap())));
    let mut files: Vec<(&str, Option<&str>)> = wt.files().iter().map(|e| (e.path.as_str(), e.orig_path.as_deref())).collect();
    files.sort();
    assert_eq!(files, [("a.txt", None), ("added.txt", None), ("b.txt", None), ("moved.txt", Some("old.txt"))]);
    assert_eq!(wt.files().iter().find(|e| e.path == "a.txt").and_then(Entry::conflict), Some(Conflict::BothModified));

    let det = of(&wips, &det);
    assert_eq!((det.head(), det.branch(), det.detached(), det.upstream()), (Some(f.sha("second")), None, true, None));
    assert_eq!(det.counts(), WipCounts { added: 1, deleted: 0, modified: 1, renamed: 0 });
    let mut files: Vec<&str> = det.files().iter().map(|e| e.path.as_str()).collect();
    files.sort();
    assert_eq!(files, ["a.txt", "new.md"]);
}

#[test]
fn a_linked_worktree_shown_leaves_the_main_checkout_among_the_others() {
    let (f, det) = two_linked_worktrees();
    let wips = wips(&f.repo(), &[det.clone()]).unwrap();
    assert_eq!(wips.len(), 2, "{wips:?}");
    let main = of(&wips, &top(&f));
    assert_eq!((main.branch(), main.head()), (Some("main"), Some(f.sha("ahead"))));
    assert!(main.counts().total() > 0, "the fixture's main checkout has work in progress");
    of(&wips, &f.worktree);
}

#[test]
fn a_worktree_whose_folder_is_gone_is_said_so_and_the_others_still_read() {
    let (f, det) = two_linked_worktrees();
    std::fs::remove_dir_all(&det).unwrap();
    let wips = wips(&f.repo(), &[top(&f)]).unwrap();
    let gone = of(&wips, &det);
    assert_eq!((&gone.read, gone.head(), gone.files().len()), (&WipRead::Gone, Some(f.sha("second")), 0));
    assert!(matches!(of(&wips, &f.worktree).read, WipRead::Read(_)));
}

#[test]
fn a_repository_with_one_worktree_reads_nothing() {
    let f = Fixture::copy();
    f.git(&["worktree", "remove", "--force", &f.worktree.to_string_lossy()]);
    assert_eq!(wips(&f.repo(), &[top(&f)]).unwrap(), []);
}

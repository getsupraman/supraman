//! A commit's and a file's writes from the graph's menus, on fixture copies only (C196).

use std::path::Path;

use supraman_gitman::fixture::{Fixture, run_git};
use supraman_gitman::write::commit::{self, Picked, ResetMode};
use supraman_gitman::write::files::{self, Ignore};
use supraman_gitman::write::rebase::Autostash;
use supraman_gitman::write::{LockSet, Session, WriteError};
use supraman_gitman::*;

fn write<T>(repo: &Repo, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Fixture::write_job(repo, LockSet::checkout(), job)
}

fn subject(f: &Fixture, dir: &Path) -> String {
    f.at(dir, &["log", "-1", "--format=%s"])
}

fn stash_count(f: &Fixture) -> usize {
    stashes(&f.repo()).unwrap().len()
}

#[test]
fn a_cherry_pick_commits_the_change_and_puts_the_work_back() {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("b.txt"), b"b\nmine\n").unwrap();
    let stashes = stash_count(&f);
    let picked = write(&f.side(), |s| commit::cherry_pick(s, f.sha("rename"), true)).unwrap();
    assert_eq!(picked, Picked::Done(Autostash::Popped));
    assert_eq!(subject(&f, &f.worktree), "Move old.txt to new.txt");
    assert!(f.worktree.join("new.txt").exists());
    assert_eq!(std::fs::read_to_string(f.worktree.join("b.txt")).unwrap(), "b\nmine\n", "the work is back");
    assert_eq!(stash_count(&f), stashes, "never a stranded stash");
}

#[test]
fn a_merge_is_picked_and_reverted_against_its_first_parent() {
    let f = Fixture::copy();
    assert!(commit::is_merge(&f.repo(), f.sha("merge")).unwrap() && !commit::is_merge(&f.repo(), f.sha("second")).unwrap());
    write(&f.side(), |s| commit::cherry_pick(s, f.sha("merge"), true)).unwrap();
    assert!(f.worktree.join("new.txt").exists() && !f.worktree.join("old.txt").exists(), "the merge's change against main_work: the rename");
    write(&f.side(), |s| commit::revert(s, f.sha("second"), true)).unwrap();
    assert_eq!(std::fs::read_to_string(f.worktree.join("a.txt")).unwrap(), "one\n");
    assert!(subject(&f, &f.worktree).starts_with("Revert \"Second\""));
}

#[test]
fn without_a_commit_the_change_is_left_staged() {
    let f = Fixture::copy();
    let head = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    write(&f.side(), |s| commit::cherry_pick(s, f.sha("ahead"), false)).unwrap();
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), head, "nothing committed");
    assert_eq!(f.at(&f.worktree, &["diff", "--cached", "--name-only"]), "a.txt");
}

#[test]
fn a_conflict_leaves_the_cherry_pick_under_way_and_keeps_the_stash() {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("a.txt"), b"one\nside\n").unwrap();
    f.at(&f.worktree, &["commit", "-q", "--no-verify", "-am", "Side's a.txt"]);
    std::fs::write(f.worktree.join("b.txt"), b"b\nmine\n").unwrap();
    let stashes = stash_count(&f);
    let Picked::Conflicted(Some(kept)) = write(&f.side(), |s| commit::cherry_pick(s, f.sha("ahead"), true)).unwrap() else { panic!("a conflict with the stash kept") };
    assert!(kept.name.contains("cherry-pick"), "{}", kept.name);
    let dirs = dirs(&f.side()).unwrap();
    assert!(matches!(operation_state(&dirs.git_dir), OperationState::CherryPick { .. }));
    assert_eq!(stash_count(&f), stashes + 1);
}

#[test]
fn a_hard_reset_snapshots_the_work_and_undo_brings_it_back() {
    let f = Fixture::copy();
    let before = f.at(&f.dir, &["rev-parse", "HEAD"]);
    write(&f.repo(), |s| commit::reset(s, f.sha("main_work"), ResetMode::Hard)).unwrap();
    assert_eq!(f.at(&f.dir, &["rev-parse", "HEAD"]), f.sha("main_work"));
    assert!(!f.dir.join("s.txt").exists(), "the work is gone");
    Fixture::undo(&f.repo()).unwrap();
    assert_eq!(f.at(&f.dir, &["rev-parse", "HEAD"]), before);
    assert_eq!(std::fs::read_to_string(f.dir.join("a.txt")).unwrap(), "staged\nunstaged\n", "undo brings the work back");
}

#[test]
fn a_soft_reset_keeps_the_changes_staged_and_a_mixed_one_unstaged() {
    let f = Fixture::copy();
    write(&f.side(), |s| commit::reset(s, f.sha("root"), ResetMode::Soft)).unwrap();
    assert_eq!(f.at(&f.worktree, &["diff", "--cached", "--name-only"]), "a.txt");
    write(&f.side(), |s| commit::reset(s, f.sha("root"), ResetMode::Mixed)).unwrap();
    assert_eq!(f.at(&f.worktree, &["diff", "--cached", "--name-only"]), "");
    assert_eq!(f.at(&f.worktree, &["diff", "--name-only"]), "a.txt");
}

#[test]
fn a_patch_is_written_to_the_folder() {
    let f = Fixture::copy();
    let folder = f.temp.path().join("patches");
    std::fs::create_dir(&folder).unwrap();
    let file = commit::format_patch(&f.repo(), f.sha("second"), &folder).unwrap();
    assert!(file.starts_with(&folder) || folder.join(&file).exists(), "{}", file.display());
    let patch = std::fs::read_to_string(folder.join(file.file_name().unwrap())).unwrap();
    assert!(patch.contains("Subject: [PATCH] Second"), "{patch}");
}

#[test]
fn a_file_is_restored_from_a_commit_and_undo_brings_the_change_back() {
    let f = Fixture::copy();
    let paths = vec!["a.txt".to_string(), "s.txt".to_string()];
    write(&f.repo(), |s| files::restore(s, f.sha("root"), &paths)).unwrap();
    assert_eq!(std::fs::read_to_string(f.dir.join("a.txt")).unwrap(), "one\n");
    assert!(!f.dir.join("s.txt").exists(), "root has no s.txt: it goes");
    assert_eq!(f.at(&f.dir, &["diff", "--cached", "--name-only", "--", "s.txt"]), "", "and leaves the index");
    Fixture::undo(&f.repo()).unwrap();
    assert_eq!(std::fs::read_to_string(f.dir.join("a.txt")).unwrap(), "staged\nunstaged\n");
    assert_eq!(std::fs::read_to_string(f.dir.join("s.txt")).unwrap(), "new\n");
}

#[test]
fn ignore_appends_its_pattern_once_and_stops_tracking_when_asked() {
    let f = Fixture::copy();
    write(&f.repo(), |s| files::ignore(s, &Ignore::File("u.txt".into()), &[])).unwrap();
    write(&f.repo(), |s| files::ignore(s, &Ignore::File("u.txt".into()), &[])).unwrap();
    assert_eq!(std::fs::read_to_string(f.dir.join(".gitignore")).unwrap(), "/u.txt\n", "once");
    write(&f.repo(), |s| files::ignore(s, &Ignore::Extension("dat".into()), &["bin.dat".to_string()])).unwrap();
    assert_eq!(std::fs::read_to_string(f.dir.join(".gitignore")).unwrap(), "/u.txt\n*.dat\n");
    assert_eq!(f.at(&f.dir, &["ls-files", "bin.dat"]), "", "no longer tracked");
    assert!(f.dir.join("bin.dat").exists(), "its file kept");
    let out = run_git(&f.git, &f.dir, &["check-ignore", "-q", "u.txt"], &[], None);
    assert!(out.success());
}

#[test]
fn delete_removes_the_working_file() {
    let f = Fixture::copy();
    write(&f.repo(), |s| files::delete(s, "u.txt")).unwrap();
    assert!(!f.dir.join("u.txt").exists());
}

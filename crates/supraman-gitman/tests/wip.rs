//! The WIP panel's writes on fixture copies only (C196): stage, unstage, discard and commit.

use std::path::Path;

use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::write::journal::{self, Done, Refused};
use supraman_gitman::write::wip::{self, Step};
use supraman_gitman::write::{LockSet, Session, WriteError, Writer};
use supraman_gitman::*;

fn change(kind: ChangeKind, path: &str, old: Option<&str>) -> FileChange {
    FileChange { kind, path: path.into(), old_path: old.map(Into::into), similarity: None }
}

fn write_in<T>(repo: Repo, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Writer::open(&repo).unwrap().run(LockSet::commit(), job)
}

fn write<T>(f: &Fixture, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> T {
    write_in(f.repo(), job).unwrap()
}

fn undo(f: &Fixture) -> Result<Done, Refused> {
    Writer::open(&f.repo()).unwrap().run(LockSet::undo(), |s| Ok(journal::undo(s, true))).unwrap()
}

fn git(f: &Fixture, dir: &Path, args: &[&str]) -> String {
    git_in(&f.git, dir, args, &[], None)
}

fn status_lines(f: &Fixture) -> Vec<String> {
    git(f, &f.dir, &["status", "--porcelain=v1", "--untracked-files=all"]).lines().map(str::to_string).collect()
}

fn read(f: &Fixture, path: &str) -> Option<Vec<u8>> {
    std::fs::read(f.dir.join(path)).ok()
}

#[test]
fn staging_takes_untracked_and_deleted_files_and_unstaging_a_rename_brings_back_its_old_path() {
    let f = Fixture::copy();
    write(&f, |s| wip::stage(s, &[change(ChangeKind::Untracked, "u.txt", None), change(ChangeKind::Deleted, "b.txt", None)]));
    let staged = git(&f, &f.dir, &["diff", "--cached", "--name-status"]);
    assert!(staged.contains("A\tu.txt") && staged.contains("D\tb.txt"), "{staged}");

    write(&f, |s| wip::unstage(s, &[change(ChangeKind::Renamed, "renamed.txt", Some("new.txt"))]));
    let index = git(&f, &f.dir, &["ls-files"]);
    assert!(index.lines().any(|l| l == "new.txt") && !index.lines().any(|l| l == "renamed.txt"), "the old path is back in the index: {index}");
    assert!(status_lines(&f).contains(&"?? renamed.txt".to_string()), "the new path is untracked again");

    write(&f, |s| wip::unstage(s, &[change(ChangeKind::Untracked, "u.txt", None), change(ChangeKind::Added, "s.txt", None)]));
    assert!(git(&f, &f.dir, &["diff", "--cached", "--name-only"]).lines().all(|l| l != "u.txt" && l != "s.txt"));
}

#[test]
fn unstaging_keeps_skip_worktree_and_assume_unchanged() {
    let f = Fixture::copy();
    f.write("bin.dat", b"changed\n");
    f.git(&["add", "bin.dat"]);
    f.git(&["update-index", "--skip-worktree", "bin.dat"]);
    f.git(&["update-index", "--assume-unchanged", "a.txt"]);
    write(&f, |s| wip::unstage(s, &[change(ChangeKind::Modified, "a.txt", None), change(ChangeKind::Modified, "bin.dat", None)]));
    assert_eq!(git(&f, &f.dir, &["diff", "--cached", "--name-only", "--", "a.txt", "bin.dat"]), "", "both unstaged");
    let tags = git(&f, &f.dir, &["ls-files", "-v", "--", "a.txt", "bin.dat"]);
    assert!(tags.lines().any(|l| l == "h a.txt") && tags.lines().any(|l| l == "S bin.dat"), "the flags stay: {tags}");
}

#[test]
fn a_file_that_became_a_folder_is_staged_as_the_folder() {
    let f = Fixture::copy();
    std::fs::remove_file(f.dir.join("bin.dat")).unwrap();
    std::fs::create_dir(f.dir.join("bin.dat")).unwrap();
    f.write("bin.dat/x.txt", b"x\n");
    write(&f, |s| wip::stage(s, &[change(ChangeKind::Deleted, "bin.dat", None), change(ChangeKind::Untracked, "bin.dat/x.txt", None)]));
    let index = git(&f, &f.dir, &["ls-files", "--", "bin.dat"]);
    assert_eq!(index, "bin.dat/x.txt");
}

#[test]
fn staging_a_conflicted_file_resolves_it_and_deletes_its_orig_leftover() {
    let f = Fixture::copy();
    f.conflict();
    std::fs::write(f.worktree.join("a.txt.orig"), b"leftover\n").unwrap();
    std::fs::write(f.worktree.join("a.txt"), b"resolved\n").unwrap();
    write_in(Repo::new(f.git.clone(), &f.worktree), |s| wip::stage(s, &[change(ChangeKind::Unmerged, "a.txt", None)])).unwrap();
    assert_eq!(git(&f, &f.worktree, &["diff", "--name-only", "--diff-filter=U"]), "", "no conflict left");
    assert!(!f.worktree.join("a.txt.orig").exists(), "the leftover is gone");
}

#[test]
fn discarding_the_unstaged_list_keeps_the_staged_part_and_undo_brings_it_all_back() {
    let f = Fixture::copy();
    let before = (status_lines(&f), read(&f, "a.txt"), read(&f, "u.txt"));
    let files = [change(ChangeKind::Modified, "a.txt", None), change(ChangeKind::Deleted, "b.txt", None), change(ChangeKind::Untracked, "u.txt", None)];
    write(&f, |s| wip::discard_unstaged(s, &files));
    assert_eq!(read(&f, "a.txt").as_deref(), Some(&b"staged\n"[..]), "the working file is the index's");
    assert_eq!(git(&f, &f.dir, &["diff", "--cached", "--name-only", "--", "a.txt"]), "a.txt", "the staged change stays");
    assert_eq!(read(&f, "b.txt").as_deref(), Some(&b"b\nb2\n"[..]), "the deleted file is back");
    assert_eq!(read(&f, "u.txt"), None, "the untracked file is deleted");
    assert_eq!(undo(&f).unwrap().toast, "Undid discard \"a.txt\" (and 2 other files)");
    assert_eq!((status_lines(&f), read(&f, "a.txt"), read(&f, "u.txt")), before);
}

#[test]
fn discard_all_keeps_ignored_files_removes_untracked_folders_and_undo_restores_everything() {
    let f = Fixture::copy();
    std::fs::write(f.dir.join(".git/info/exclude"), b"*.log\n").unwrap();
    f.write("keep.log", b"ignored\n");
    std::fs::create_dir(f.dir.join("newdir")).unwrap();
    f.write("newdir/x.txt", b"x\n");
    let before = (status_lines(&f), git(&f, &f.dir, &["ls-files", "-s"]), read(&f, "a.txt"), read(&f, "newdir/x.txt"));
    let head = git(&f, &f.dir, &["reflog", "-1", "--format=%gs", "HEAD"]);

    assert_eq!(write(&f, wip::discard_all), 7);
    assert_eq!(status_lines(&f), Vec::<String>::new(), "nothing left to commit");
    assert!(f.dir.join("keep.log").exists(), "an ignored file stays");
    assert!(!f.dir.join("newdir").exists(), "an untracked folder goes with its files");
    assert_eq!(git(&f, &f.dir, &["reflog", "-1", "--format=%gs", "HEAD"]), head, "the reset leaves no line in HEAD's reflog");

    assert_eq!(undo(&f).unwrap().toast, "Undid discard \"a.txt\" (and 6 other files)");
    assert_eq!((status_lines(&f), git(&f, &f.dir, &["ls-files", "-s"]), read(&f, "a.txt"), read(&f, "newdir/x.txt")), before, "undo restores the index and every file");
}

fn hook(f: &Fixture, name: &str, body: &str) {
    std::fs::write(f.dir.join(".git/hooks").join(name), format!("#!/bin/sh\n{body}\n")).unwrap();
}

/// git exits 1 whatever code the hook exits with.
#[test]
fn a_commit_hook_failure_is_its_exit_code_and_stderr_and_skip_hooks_commits() {
    let f = Fixture::copy();
    assert!(!commit_hooks(&f.repo()).unwrap(), "only samples");
    hook(&f, "commit-msg", "echo 'no ticket in the message' >&2\nexit 3");
    assert!(commit_hooks(&f.repo()).unwrap());
    let head = f.git(&["rev-parse", "HEAD"]);

    let failed = write_in(f.repo(), |s| wip::commit(s, &wip::message("Fix it", ""), false, false)).unwrap_err();
    assert!(matches!(&failed, WriteError::CommitFailed { code: 1, stderr } if stderr.contains("no ticket in the message")), "{failed}");
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head, "nothing committed");
    assert!(matches!(write_in(f.repo(), |s| wip::commit(s, "  \n", false, true)), Err(WriteError::Refused(_))), "a blank message is refused");

    write(&f, |s| wip::commit(s, &wip::message("Fix it", "Because"), false, true));
    assert_eq!(f.git(&["log", "-1", "--format=%B"]), "Fix it\n\nBecause");
    assert_eq!(f.git(&["rev-parse", "HEAD^"]), head);
    assert_eq!(journal::preview(&Writer::open(&f.repo()).unwrap()).unwrap().undo_tooltip(), "Undo commit \"Fix it\"");
}

#[test]
fn an_amend_takes_what_is_staged_and_the_new_message_and_undo_takes_it_back() {
    let f = Fixture::copy();
    let (head, parent) = (f.git(&["rev-parse", "HEAD"]), f.git(&["rev-parse", "HEAD^"]));
    assert_eq!(head_message(&f.repo()).unwrap().as_deref(), Some("Ahead of origin"));
    write(&f, |s| wip::commit(s, &wip::message("Ahead, amended", ""), true, false));
    assert_eq!((f.git(&["log", "-1", "--format=%s"]), f.git(&["rev-parse", "HEAD^"])), ("Ahead, amended".to_string(), parent));
    assert!(f.git(&["show", "--name-only", "--format=", "HEAD"]).lines().any(|l| l == "s.txt"), "the staged file is in the amended commit");
    assert_eq!(undo(&f).unwrap().toast, "Undid commit amend \"Ahead, amended\"");
    assert_eq!(f.git(&["rev-parse", "HEAD"]), head);
}

#[test]
fn a_merge_aborts_from_the_commit_area() {
    let f = Fixture::copy();
    f.conflict();
    write_in(Repo::new(f.git.clone(), &f.worktree), |s| wip::step(s, Step::AbortMerge)).unwrap();
    assert_eq!(operation_state(&f.dir.join(".git/worktrees/wt")), OperationState::None);
}

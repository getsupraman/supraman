//! An older commit reworded and the rebase's continue, skip and abort, on fixture copies and their bare temp
//! remote only (C196).

use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::rebase::{self, Autostash, Rebased};
use supraman_gitman::write::wip::{self, Step};
use supraman_gitman::write::{LockSet, Session, WriteError};
use supraman_gitman::*;

fn write<T>(repo: &Repo, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Fixture::write_job(repo, LockSet::rebase(), job)
}

fn commit(f: &Fixture, path: &str, bytes: &[u8], message: &str) -> String {
    std::fs::write(f.worktree.join(path), bytes).unwrap();
    f.at(&f.worktree, &["add", path]);
    f.at(&f.worktree, &["commit", "-q", "--no-verify", "-m", message]);
    f.at(&f.worktree, &["rev-parse", "HEAD"])
}

/// `side` (at `second`, above `root`) with three commits on it: w1 adds c.txt, w2 and w3 change it.
fn three_above(f: &Fixture) -> [String; 3] {
    [commit(f, "c.txt", b"one\n", "W1"), commit(f, "c.txt", b"two\n", "W2\n\nWhy two"), commit(f, "c.txt", b"three\n", "W3")]
}

fn hook(f: &Fixture, name: &str, script: &str) {
    std::fs::write(f.dir.join(".git/hooks").join(name), script).unwrap();
}

/// GitMan's reword of W1 with W2 dropped: the pick of W3 (two to three) above it conflicts on c.txt.
fn stopped(f: &Fixture, [w1, _, w3]: &[String; 3]) -> Rebased {
    f.gitmans_rebase(&f.side(), w1, &format!("reword {w1}
pick {w3}
"))
}

fn log(f: &Fixture, format: &str) -> Vec<String> {
    f.at(&f.worktree, &["log", &format!("--format={format}"), "HEAD"]).lines().map(str::to_string).collect()
}

fn state(f: &Fixture) -> OperationState {
    operation_state(&f.dir.join(".git/worktrees/wt"))
}

/// Staged, unstaged and untracked work in the worktree, and how `status` lists it.
fn work_in_progress(f: &Fixture) -> String {
    std::fs::write(f.worktree.join("b.txt"), b"b staged\n").unwrap();
    f.at(&f.worktree, &["add", "b.txt"]);
    std::fs::write(f.worktree.join("a.txt"), b"one\ntwo\nunstaged\n").unwrap();
    std::fs::write(f.worktree.join("u.txt"), b"untracked\n").unwrap();
    f.at(&f.worktree, &["status", "--porcelain=v1", "--untracked-files=all"])
}

#[test]
fn only_commits_on_heads_first_parent_line_above_the_first_merge_qualify() {
    let f = Fixture::copy();
    let repo = f.repo();
    let tree = f.at(&f.dir, &["rev-parse", "HEAD^{tree}"]);
    let top = f.at(&f.dir, &["commit-tree", &tree, "-p", "HEAD", "-m", "Top"]);
    f.at(&f.dir, &["update-ref", "refs/heads/main", &top]);
    assert_eq!(rewordable(&repo, f.sha("ahead")).unwrap(), Some(1), "one parent, the merge below it");
    for no in ["merge", "main_work", "second", "root", "rename"] {
        assert_eq!(rewordable(&repo, f.sha(no)).unwrap(), None, "{no}: a merge, below one or off the first-parent line");
    }
    assert_eq!(rewordable(&repo, &top).unwrap(), None, "HEAD is an amend");

    let [w1, w2, w3] = three_above(&f);
    let side = f.side();
    assert_eq!([f.sha("second"), &w1, &w2].map(|c| rewordable(&side, c).unwrap()), [Some(3), Some(2), Some(1)]);
    assert_eq!([f.sha("root"), &w3].map(|c| rewordable(&side, c).unwrap()), [None, None], "the root and HEAD");
    assert_eq!(amendable(&side, &w1).unwrap(), Amendable::Older { name: "side".into(), above: 2, pushed: false });
    f.at(&f.worktree, &["push", "-q", "origin", "side"]);
    assert_eq!(amendable(&side, &w1).unwrap(), Amendable::Older { name: "side".into(), above: 2, pushed: true });
    f.at(&f.worktree, &["checkout", "-q", "--detach"]);
    assert_eq!(amendable(&side, &w1).unwrap(), Amendable::OlderDetached);
    assert!(matches!(write(&side, |s| rebase::reword(s, &w1, "New\n")), Err(WriteError::Refused(rebase::DETACHED))));
}

#[test]
fn a_reword_changes_only_that_message_repicks_the_rest_with_their_trees_and_pops_the_autostash() {
    let f = Fixture::copy();
    let [w1, ..] = three_above(&f);
    let (trees, authors, old) = (log(&f, "%T"), log(&f, "%an %ae %ad"), f.at(&f.worktree, &["rev-parse", "HEAD"]));
    let wip = work_in_progress(&f);
    let side = f.side();
    assert!(matches!(write(&side, |s| rebase::reword(s, &w1, "  \n")), Err(WriteError::Refused(rebase::EMPTY_MESSAGE))));

    let done = write(&side, |s| rebase::reword(s, &w1, "W1, reworded\n\nThe body\n")).unwrap();
    let head = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    assert_eq!(done, Rebased::Done { head: head.clone(), autostash: Autostash::Popped });
    assert_eq!(log(&f, "%s"), ["W3", "W2", "W1, reworded", "Second", "Root"]);
    assert_eq!(f.at(&f.worktree, &["log", "-1", "--format=%B", "HEAD~2"]), "W1, reworded\n\nThe body");
    assert_eq!(f.at(&f.worktree, &["log", "-1", "--format=%B", "HEAD~1"]), "W2\n\nWhy two", "a picked commit keeps its message");
    assert_eq!((log(&f, "%T"), log(&f, "%an %ae %ad")), (trees, authors), "the same trees, authors and dates");
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD~3"]), f.sha("second"), "nothing below it moved");
    assert_eq!(f.at(&f.worktree, &["status", "--porcelain=v1", "--untracked-files=all"]), wip.replace("M  b.txt", " M b.txt"), "the work back, unstaged as a stash pop puts it");
    assert!(f.at(&f.worktree, &["stash", "list"]).lines().all(|l| !l.contains("GitMan autostash")), "the autostash is gone");
    assert!(!f.dir.join(".git/worktrees/wt").read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("gitman-reword")), "the prepared files are gone");
    assert_eq!(state(&f), OperationState::None);

    assert_eq!(Fixture::undo(&side).unwrap().toast, format!("Undid rebase from \"{}\"; your changes are in a stash", short_sha(&old)));
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), old, "Undo puts the old tip back");
}

#[test]
fn a_later_pick_that_conflicts_stops_continue_needs_the_files_staged_and_then_finishes() {
    let f = Fixture::copy();
    let commits = three_above(&f);
    let old = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    assert_eq!(stopped(&f, &commits), Rebased::Conflicted);
    assert_eq!(state(&f), OperationState::Rebase { external: false }, "GitMan's marker in the rebase's folder");

    let side = f.side();
    assert!(matches!(write(&side, |s| wip::step(s, Step::ContinueRebase)), Err(WriteError::Refused(rebase::STAGE_FIRST))));
    assert_eq!(state(&f), OperationState::Rebase { external: false }, "still under way");
    std::fs::write(f.worktree.join("c.txt"), b"three
").unwrap();
    f.at(&f.worktree, &["add", "c.txt"]);
    let done = write(&side, |s| wip::step(s, Step::ContinueRebase)).unwrap();
    assert!(matches!(done, Rebased::Done { autostash: Autostash::None, .. }));
    assert_eq!(state(&f), OperationState::None);
    assert_eq!(log(&f, "%s"), ["W3", "Reworded", "Second", "Root"]);
    assert_eq!(f.at(&f.worktree, &["log", "-1", "--format=%B", "HEAD"]), "W3", "no Conflicts: section in the continued commit");

    assert_eq!(Fixture::undo(&side).unwrap().toast, format!("Undid rebase from \"{}\"", short_sha(&old)));
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD"]), old, "Undo puts the old tip back");
}

#[test]
fn abort_restores_the_old_tip_and_pops_the_autostash_and_skip_leaves_the_commit_out() {
    let f = Fixture::copy();
    let commits = three_above(&f);
    let old = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    let wip = work_in_progress(&f);
    let side = f.side();
    assert_eq!(stopped(&f, &commits), Rebased::Conflicted);
    assert!(f.at(&f.worktree, &["stash", "list"]).contains(&rebase::autostash_name(&commits[0])), "the work is in GitMan's autostash meanwhile");
    let aborted = write(&side, |s| wip::step(s, Step::AbortRebase)).unwrap();
    assert_eq!(aborted, Rebased::Done { head: old.clone(), autostash: Autostash::Popped });
    assert_eq!(state(&f), OperationState::None);
    assert_eq!(f.at(&f.worktree, &["status", "--porcelain=v1", "--untracked-files=all"]), wip.replace("M  b.txt", " M b.txt"));
    assert!(!f.at(&f.worktree, &["stash", "list"]).contains(&rebase::autostash_name(&commits[0])));

    f.at(&f.worktree, &["checkout", "-q", "--", "."]);
    f.at(&f.worktree, &["clean", "-q", "-f"]);
    assert_eq!(stopped(&f, &commits), Rebased::Conflicted);
    assert!(matches!(write(&side, |s| wip::step(s, Step::SkipCommit)), Ok(Rebased::Done { .. })));
    assert_eq!(log(&f, "%s"), ["Reworded", "Second", "Root"], "the conflicting pick left out");
}

#[test]
fn a_failure_that_is_not_a_conflict_aborts_and_says_why() {
    let f = Fixture::copy();
    let [w1, ..] = three_above(&f);
    let old = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    let wip = work_in_progress(&f);
    hook(&f, "commit-msg", "#!/bin/sh\necho 'no ticket in the message' >&2\nexit 1\n");
    let side = f.side();
    let why = write(&side, |s| rebase::reword(s, &w1, "W1, reworded\n")).unwrap_err().to_string();
    assert!(why.contains("no ticket in the message"), "{why}");
    assert_eq!((state(&f), f.at(&f.worktree, &["rev-parse", "HEAD"])), (OperationState::None, old), "aborted: the old tip");
    assert_eq!(f.at(&f.worktree, &["status", "--porcelain=v1", "--untracked-files=all"]), wip.replace("M  b.txt", " M b.txt"), "the autostash popped");
}

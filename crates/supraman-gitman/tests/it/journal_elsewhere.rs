//! Undo and Redo of a merge into, or a rebase of, a branch other than HEAD's, on fixture copies only (C196).

use std::path::Path;

use supraman_gitman::fixture::{Fixture, run_git};
use supraman_gitman::write::branch::{self, Merged};
use supraman_gitman::write::journal::{self, Done, Kind, Refused, Undo};
use supraman_gitman::write::rebase::{self, Autostash, Rebased};
use supraman_gitman::write::{LockSet, Session, WriteError, Writer};
use supraman_gitman::Repo;

/// HEAD, every branch's tip and the worktree `wt`'s status and files.
#[derive(Debug, PartialEq, Eq)]
struct State {
    head: String,
    branches: String,
    status: String,
    files: Vec<(String, Vec<u8>)>,
}

fn state(f: &Fixture) -> State {
    let out = run_git(&f.git, &f.worktree, &["symbolic-ref", "-q", "HEAD"], &[], None);
    let head = format!("{} {}", String::from_utf8_lossy(&out.stdout).trim(), f.at(&f.worktree, &["rev-parse", "HEAD"]));
    let branches = f.at(&f.dir, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads"]);
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&f.worktree).unwrap().flatten().filter(|e| e.path().is_file()).map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect();
    files.sort();
    State { head, branches, status: f.at(&f.worktree, &["status", "--porcelain"]), files }
}

fn write<T>(repo: &Repo, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> T {
    Fixture::write_job(repo, locks, job).unwrap()
}

fn redo(repo: &Repo) -> Result<Done, Refused> {
    Fixture::write_job(repo, LockSet::undo(), |s| Ok(journal::redo(s, true))).unwrap()
}

fn undo_tooltip(repo: &Repo) -> String {
    journal::preview(&Writer::open(repo).unwrap()).unwrap().undo_tooltip()
}

fn tip(f: &Fixture, branch: &str) -> String {
    f.at(&f.dir, &["rev-parse", &format!("refs/heads/{branch}")])
}

fn commit_on(f: &Fixture, dir: &Path, path: &str, message: &str) {
    std::fs::write(dir.join(path), message.as_bytes()).unwrap();
    f.at(dir, &["add", path]);
    f.at(dir, &["commit", "-q", "--no-verify", "-m", message]);
}

/// `wt` on `side` with a change of its own, unstaged as a popped stash leaves it.
fn dirty_side(f: &Fixture) -> Repo {
    std::fs::write(f.worktree.join("b.txt"), b"mine\n").unwrap();
    f.side()
}

/// Undo puts `branch` back with HEAD, the files and the other branches as they were, then offers what it offered before (`earlier`); Redo moves it on again.
fn undone_and_redone(f: &Fixture, before: &State, earlier: &str, branch: &str, words: &str) {
    let wt = f.side();
    let after = state(f);
    assert_ne!(&after, before, "the write moved {branch}");
    assert_eq!(after.head, before.head, "HEAD's checkout is back");
    assert_eq!((after.status.as_str(), &after.files), (before.status.as_str(), &before.files), "the work in progress is back on side");
    assert_eq!(undo_tooltip(&wt), format!("Undo {words}"));
    assert_eq!(Fixture::undo(&wt).unwrap().toast, format!("Undid {words}"));
    assert_eq!(&state(f), before, "only {branch} went back");
    assert_eq!(undo_tooltip(&wt), earlier, "the checkouts it made are not Undo's next step");
    assert_eq!(redo(&wt).unwrap().toast, format!("Redid {words}"));
    assert_eq!(state(f), after, "redo moves {branch} on again");
    assert_eq!(Fixture::undo(&wt).unwrap().toast, format!("Undid {words}"));
    assert_eq!(&state(f), before);
}

fn merge_into_stale(f: &Fixture) {
    let wt = dirty_side(f);
    let (before, earlier) = (state(f), undo_tooltip(&wt));
    assert_eq!(write(&wt, LockSet::checkout(), |s| branch::merge(s, "feature", "stale")), Merged::Done(Autostash::Popped));
    undone_and_redone(f, &before, &earlier, "stale", "merge \"feature\" into \"stale\"");
}

#[test]
fn undo_of_a_merge_into_another_branch_puts_that_branch_back_and_never_checks_it_out() {
    merge_into_stale(&Fixture::copy());
}

#[test]
fn undo_of_a_merge_into_another_branch_on_reftable() {
    let Ok(f) = Fixture::reftable() else { return eprintln!("skipped: no reftable") };
    merge_into_stale(&f);
}

#[test]
fn undo_of_a_merge_into_another_branch_that_moved_since_is_refused_and_moves_nothing() {
    let f = Fixture::copy();
    let wt = dirty_side(&f);
    write(&wt, LockSet::checkout(), |s| branch::merge(s, "feature", "stale"));
    f.at(&f.dir, &["branch", "-f", "stale", "feature"]);
    let now = state(&f);
    let refused = Fixture::undo(&wt).unwrap_err();
    assert_eq!(refused.to_string(), "Cannot undo merge \"feature\" into \"stale\": stale moved since");
    assert_eq!(state(&f), now);

    f.at(&f.dir, &["branch", "-f", "stale", &f.at(&f.dir, &["rev-parse", "stale@{1}"])]);
    f.at(&f.dir, &["stash", "push", "-q", "--include-untracked"]);
    f.at(&f.dir, &["checkout", "-q", "stale"]);
    let refused = Fixture::undo(&wt).unwrap_err();
    assert_eq!(refused.to_string(), "Cannot undo merge \"feature\" into \"stale\": stale is checked out");
}

/// A branch `d` off `side` with a commit of its own, `side` dirty again after it.
fn branch_d(f: &Fixture) -> Repo {
    f.at(&f.worktree, &["checkout", "-q", "-b", "d"]);
    commit_on(f, &f.worktree, "d.txt", "D's own");
    f.at(&f.worktree, &["checkout", "-q", "side"]);
    dirty_side(f)
}

fn rebase_d(f: &Fixture) {
    let wt = branch_d(f);
    let (before, earlier) = (state(f), undo_tooltip(&wt));
    let Rebased::Done { head, autostash } = write(&wt, LockSet::rebase(), |s| rebase::onto(s, "d", "feature")) else { panic!("done") };
    assert_eq!((head, autostash), (tip(f, "side"), Autostash::Popped), "side is checked out again, its changes popped there");
    assert_eq!(f.at(&f.dir, &["rev-parse", "d^"]), tip(f, "feature"), "d is on feature");
    undone_and_redone(f, &before, &earlier, "d", "rebase \"d\" onto \"feature\"");
}

#[test]
fn undo_of_a_rebase_of_another_branch_puts_that_branch_back_and_keeps_heads_checkout() {
    rebase_d(&Fixture::copy());
}

#[test]
fn undo_of_a_rebase_of_another_branch_on_reftable() {
    let Ok(f) = Fixture::reftable() else { return eprintln!("skipped: no reftable") };
    rebase_d(&f);
}

#[test]
fn a_rebase_of_another_branch_that_conflicts_waits_on_it_and_abort_goes_back() {
    let f = Fixture::copy();
    f.at(&f.worktree, &["checkout", "-q", "-b", "d"]);
    std::fs::write(f.worktree.join("a.txt"), b"d\n").unwrap();
    f.at(&f.worktree, &["commit", "-q", "--no-verify", "-am", "D's a.txt"]);
    f.at(&f.worktree, &["checkout", "-q", "side"]);
    commit_on(&f, &f.worktree, "a.txt", "side's a.txt");
    let wt = dirty_side(&f);
    let before = state(&f);
    assert_eq!(write(&wt, LockSet::rebase(), |s| rebase::onto(s, "d", "side")), Rebased::Conflicted);
    let Rebased::Done { autostash, .. } = write(&wt, LockSet::rebase(), rebase::abort) else { panic!("aborted") };
    assert_eq!(autostash, Autostash::Popped);
    assert_eq!(state(&f), before, "side checked out again with its changes");
    let undo = journal::preview(&Writer::open(&wt).unwrap()).unwrap().undo;
    assert!(!matches!(undo, Some(Undo::Step(ref step)) if step.kind == Kind::Elsewhere), "nothing recorded: {undo:?}");
}

#[test]
fn a_rebase_of_another_branch_continued_after_its_conflict_goes_back_and_is_undone() {
    let f = Fixture::copy();
    f.at(&f.worktree, &["checkout", "-q", "-b", "d"]);
    std::fs::write(f.worktree.join("a.txt"), b"d\n").unwrap();
    f.at(&f.worktree, &["commit", "-q", "--no-verify", "-am", "D's a.txt"]);
    f.at(&f.worktree, &["checkout", "-q", "side"]);
    commit_on(&f, &f.worktree, "a.txt", "side's a.txt");
    let wt = dirty_side(&f);
    let (before, from) = (state(&f), tip(&f, "d"));
    assert_eq!(write(&wt, LockSet::rebase(), |s| rebase::onto(s, "d", "side")), Rebased::Conflicted);
    std::fs::write(f.worktree.join("a.txt"), b"both\n").unwrap();
    f.at(&f.worktree, &["add", "a.txt"]);
    let Rebased::Done { head, autostash } = write(&wt, LockSet::rebase(), rebase::continue_rebase) else { panic!("continued") };
    assert_eq!((head, autostash), (tip(&f, "side"), Autostash::Popped));
    assert_eq!((state(&f).head, std::fs::read(f.worktree.join("b.txt")).unwrap()), (before.head.clone(), b"mine\n".to_vec()), "side checked out again with its changes");
    assert_eq!(Fixture::undo(&wt).unwrap().toast, "Undid rebase \"d\" onto \"side\"");
    assert_eq!((tip(&f, "d"), state(&f)), (from, before));
}

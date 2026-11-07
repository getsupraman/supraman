//! Checkout, branches, tags, fast-forward, merge, rebase onto and the worktrees' writes, on fixture copies and
//! their bare temp remote only (C196).

use std::path::Path;

use supraman_gitman::fixture::{Fixture, git_in, run_git};
use supraman_gitman::write::branch::{self, CheckedOut, Merged, Plan, Target};
use supraman_gitman::write::journal::{self, Done, Refused};
use supraman_gitman::write::rebase::{self, Autostash, Rebased};
use supraman_gitman::write::{LockSet, Session, WriteError, Writer};
use supraman_gitman::*;

fn git(f: &Fixture, dir: &Path, args: &[&str]) -> String {
    git_in(&f.git, dir, args, &[], None)
}

fn side(f: &Fixture) -> Repo {
    Repo::new(f.git.clone(), &f.worktree)
}

fn write<T>(repo: &Repo, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Writer::open(repo).unwrap().run(LockSet::checkout(), job)
}

fn undo(repo: &Repo) -> Result<Done, Refused> {
    Writer::open(repo).unwrap().run(LockSet::undo(), |s| Ok(journal::undo(s, true))).unwrap()
}

fn refused<T: std::fmt::Debug>(r: Result<T, WriteError>) -> &'static str {
    match r {
        Err(WriteError::Refused(why)) => why,
        other => panic!("refused, not {other:?}"),
    }
}

fn plan(repo: &Repo, target: Target) -> Plan {
    let refs = refs(repo).unwrap();
    let top = Writer::open(repo).unwrap().dirs().top.clone();
    branch::plan(repo, &refs, &top, &target).unwrap()
}

fn head(f: &Fixture, dir: &Path) -> String {
    let out = run_git(&f.git, dir, &["symbolic-ref", "-q", "--short", "HEAD"], &[], None);
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() { git(f, dir, &["rev-parse", "HEAD"]) } else { name }
}

fn sha(f: &Fixture, name: &str) -> String {
    git(f, &f.dir, &["rev-parse", name])
}

fn stash_count(f: &Fixture) -> usize {
    stashes(&f.repo()).unwrap().len()
}

/// `origin/<name>` at `at`, fetched.
fn remote_branch(f: &Fixture, name: &str, at: &str) {
    git(f, &f.dir, &["push", "-q", "origin", &format!("{at}:refs/heads/{name}")]);
    git(f, &f.dir, &["fetch", "-q", "origin"]);
}

#[test]
fn a_checkout_is_planned_from_the_ref_and_where_its_branch_lives() {
    let f = Fixture::copy();
    let (main, wt) = (f.repo(), side(&f));
    remote_branch(&f, "topic", f.sha("rename"));
    remote_branch(&f, "next", f.sha("merge"));
    git(&f, &f.dir, &["branch", "-q", "next", f.sha("main_work")]);
    git(&f, &f.dir, &["branch", "-q", "-u", "origin/next", "next"]);
    git(&f, &f.dir, &["branch", "-q", "apart", f.sha("root")]);
    remote_branch(&f, "apart", f.sha("rename"));

    assert_eq!(plan(&main, Target::Branch("main".into())), Plan::Head);
    assert_eq!(plan(&main, Target::Branch("feature".into())), Plan::Branch("feature".into()));
    assert!(matches!(plan(&main, Target::Branch("side".into())), Plan::Worktree(p) if supraman_core::paths::path_key(&p) == supraman_core::paths::path_key(&f.worktree)));
    assert!(matches!(plan(&wt, Target::Remote("origin/main".into())), Plan::Worktree(p) if supraman_core::paths::path_key(&p) == supraman_core::paths::path_key(&f.dir)), "a remote whose local lives in another worktree opens it");
    assert_eq!(plan(&main, Target::Remote("origin/topic".into())), Plan::Track { remote: "origin/topic".into(), name: "topic".into() });
    assert_eq!(plan(&main, Target::Remote("origin/next".into())), Plan::Forward { name: "next".into(), remote: "origin/next".into(), sha: f.sha("merge").into() }, "a tracking local behind its remote moves to it");
    assert_eq!(plan(&main, Target::Remote("origin/apart".into())), Plan::Clash { name: "apart".into(), remote: "origin/apart".into(), sha: f.sha("rename").into() }, "a local that went apart asks");
    assert_eq!(plan(&main, Target::Commit(f.sha("root").into())), Plan::Detached(f.sha("root").into()));
}

#[test]
fn a_checkout_is_undoable_and_a_refused_one_offers_the_stash_which_is_put_back_or_kept_and_named() {
    let f = Fixture::copy();
    let wt = side(&f);
    assert_eq!(write(&wt, |s| branch::checkout(s, &Plan::Branch("feature".into()), false)).unwrap(), CheckedOut::Done(Autostash::None));
    assert_eq!(head(&f, &f.worktree), "feature");
    undo(&wt).unwrap();
    assert_eq!(head(&f, &f.worktree), "side", "Undo checks the branch before out again");

    std::fs::write(f.worktree.join("b.txt"), b"b\nmine\n").unwrap();
    let stashes = stash_count(&f);
    assert_eq!(write(&wt, |s| branch::checkout(s, &Plan::Branch("stale".into()), false)).unwrap(), CheckedOut::WouldOverwrite { files: 1 });
    assert_eq!(head(&f, &f.worktree), "side", "a refused checkout changes nothing");
    assert_eq!(stash_count(&f), stashes);
    let CheckedOut::Done(Autostash::Kept { name, .. }) = write(&wt, |s| branch::checkout(s, &Plan::Branch("stale".into()), true)).unwrap() else { panic!("the pop conflicts") };
    assert_eq!(head(&f, &f.worktree), "stale");
    assert_eq!(name, "GitMan: before checking out stale");
    assert_eq!(stash_count(&f), stashes + 1, "a pop that conflicts keeps the stash");
}

#[test]
fn a_detached_checkout_and_a_forward_stash_the_work_and_pop_it_back() {
    let f = Fixture::copy();
    let wt = side(&f);
    std::fs::write(f.worktree.join("old.txt"), b"mine\n").unwrap();
    let stashes = stash_count(&f);
    let root = f.sha("root").to_string();
    assert_eq!(write(&wt, |s| branch::checkout(s, &Plan::Detached(root.clone()), false)).unwrap(), CheckedOut::Done(Autostash::Popped));
    assert_eq!(head(&f, &f.worktree), root);
    assert_eq!(std::fs::read_to_string(f.worktree.join("old.txt")).unwrap(), "mine\n", "the work is back");
    assert_eq!(stash_count(&f), stashes, "never a stranded stash");

    remote_branch(&f, "next", f.sha("merge"));
    git(&f, &f.dir, &["branch", "-q", "next", f.sha("second")]);
    git(&f, &f.dir, &["branch", "-q", "-u", "origin/next", "next"]);
    let forward = plan(&wt, Target::Remote("origin/next".into()));
    assert!(matches!(forward, Plan::Forward { .. }));
    assert_eq!(write(&wt, |s| branch::checkout(s, &forward, false)).unwrap(), CheckedOut::Done(Autostash::Popped));
    assert_eq!((head(&f, &f.worktree), sha(&f, "next")), ("next".to_string(), f.sha("merge").to_string()));
    assert_eq!(stash_count(&f), stashes);
}

#[test]
fn branches_are_made_renamed_and_deleted_with_their_names_checked() {
    let f = Fixture::copy();
    let main = f.repo();
    assert_eq!(refused(write(&main, |s| branch::create_branch(s, "bad name", "HEAD"))), "A name cannot hold a space");
    assert_eq!(refused(write(&main, |s| branch::create_branch(s, "feature", "HEAD"))), branch::BRANCH_EXISTS);
    write(&main, |s| branch::create_branch(s, "topic/new", f.sha("second"))).unwrap();
    assert_eq!(sha(&f, "topic/new"), f.sha("second"));
    write(&main, |s| branch::rename_branch(s, "topic/new", "topic/renamed")).unwrap();
    assert!(run_git(&f.git, &f.dir, &["rev-parse", "-q", "--verify", "refs/heads/topic/new"], &[], None).code != Some(0));
    assert!(!branch::only_here(&main, "topic/renamed").unwrap(), "second is on other branches");

    write(&main, |s| branch::create_branch(s, "lonely", "HEAD")).unwrap();
    git(&f, &f.dir, &["commit", "-q", "--no-verify", "--allow-empty", "-m", "Only on main"]);
    git(&f, &f.dir, &["branch", "-q", "-f", "lonely", "HEAD"]);
    git(&f, &f.dir, &["reset", "-q", "--soft", "HEAD~"]);
    assert!(branch::only_here(&main, "lonely").unwrap(), "its commit is on no other branch and it has no upstream");
    assert!(!branch::only_here(&main, "stale").unwrap(), "stale has an upstream");

    assert_eq!(refused(write(&main, |s| branch::delete_branch(s, "main"))), branch::DELETE_CURRENT);
    let lonely = sha(&f, "lonely");
    write(&main, |s| branch::delete_branch(s, "lonely")).unwrap();
    assert!(run_git(&f.git, &f.dir, &["rev-parse", "-q", "--verify", "refs/heads/lonely"], &[], None).code != Some(0));
    undo(&main).unwrap();
    assert_eq!(sha(&f, "lonely"), lonely, "a local delete is undoable");
}

#[test]
fn tags_are_made_light_or_annotated_and_deleted() {
    let f = Fixture::copy();
    let main = f.repo();
    write(&main, |s| branch::create_tag(s, "t-light", f.sha("root"), None, false)).unwrap();
    write(&main, |s| branch::create_tag(s, "t-note", f.sha("root"), Some("A note\n"), false)).unwrap();
    assert_eq!(git(&f, &f.dir, &["cat-file", "-t", "t-light"]), "commit");
    assert_eq!(git(&f, &f.dir, &["cat-file", "-t", "t-note"]), "tag");
    assert_eq!(git(&f, &f.dir, &["tag", "-l", "--format=%(contents:subject)", "t-note"]), "A note");
    assert_eq!(refused(write(&main, |s| branch::create_tag(s, "v1", "HEAD", None, false))), branch::TAG_EXISTS);
    write(&main, |s| branch::create_tag(s, "t-light", f.sha("root"), Some("Annotated now"), true)).unwrap();
    assert_eq!(git(&f, &f.dir, &["cat-file", "-t", "t-light"]), "tag", "annotate re-makes it annotated");
    write(&main, |s| branch::delete_tag(s, "t-light")).unwrap();
    assert!(git(&f, &f.dir, &["tag", "-l", "t-light"]).is_empty());
}

#[test]
fn another_branch_fast_forwards_without_a_checkout() {
    let f = Fixture::copy();
    let main = f.repo();
    let status = git(&f, &f.dir, &["status", "--porcelain"]);
    let root = f.sha("root").to_string();
    write(&main, |s| branch::fast_forward(s, "stale", "origin/main")).unwrap();
    assert_eq!(sha(&f, "stale"), sha(&f, "origin/main"));
    assert_eq!(head(&f, &f.dir), "main", "nothing checked out");
    assert_eq!(git(&f, &f.dir, &["status", "--porcelain"]), status, "the work in progress untouched");
    assert_eq!(refused(write(&main, |s| branch::fast_forward(s, "feature", &root))), branch::NOT_FAST_FORWARD);
    assert_eq!(refused(write(&main, |s| branch::fast_forward(s, "side", "main"))), branch::IN_WORKTREE);

    let wt = side(&f);
    write(&wt, |s| branch::fast_forward(s, "side", "feature")).unwrap();
    assert_eq!(sha(&f, "side"), f.sha("rename"), "HEAD's branch by merge --ff-only");
}

fn commit_on(f: &Fixture, dir: &Path, path: &str, bytes: &[u8], message: &str) {
    std::fs::write(dir.join(path), bytes).unwrap();
    git(f, dir, &["add", path]);
    git(f, dir, &["commit", "-q", "--no-verify", "-m", message]);
}

#[test]
fn a_merge_into_head_or_into_another_branch_goes_back_and_pops() {
    let f = Fixture::copy();
    let wt = side(&f);
    let root = f.sha("root").to_string();
    assert_eq!(write(&wt, |s| branch::merge(s, &root, "side")).unwrap(), Merged::UpToDate);
    assert_eq!(write(&wt, |s| branch::merge(s, "feature", "side")).unwrap(), Merged::Done(Autostash::None));
    assert_eq!(git(&f, &f.worktree, &["rev-list", "--count", "--merges", "-1", "HEAD"]), "1", "a merge commit");

    std::fs::write(f.worktree.join("c.txt"), b"mine\n").unwrap();
    git(&f, &f.worktree, &["add", "c.txt"]);
    let before = sha(&f, "stale");
    assert_eq!(write(&wt, |s| branch::merge(s, "feature", "stale")).unwrap(), Merged::Done(Autostash::Popped));
    assert_eq!(head(&f, &f.worktree), "side", "the checkout before is back");
    assert_eq!(git(&f, &f.dir, &["rev-parse", "stale^1"]), before, "stale got the merge");
    assert_eq!(std::fs::read_to_string(f.worktree.join("c.txt")).unwrap(), "mine\n");
}

#[test]
fn a_conflicted_merge_into_another_branch_waits_or_is_abandoned_back() {
    let f = Fixture::copy();
    let wt = side(&f);
    git(&f, &f.worktree, &["checkout", "-q", "-b", "c1"]);
    commit_on(&f, &f.worktree, "a.txt", b"one\nc1\n", "C1");
    git(&f, &f.worktree, &["checkout", "-q", "-b", "c2", "side"]);
    commit_on(&f, &f.worktree, "a.txt", b"one\nc2\n", "C2");
    git(&f, &f.worktree, &["checkout", "-q", "side"]);
    std::fs::write(f.worktree.join("old.txt"), b"mine\n").unwrap();
    let stashes = stash_count(&f);
    let Merged::Conflicted { away: Some(away) } = write(&wt, |s| branch::merge(s, "c1", "c2")).unwrap() else { panic!("conflicts") };
    assert_eq!(head(&f, &f.worktree), "c2", "it waits on the branch merged into");
    assert_eq!(away.back, "side");
    assert_eq!(operation_state(&f.dir.join(".git/worktrees/wt")), OperationState::Merge);
    assert_eq!(write(&wt, |s| branch::abandon(s, &away)).unwrap(), Autostash::Popped);
    assert_eq!(head(&f, &f.worktree), "side");
    assert_eq!(operation_state(&f.dir.join(".git/worktrees/wt")), OperationState::None);
    assert_eq!(std::fs::read_to_string(f.worktree.join("old.txt")).unwrap(), "mine\n");
    assert_eq!(stash_count(&f), stashes);
}

#[test]
fn heads_branch_rebases_onto_a_ref_undoably() {
    let f = Fixture::copy();
    let wt = side(&f);
    commit_on(&f, &f.worktree, "c.txt", b"c\n", "Side's own");
    let before = sha(&f, "side");
    let Rebased::Done { head: new, .. } = write(&wt, |s| rebase::onto(s, "side", "feature")).unwrap() else { panic!("done") };
    assert_eq!(git(&f, &f.worktree, &["rev-parse", "HEAD^"]), f.sha("rename"));
    assert_eq!(sha(&f, "side"), new);
    undo(&wt).unwrap();
    assert_eq!(sha(&f, "side"), before);
}

#[test]
fn a_worktree_is_locked_unlocked_removed_with_its_branch_and_pruned() {
    let f = Fixture::copy();
    let main = f.repo();
    let extra = f.dir.parent().unwrap().join("extra");
    git(&f, &f.dir, &["worktree", "add", "-q", "-b", "extra", &extra.to_string_lossy(), f.sha("root")]);
    let locked = || refs(&main).unwrap().worktrees.iter().find(|w| w.branch.as_deref() == Some("extra")).map(|w| w.locked);
    write(&main, |s| branch::lock_worktree(s, &extra, true)).unwrap();
    assert_eq!(locked(), Some(true));
    write(&main, |s| branch::lock_worktree(s, &extra, false)).unwrap();
    assert_eq!(locked(), Some(false));
    std::fs::write(extra.join("dirty.txt"), b"x\n").unwrap();
    write(&main, |s| branch::remove_worktree(s, &extra, Some("extra"))).unwrap();
    assert!(!extra.exists() && locked().is_none());
    assert!(run_git(&f.git, &f.dir, &["rev-parse", "-q", "--verify", "refs/heads/extra"], &[], None).code != Some(0));

    std::fs::remove_dir_all(&f.worktree).unwrap();
    assert!(refs(&main).unwrap().worktrees.iter().any(|w| w.prunable));
    write(&main, |s| branch::prune_worktrees(s)).unwrap();
    assert_eq!(refs(&main).unwrap().worktrees.len(), 1);
}

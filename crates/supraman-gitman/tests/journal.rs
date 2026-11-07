//! Undo and redo on fixture copies only (C196): each restores the refs, the index and the working files exactly.

use std::collections::BTreeMap;
use std::path::Path;

use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::write::journal::{self, Action, Done, Kind, Preview, Refused, Undo};
use supraman_gitman::write::{Call, LockSet, Session, WriteError, Writer};
use supraman_gitman::*;

/// The refs (but the journal's), HEAD, the index and every working file of the checkout at `dir`.
#[derive(Debug, PartialEq, Eq)]
struct State {
    head: String,
    refs: String,
    config: String,
    index: String,
    files: BTreeMap<String, Vec<u8>>,
}

fn files(top: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if entry.file_name() == ".git" {
            continue;
        }
        if path.is_dir() {
            files(top, &path, into);
        } else {
            let rel = path.strip_prefix(top).unwrap().to_string_lossy().replace('\\', "/");
            into.insert(rel, std::fs::read(&path).unwrap());
        }
    }
}

fn state(f: &Fixture) -> State {
    let git = |args: &[&str]| git_in(&f.git, &f.dir, args, &[], None);
    let head = format!("{} {}", git(&["rev-parse", "HEAD"]), run(f, &["symbolic-ref", "-q", "HEAD"]));
    let refs = git(&["for-each-ref", "--format=%(refname) %(objectname)"]).lines().filter(|l| !l.starts_with("refs/worktree/") && !l.starts_with("refs/stash")).collect::<Vec<_>>().join("\n");
    let config = git(&["config", "--local", "--get-regexp", "^branch\\."]);
    let mut all = BTreeMap::new();
    files(&f.dir, &f.dir, &mut all);
    State { head, refs, config, index: git(&["ls-files", "-s"]), files: all }
}

fn run(f: &Fixture, args: &[&str]) -> String {
    let out = supraman_gitman::fixture::run_git(&f.git, &f.dir, args, &[], None);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write<T>(f: &Fixture, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> T {
    Writer::open(&f.repo()).unwrap().run(LockSet::undo(), job).unwrap()
}

fn undo(f: &Fixture) -> Result<Done, Refused> {
    Writer::open(&f.repo()).unwrap().run(LockSet::undo(), |s| Ok(journal::undo(s, true))).unwrap()
}

fn redo(f: &Fixture) -> Result<Done, Refused> {
    Writer::open(&f.repo()).unwrap().run(LockSet::undo(), |s| Ok(journal::redo(s, true))).unwrap()
}

fn preview(f: &Fixture) -> Preview {
    journal::preview(&Writer::open(&f.repo()).unwrap()).unwrap()
}

/// `act` then Undo gives back the state before it, Redo the state after it, and Undo again the state before.
fn round_trip(f: &Fixture, act: impl FnOnce(&Session<'_>) -> Result<(), WriteError>, undid: &str) {
    let before = state(f);
    write(f, act);
    let after = state(f);
    assert_ne!(before, after, "the action changed something");
    let said = |done: Result<Done, Refused>, verb: &str| {
        let toast = done.unwrap().toast;
        assert!(toast == format!("{verb} {undid}") || toast == format!("{verb} {undid}; your changes are in a stash"), "{toast}");
    };
    said(undo(f), "Undid");
    assert_eq!(state(f), before, "undo restores the state before");
    assert_eq!(preview(f).redo.map(|s| s.action), Some(undid.to_string()));
    said(redo(f), "Redid");
    assert_eq!(state(f), after, "redo restores the state after");
    said(undo(f), "Undid");
    assert_eq!(state(f), before, "undo after redo");
}

fn git(s: &Session, args: &[&str]) -> Result<(), WriteError> {
    s.output(&Call::new(args)).map(drop)
}

#[test]
fn undo_and_redo_of_a_commit_and_an_amend_keep_the_index_and_files() {
    let f = Fixture::copy();
    round_trip(&f, |s| {
        git(s, &["commit", "-q", "--no-verify", "-m", "Staged work"])?;
        journal::record(s, Action::Commit, None)
    }, "commit \"Staged work\"");
    round_trip(&f, |s| {
        git(s, &["commit", "-q", "--no-verify", "--amend", "-m", "Ahead, amended"])?;
        journal::record(s, Action::Amend, None)
    }, "commit amend \"Ahead, amended\"");
}

#[test]
fn a_terminal_commit_is_undone_too() {
    let f = Fixture::copy();
    let before = state(&f);
    git_in(&f.git, &f.dir, &["commit", "-q", "--no-verify", "-m", "From a shell"], &[], None);
    assert_eq!(undo(&f).unwrap().toast, "Undid commit \"From a shell\"");
    assert_eq!(state(&f), before);
}

#[test]
fn undo_and_redo_of_a_checkout() {
    let f = Fixture::copy();
    git_in(&f.git, &f.dir, &["stash", "push", "-q", "--include-untracked"], &[], None);
    round_trip(&f, |s| {
        git(s, &["checkout", "-q", "stale"])?;
        journal::record(s, Action::Checkout, None)
    }, "checkout from \"main\" to \"stale\"");
}

#[test]
fn undo_of_a_checkout_that_would_overwrite_local_changes_is_refused_with_their_count() {
    let f = Fixture::copy();
    git_in(&f.git, &f.dir, &["stash", "push", "-q", "--include-untracked"], &[], None);
    write(&f, |s| {
        git(s, &["checkout", "-q", "stale"])?;
        journal::record(s, Action::Checkout, None)
    });
    f.write("a.txt", b"changed on stale\n");
    let refused = undo(&f).unwrap_err();
    assert!(matches!(refused, Refused::Conflicts { ref target, files: 1 } if target == "main"), "{refused}");
    assert_eq!(run(&f, &["symbolic-ref", "HEAD"]), "refs/heads/stale", "nothing moved");
}

#[test]
fn undo_of_a_hard_reset_restores_its_snapshot_and_redo_resets_again() {
    let f = Fixture::copy();
    let paths: Vec<String> = status(&f.repo()).unwrap().entries.iter().flat_map(|e| [Some(e.path.clone()), e.orig_path.clone()]).flatten().collect();
    round_trip(&f, |s| {
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let snapshot = s.snapshot(&refs, "Before reset")?;
        git(s, &["reset", "-q", "--hard", "HEAD~1"])?;
        journal::record(s, Action::Reset { hard: true }, Some(&snapshot))
    }, "reset to \"Merge feature\"");
    assert!(run(&f, &["stash", "list"]).lines().any(|l| l.contains("GitMan: before")), "the work in progress went to a stash before the redo");
}

#[test]
fn undo_of_a_hard_reset_asks_before_it_stashes() {
    let f = Fixture::copy();
    write(&f, |s| {
        let snapshot = s.snapshot(&["a.txt", "u.txt"], "Before reset")?;
        git(s, &["reset", "-q", "--hard", "HEAD~1"])?;
        journal::record(s, Action::Reset { hard: true }, Some(&snapshot))
    });
    f.write("a.txt", b"edited since\n");
    let refused = Writer::open(&f.repo()).unwrap().run(LockSet::undo(), |s| Ok(journal::undo(s, false))).unwrap().unwrap_err();
    assert!(matches!(refused, Refused::WouldStash { files: 2, .. }), "{refused}");
    assert_eq!(std::fs::read(f.dir.join("a.txt")).unwrap(), b"edited since\n", "nothing changed before the answer");
}

#[test]
fn a_soft_reset_only_moves_the_ref() {
    let f = Fixture::copy();
    round_trip(&f, |s| {
        git(s, &["reset", "-q", "--soft", "HEAD~1"])?;
        journal::record(s, Action::Reset { hard: false }, None)
    }, "reset to \"Merge feature\"");
}

#[test]
fn undo_and_redo_of_a_discard_with_its_snapshot() {
    let f = Fixture::copy();
    let paths: Vec<String> = ["a.txt", "b.txt", "u.txt", "s.txt", "new.txt", "renamed.txt"].map(String::from).to_vec();
    round_trip(&f, |s| journal::discard(s, &paths), "discard \"a.txt\" (and 5 other files)");
}

#[test]
fn undo_of_a_discard_stashes_what_changed_in_its_paths_since() {
    let f = Fixture::copy();
    let before = state(&f);
    write(&f, |s| journal::discard(s, &["a.txt".to_string()]));
    f.write("a.txt", b"after the discard\n");
    assert_eq!(undo(&f).unwrap().toast, "Undid discard \"a.txt\"; your changes are in a stash");
    assert_eq!(state(&f), before);
    git_in(&f.git, &f.dir, &["checkout", "-q", "stash@{0}", "--", "a.txt"], &[], None);
    assert_eq!(std::fs::read(f.dir.join("a.txt")).unwrap(), b"after the discard\n");
}

#[test]
fn undo_and_redo_of_a_branch_delete_with_its_upstream() {
    let f = Fixture::copy();
    round_trip(&f, |s| journal::delete_branch(s, "stale"), "delete branch \"stale\"");
    assert_eq!(run(&f, &["config", "branch.stale.merge"]), "refs/heads/main", "its upstream again");
    write(&f, |s| git(s, &["tag", "stale", "main"]));
    assert!(matches!(redo(&f), Err(Refused::Stale { .. })), "a tag has the branch's name");
    assert!(!run(&f, &["rev-parse", "-q", "--verify", "refs/heads/stale"]).is_empty());
}

#[test]
fn undo_goes_back_several_steps_and_a_new_action_clears_redo() {
    let f = Fixture::copy();
    let start = state(&f);
    write(&f, |s| {
        git(s, &["commit", "-q", "--no-verify", "-m", "One"])?;
        journal::record(s, Action::Commit, None)?;
        journal::discard(s, &["u.txt".to_string()])?;
        journal::delete_branch(s, "stale")
    });
    for undid in ["delete branch \"stale\"", "discard \"u.txt\"", "commit \"One\""] {
        assert_eq!(undo(&f).unwrap().toast, format!("Undid {undid}"));
    }
    assert_eq!(state(&f), start);
    assert_eq!(preview(&f).redo.map(|s| s.action).as_deref(), Some("commit \"One\""));
    write(&f, |s| journal::delete_branch(s, "stale"));
    assert_eq!(preview(&f).redo, None, "a new action clears redo");
}

#[test]
fn an_entry_gitman_cannot_undo_stops_the_walk_and_says_what_can_be() {
    let f = Fixture::copy();
    git_in(&f.git, &f.dir, &["stash", "push", "-q", "--include-untracked"], &[], None);
    for args in [&["checkout", "-q", "-b", "topic", "HEAD~1"][..], &["commit", "-q", "--allow-empty", "-m", "Topic"], &["checkout", "-q", "main"], &["merge", "-q", "--no-ff", "--no-verify", "-m", "Merge topic", "topic"]] {
        git_in(&f.git, &f.dir, args, &[], None);
    }
    let p = preview(&f);
    let Some(Undo::Cannot(message)) = &p.undo else { panic!("{p:?}") };
    assert!(message.starts_with("merge topic"), "{message}");
    assert_eq!(p.undo_tooltip(), format!("Cannot undo {message}: only commits, checkouts, resets, discards, branch deletes and GitMan's rebases can be undone"));
    assert!(!p.can_undo());
    assert!(matches!(undo(&f), Err(Refused::Cannot(_))));
    write(&f, |s| journal::discard(s, &["a.txt".to_string()]));
    assert_eq!(preview(&f).undo, Some(Undo::Step(journal::Step { kind: Kind::Discard, action: "discard \"a.txt\"".into() })));
}

#[test]
fn nothing_to_undo_on_a_fresh_repo_and_undo_waits_out_a_merge() {
    let dir = tempfile::tempdir().unwrap();
    let system = supraman_gitman::fixture::system_git();
    git_in(&system, dir.path(), &["init", "-q", "-b", "main", "r"], &[], None);
    let p = journal::preview(&Writer::open(&Repo::new(system, dir.path().join("r"))).unwrap()).unwrap();
    assert_eq!((p.undo_tooltip(), p.redo_tooltip()), ("Nothing to undo".to_string(), "No recent undo to redo".to_string()));

    let f = Fixture::copy();
    let wt = Repo::new(f.git.clone(), &f.worktree);
    Writer::open(&wt).unwrap().run(LockSet::undo(), |s| {
        git(s, &["commit", "-q", "--no-verify", "--allow-empty", "-m", "Side"])?;
        journal::record(s, Action::Commit, None)
    }).unwrap();
    assert!(journal::preview(&Writer::open(&wt).unwrap()).unwrap().can_undo());
    f.conflict();
    let p = journal::preview(&Writer::open(&wt).unwrap()).unwrap();
    assert_eq!((p.can_undo(), p.undo_tooltip(), p.redo_tooltip()), (false, "Cannot undo during a merge".to_string(), "Cannot redo during a merge".to_string()));
    assert!(matches!(Writer::open(&wt).unwrap().run(LockSet::undo(), |s| Ok(journal::undo(s, true))).unwrap(), Err(Refused::Busy("a merge"))));
}

#[test]
fn gitmans_rebase_is_undone_as_a_hard_reset() {
    let f = Fixture::copy();
    git_in(&f.git, &f.dir, &["stash", "push", "-q", "--include-untracked"], &[], None);
    git_in(&f.git, &f.dir, &["checkout", "-q", "-b", "topic", f.sha("second")], &[], None);
    git_in(&f.git, &f.dir, &["commit", "-q", "--allow-empty", "-m", "Topic"], &[], None);
    let before = state(&f);
    let from = run(&f, &["rev-parse", "HEAD"]);
    write(&f, |s| {
        git(s, &["rebase", "-q", "--no-verify", "main"])?;
        journal::record(s, Action::Rebase { from: from.clone() }, None)
    });
    assert_eq!(undo(&f).unwrap().toast, format!("Undid rebase from \"{}\"", short_sha(&from)));
    assert_eq!(state(&f), before);
}

#[test]
fn a_snapshot_outlives_gc_while_its_journal_entry_does_and_is_gone_after() {
    let f = Fixture::copy();
    write(&f, |s| journal::discard(s, &["a.txt".to_string()]));
    let marker = run(&f, &["rev-parse", "refs/worktree/supraman/undo@{1}"]);
    let snapshot = run(&f, &["rev-parse", &format!("{marker}^")]);
    git_in(&f.git, &f.dir, &["gc", "-q", "--prune=now"], &[], None);
    assert_eq!(run(&f, &["cat-file", "-t", &snapshot]), "commit", "the journal's reflog keeps the snapshot");
    assert_eq!(run(&f, &["cat-file", "-t", &format!("{snapshot}^2")]), "commit", "and its index");
    git_in(&f.git, &f.dir, &["reflog", "expire", "--expire=now", "--expire-unreachable=now", "refs/worktree/supraman/undo"], &[], None);
    git_in(&f.git, &f.dir, &["gc", "-q", "--prune=now"], &[], None);
    assert!(run(&f, &["cat-file", "-t", &snapshot]).is_empty(), "pruned once its entry expired");
}

#[test]
fn undo_says_when_a_snapshot_is_gone() {
    let f = Fixture::copy();
    write(&f, |s| journal::discard(s, &["a.txt".to_string()]));
    let marker = run(&f, &["rev-parse", "refs/worktree/supraman/undo@{1}"]);
    let snapshot = run(&f, &["rev-parse", &format!("{marker}^")]);
    let object = f.dir.join(".git/objects").join(&snapshot[..2]).join(&snapshot[2..]);
    std::fs::remove_file(object).unwrap();
    let refused = undo(&f).unwrap_err();
    assert!(matches!(refused, Refused::Gone(_)), "{refused}");
    assert!(refused.to_string().contains("probably garbage-collected"), "{refused}");
}

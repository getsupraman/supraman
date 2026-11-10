//! The write layer on fixture copies only (C196).

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use supraman_gitman::fixture::{Fixture, git_in, run_git};
use supraman_gitman::write::{Call, LockSet, PINNED, Resource, Writer, refresh_lock};
use supraman_gitman::*;

const WAIT: Duration = Duration::from_secs(60);

#[test]
fn two_writes_never_overlap_and_a_read_waits_for_an_index_write() {
    let f = Fixture::copy();
    let writer = Writer::open(&f.repo()).unwrap();
    let in_worktree = Writer::open(&Repo::new(f.git.clone(), &f.worktree)).unwrap();
    assert!(Arc::ptr_eq(writer.scheduler(), in_worktree.scheduler()), "one scheduler per repository");
    let held = writer.scheduler().acquire(LockSet::commit());
    let (tx, rx) = mpsc::channel();
    let write = {
        let (writer, tx) = (writer.clone(), tx.clone());
        thread::spawn(move || {
            writer
                .run(LockSet::reset(), |session| {
                    tx.send("write starts").unwrap();
                    session.output(&Call::new(&["add"]).paths(["u.txt"]))?;
                    tx.send("write ends").unwrap();
                    Ok(())
                })
                .unwrap();
        })
    };
    assert!(writer.scheduler().await_waiting(1, WAIT));
    let read = {
        let repo = f.repo().with_dirs(writer.dirs().clone());
        thread::spawn(move || {
            let _read = refresh_lock(&repo).expect("dirs known");
            tx.send("read").unwrap();
        })
    };
    assert!(writer.scheduler().await_waiting(2, WAIT));
    assert!(rx.try_recv().is_err(), "both wait while the commit's locks are held");
    drop(held);
    let order: Vec<&str> = (0..3).map(|_| rx.recv_timeout(WAIT).unwrap()).collect();
    assert_eq!(order, ["write starts", "write ends", "read"]);
    write.join().unwrap();
    read.join().unwrap();
    assert!(git_in(&f.git, &f.dir, &["diff", "--cached", "--name-only"], &[], None).lines().any(|l| l == "u.txt"));
}

#[test]
fn a_later_write_on_free_resources_overtakes_a_blocked_one() {
    let f = Fixture::copy();
    let writer = Writer::open(&f.repo()).unwrap();
    let config = writer.scheduler().acquire(LockSet::new().write(Resource::Config));
    let (tx, rx) = mpsc::channel();
    let blocked = {
        let (writer, tx) = (writer.clone(), tx.clone());
        thread::spawn(move || writer.run(LockSet::commit(), |_| Ok(tx.send("commit").unwrap())).unwrap())
    };
    assert!(writer.scheduler().await_waiting(1, WAIT));
    let free = {
        let writer = writer.clone();
        thread::spawn(move || writer.run(LockSet::new().write(Resource::Index), |_| Ok(tx.send("index").unwrap())).unwrap())
    };
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), "index", "the index write runs past the blocked commit");
    free.join().unwrap();
    assert_eq!(writer.scheduler().waiting(), 1, "the commit still waits for the config");
    drop(config);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), "commit");
    blocked.join().unwrap();
}

fn args(cmd: &std::process::Command) -> Vec<String> {
    cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
}

fn envs(cmd: &std::process::Command) -> HashMap<String, Option<String>> {
    cmd.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect()
}

#[test]
fn every_write_carries_the_pinned_flags_env_and_literal_pathspecs() {
    let f = Fixture::copy();
    f.git(&["config", "core.ignorecase", "false"]);
    let writer = Writer::open(&f.repo()).unwrap();

    let (one, input) = writer.prepare(&Call::new(&["add"]).paths(["a b.txt"])).unwrap();
    let a = args(&one);
    assert!(a.windows(PINNED.len()).any(|w| w == PINNED), "{a:?}");
    assert!(a.ends_with(&["add".into(), "--".into(), ":(literal)a b.txt".into()]), "{a:?}");
    assert!(!a.iter().any(|a| a.contains("safe.directory")), "{a:?}");
    assert_eq!(input, None);
    let e = envs(&one);
    for (key, value) in [("LC_ALL", Some("C")), ("LANG", None), ("LANGUAGE", None), ("GIT_TERMINAL_PROMPT", Some("0")), ("GIT_EDITOR", Some("")), ("GIT_INDEX_FILE", None)] {
        assert_eq!(e.get(key).map(Option::as_deref), Some(value), "{key} in {e:?}");
    }

    let (many, input) = writer.prepare(&Call::new(&["add"]).paths(["u.txt", "s p.txt"]).editor("notepad")).unwrap();
    assert!(args(&many).ends_with(&["add".into(), "--pathspec-from-file=-".into(), "--pathspec-file-nul".into()]));
    assert_eq!(input.as_deref(), Some(&b":(literal)u.txt\0:(literal)s p.txt\0"[..]));
    assert_eq!(envs(&many).get("GIT_EDITOR"), Some(&Some("notepad".into())));
    assert!(writer.prepare(&Call::new(&["add"]).paths(["u.txt", "s.txt"]).input(b"x".to_vec())).is_err(), "one stdin");

    f.write("s p.txt", b"spaced\n");
    f.write("[u].txt", b"a glob would take u.txt\n");
    writer.run(LockSet::new().write(Resource::Index), |s| s.output(&Call::new(&["add"]).paths(["s p.txt", "[u].txt"]))).unwrap();
    let staged = git_in(&f.git, &f.dir, &["-c", "core.quotePath=false", "diff", "--cached", "--name-only"], &[], None);
    assert!(staged.lines().any(|l| l == "s p.txt") && staged.lines().any(|l| l == "[u].txt"), "{staged}");
    assert!(!staged.lines().any(|l| l == "u.txt"), "a literal pathspec is no glob: {staged}");

    f.git(&["config", "core.ignorecase", "true"]);
    assert_eq!(Writer::open(&f.repo()).unwrap().pathspec("A.txt"), ":(literal,icase)A.txt");
}

#[test]
fn a_snapshot_holds_the_index_and_the_working_files_and_leaves_the_index_alone() {
    let f = Fixture::copy();
    let writer = Writer::open(&f.repo()).unwrap();
    let index = f.dir.join(".git").join("index");
    let (before, status_before) = (std::fs::read(&index).unwrap(), f.git(&["--no-optional-locks", "status", "--porcelain"]));
    let sha = writer.snapshot(&["a.txt", "u.txt", "b.txt"], "Before discarding a.txt").unwrap();
    assert_eq!(std::fs::read(&index).unwrap(), before, "the real index is not touched");
    assert_eq!(f.git(&["--no-optional-locks", "status", "--porcelain"]), status_before);
    assert_eq!(f.git(&["cat-file", "-t", &sha]), "commit");
    assert_eq!(f.git(&["rev-parse", &format!("{sha}^")]), f.sha("ahead"));
    assert_eq!(f.git(&["log", "-1", "--format=%s", &sha]), "Before discarding a.txt");
    let show = |path: &str| f.git(&["show", &format!("{sha}:{path}")]);
    assert_eq!(show("a.txt"), "staged\nunstaged", "the working file, not the staged one");
    assert_eq!(show("u.txt"), "untracked");
    assert_eq!(show("s.txt"), "new", "the index's other entries");
    let files = f.git(&["ls-tree", "--name-only", &sha]);
    assert!(!files.lines().any(|l| l == "b.txt"), "a deleted file is gone: {files}");
    assert_eq!(f.git(&["for-each-ref", "--contains", &sha]), "", "dangling");
    let left: Vec<_> = std::fs::read_dir(f.dir.join(".git")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("gitman-snapshot")).collect();
    assert!(left.is_empty(), "the temp index is removed: {left:?}");
}

fn wt(f: &Fixture, args: &[&str], input: Option<Vec<u8>>) {
    let out = run_git(&f.git, &f.worktree, args, &[], input);
    assert!(!out.success(), "git {args:?} stops on a conflict");
}

fn side_commit(f: &Fixture) {
    std::fs::write(f.worktree.join("a.txt"), b"side\n").unwrap();
    git_in(&f.git, &f.worktree, &["commit", "-q", "--no-verify", "-am", "Side's a.txt"], &[], None);
}

fn state(f: &Fixture) -> OperationState {
    let repo = Repo::new(f.git.clone(), &f.worktree);
    let git_dir = dirs(&repo).unwrap().git_dir;
    let state = operation_state(&git_dir);
    assert_eq!(status(&repo).unwrap().state, state, "the status reads the same state");
    state
}

#[test]
fn operation_state_reads_a_merge_and_a_revert() {
    let f = Fixture::copy();
    assert_eq!(state(&f), OperationState::None);
    f.conflict();
    assert_eq!(state(&f), OperationState::Merge);

    let f = Fixture::copy();
    side_commit(&f);
    wt(&f, &["revert", "--no-edit", f.sha("second")], None);
    assert_eq!(state(&f), OperationState::Revert { sequence: false });
}

#[test]
fn operation_state_reads_a_rebase_external_or_gitmans_own_and_an_am() {
    let f = Fixture::copy();
    side_commit(&f);
    wt(&f, &["rebase", "main"], None);
    assert_eq!(state(&f), OperationState::Rebase { external: true });
    let git_dir = dirs(&Repo::new(f.git.clone(), &f.worktree)).unwrap().git_dir;
    std::fs::write(git_dir.join("rebase-merge").join(REBASE_MARKER), b"").unwrap();
    assert_eq!(state(&f), OperationState::Rebase { external: false });

    let f = Fixture::copy();
    side_commit(&f);
    let patch = git_in(&f.git, &f.dir, &["format-patch", "-1", "--stdout", f.sha("ahead")], &[], None) + "\n";
    wt(&f, &["am"], Some(patch.into_bytes()));
    assert_eq!(state(&f), OperationState::Am);
}

#[test]
fn operation_state_reads_a_cherry_pick_and_its_sequence() {
    let f = Fixture::copy();
    side_commit(&f);
    wt(&f, &["cherry-pick", f.sha("ahead")], None);
    assert_eq!(state(&f), OperationState::CherryPick { sequence: false });

    let f = Fixture::copy();
    side_commit(&f);
    wt(&f, &["cherry-pick", f.sha("ahead"), f.sha("main_work")], None);
    assert_eq!(state(&f), OperationState::CherryPick { sequence: true });
}

#[test]
fn the_first_status_of_a_repo_may_refresh_the_index_and_later_ones_do_not() {
    let f = Fixture::copy();
    let index = f.dir.join(".git").join("index");
    let before = std::fs::read(&index).unwrap();
    let repo = f.repo();
    let later = status_first(&repo).unwrap();
    let refreshed = std::fs::read(&index).unwrap();
    assert_ne!(refreshed, before, "bin.dat's old mtime is written to the stat cache");
    assert_eq!(status_first(&repo).unwrap(), later);
    assert_eq!(std::fs::read(&index).unwrap(), refreshed, "a later status leaves the index");
}

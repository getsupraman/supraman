//! A write's Cancel on fixture copies only (C196).

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use supraman_core::process::{alive, spawned_here};
use supraman_core::testing::{eventually, polled};
use supraman_gitman::OperationState;
use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::{Call, Cancel, LockSet, Left, WriteError, Writer, left, wip};

const WAIT: Duration = Duration::from_secs(20);

/// Ends the hook's wait when the test ends, passed or not.
struct Release(std::path::PathBuf);

impl Drop for Release {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, b"");
    }
}

#[test]
fn a_cancel_kills_a_hanging_hooks_git_frees_the_locks_and_refuses_the_jobs_later_calls() {
    let f = Fixture::copy();
    let git_dir = f.dir.join(".git");
    let _release = Release(git_dir.join("never"));
    std::fs::write(git_dir.join("hooks/pre-commit"), "#!/bin/sh\ncat /proc/$$/winpid > .git/hook.pid\nwhile [ ! -e .git/never ]; do sleep 1; done\n").unwrap();
    let writer = Writer::open(&f.repo()).unwrap();
    let cancel = Cancel::default();
    let (tx, rx) = mpsc::channel();
    {
        let (writer, cancel) = (writer.clone(), cancel.clone());
        thread::spawn(move || {
            let ran = writer.run_with(LockSet::commit(), cancel, |s| {
                s.output(&Call::new(&["add"]).paths(["u.txt"]))?;
                wip::commit(s, &wip::message("Hangs", ""), false, false)
            });
            tx.send(ran).unwrap();
        });
    }
    let hook = eventually(WAIT, "the pre-commit hook started", polled(Duration::from_millis(20)), || std::fs::read_to_string(git_dir.join("hook.pid")).ok()?.trim().parse::<u32>().ok());
    assert!(alive(hook));

    cancel.cancel();
    let ran = rx.recv_timeout(WAIT).expect("the cancel ends git's tree and the job");
    assert!(matches!(ran, Err(WriteError::Cancelled)), "{ran:?}");
    eventually(WAIT, "the hook is gone with git's tree", polled(Duration::from_millis(20)), || (!alive(hook)).then_some(()));
    assert!(writer.scheduler().try_acquire(LockSet::commit()).is_some(), "the job's locks are released");
    assert_eq!(writer.run(LockSet::commit(), |s| s.rev("HEAD")).unwrap(), f.git(&["rev-parse", "HEAD"]), "a second write runs");
    assert_eq!(left(writer.dirs()), Left { state: OperationState::None, index_lock: None });
    std::fs::write(git_dir.join("index.lock"), b"").unwrap();
    assert!(left(writer.dirs()).index_lock.is_some_and(|l| l.ends_with("index.lock")), "an index.lock a killed git left is said");

    let refused = writer.run_with(LockSet::commit(), cancel, |s| {
        let before = spawned_here();
        let ran = s.run(&Call::new(&["status"]));
        assert_eq!(spawned_here(), before, "no git starts");
        ran.map(drop)
    });
    assert!(matches!(refused, Err(WriteError::Cancelled)), "{refused:?}");
}

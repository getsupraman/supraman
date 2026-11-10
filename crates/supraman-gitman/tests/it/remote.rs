//! Fetch, pull and push against the fixture's bare temp remote and temp clones of it only (C196): file paths as
//! URLs, and a fake `GIT_SSH_COMMAND` for the slow transfer; no network, no ssh server.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use supraman_gitman::fixture::{Fixture, git_in, run_git};
use supraman_gitman::write::rebase::Rebased;
use supraman_gitman::write::remote::{self, Force, Progress, PullMode, Pulled, Pushed, Transfer};
use supraman_gitman::write::{LockSet, Session, WriteError, Writer};
use supraman_gitman::{Repo, remotes};

fn origin(f: &Fixture) -> PathBuf {
    f.temp.path().join("origin.git")
}

/// A clone of the fixture's remote beside it, its tree clean.
fn clone(f: &Fixture, name: &str) -> PathBuf {
    f.at(f.temp.path(), &["clone", "-q", "-b", "main", &origin(f).to_string_lossy(), name]);
    let dir = f.temp.path().join(name);
    for (key, value) in [("user.name", "Other"), ("user.email", "other@example.test"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        f.at(&dir, &["config", key, value]);
    }
    dir
}

fn commit(f: &Fixture, dir: &Path, file: &str, message: &str) -> String {
    std::fs::write(dir.join(file), format!("{message}\n")).unwrap();
    f.at(dir, &["add", file]);
    f.at(dir, &["commit", "-q", "-m", message]);
    f.at(dir, &["rev-parse", "HEAD"])
}

fn write<T>(f: &Fixture, dir: &Path, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Fixture::write_job(&Repo::new(f.git.clone(), dir), locks, job)
}

fn pull(f: &Fixture, dir: &Path, mode: PullMode) -> Result<Pulled, WriteError> {
    write(f, dir, LockSet::checkout(), |s| remote::pull(s, mode, true, &Transfer::default()))
}

fn push(f: &Fixture, dir: &Path, force: Force) -> Result<Pushed, WriteError> {
    write(f, dir, LockSet::push(), |s| remote::push(s, "main", "origin", "main", force, &Transfer::default()))
}

fn on_remote(f: &Fixture, name: &str) -> String {
    f.at(&origin(f), &["rev-parse", name])
}

#[test]
fn fetch_all_fetches_every_remote_prunes_the_gone_and_refuses_without_remotes() {
    let f = Fixture::copy();
    let other = clone(&f, "other");
    commit(&f, &other, "n.txt", "New on topic");
    f.at(&other, &["push", "-q", "origin", "HEAD:refs/heads/topic"]);
    git_in(&f.git, f.temp.path(), &["init", "-q", "--bare", "mirror.git"], &[], None);
    f.at(&other, &["push", "-q", &f.temp.path().join("mirror.git").to_string_lossy(), "HEAD:refs/heads/mirrored"]);
    f.git(&["remote", "add", "mirror", &f.temp.path().join("mirror.git").to_string_lossy()]);
    let names: Vec<String> = remotes(&f.repo()).unwrap().into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["mirror", "origin"]);

    write(&f, &f.dir, LockSet::fetch(), |s| remote::fetch_all(s, &names, true, &Transfer::default())).unwrap();
    assert_eq!(f.git(&["rev-parse", "origin/topic"]), on_remote(&f, "topic"));
    assert_eq!(f.git(&["rev-parse", "mirror/mirrored"]), on_remote(&f, "topic"));

    f.at(&other, &["push", "-q", "origin", "--delete", "topic"]);
    write(&f, &f.dir, LockSet::fetch(), |s| remote::fetch_all(s, &names, true, &Transfer::default())).unwrap();
    assert!(!run_git(&f.git, &f.dir, &["rev-parse", "-q", "--verify", "refs/remotes/origin/topic"], &[], None).success(), "pruned");

    let lone = clone(&f, "lone");
    f.at(&lone, &["remote", "remove", "origin"]);
    let none = write(&f, &lone, LockSet::fetch(), |s| remote::fetch_all(s, &[], true, &Transfer::default()));
    assert!(matches!(none, Err(WriteError::Refused(remote::NO_REMOTES))), "{none:?}");
}

#[test]
fn pull_fast_forwards_merges_or_rebases_per_mode() {
    let f = Fixture::copy();
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    let theirs = commit(&f, &other, "t.txt", "Theirs");
    f.at(&other, &["push", "-q"]);
    assert_eq!(pull(&f, &mine, PullMode::FastForwardOnly).unwrap(), Pulled::Merged);
    assert_eq!(f.at(&mine, &["rev-parse", "HEAD"]), theirs, "fast-forwarded");
    assert_eq!(pull(&f, &mine, PullMode::FastForwardOnly).unwrap(), Pulled::UpToDate);

    let theirs = commit(&f, &other, "t2.txt", "Theirs again");
    f.at(&other, &["push", "-q"]);
    commit(&f, &mine, "m.txt", "Mine");
    std::fs::write(mine.join("m.txt"), "work in progress\n").unwrap();
    assert!(pull(&f, &mine, PullMode::FastForwardOnly).is_err(), "gone apart: no fast-forward");
    assert_eq!(pull(&f, &mine, PullMode::FastForwardIfPossible).unwrap(), Pulled::Merged);
    assert_eq!(f.at(&mine, &["rev-parse", "HEAD^2"]), theirs, "a merge commit");
    assert_eq!(std::fs::read_to_string(mine.join("m.txt")).unwrap(), "work in progress\n", "the work in progress is put back");

    let rebasing = clone(&f, "rebasing");
    commit(&f, &rebasing, "r.txt", "Rebased mine");
    commit(&f, &other, "t3.txt", "Theirs third");
    f.at(&other, &["push", "-q"]);
    let pulled = pull(&f, &rebasing, PullMode::Rebase).unwrap();
    assert!(matches!(pulled, Pulled::Rebased(Rebased::Done { .. })), "{pulled:?}");
    assert_eq!(f.at(&rebasing, &["rev-parse", "HEAD^"]), on_remote(&f, "main"), "mine on top of theirs, no merge");
    assert_eq!(f.at(&rebasing, &["log", "-1", "--format=%s"]), "Rebased mine");
}

#[test]
fn pull_without_upstream_or_on_a_detached_head_says_so() {
    let f = Fixture::copy();
    let mine = clone(&f, "mine");
    f.at(&mine, &["checkout", "-q", "-b", "topic"]);
    assert_eq!(pull(&f, &mine, PullMode::FastForwardIfPossible).unwrap(), Pulled::NoUpstream { branch: "topic".into() });
    f.at(&mine, &["checkout", "-q", "--detach"]);
    assert!(matches!(pull(&f, &mine, PullMode::FastForwardIfPossible), Err(WriteError::Refused(remote::DETACHED))));
}

#[test]
fn push_says_behind_forces_with_lease_and_a_stale_lease_needs_a_fetch_or_force_anyway() {
    let f = Fixture::copy();
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    let pushed = commit(&f, &mine, "m.txt", "Mine");
    assert_eq!(push(&f, &mine, Force::No).unwrap(), Pushed::Done);
    assert_eq!(on_remote(&f, "main"), pushed);

    f.at(&other, &["pull", "-q"]);
    commit(&f, &other, "o.txt", "Other's");
    f.at(&other, &["push", "-q"]);
    let rewritten = commit(&f, &mine, "m2.txt", "Mine again");
    assert_eq!(push(&f, &mine, Force::No).unwrap(), Pushed::Behind, "the remote has a commit this branch lacks");
    assert_eq!(push(&f, &mine, Force::Lease).unwrap(), Pushed::StaleLease, "never fetched: the lease is stale");
    write(&f, &mine, LockSet::fetch(), |s| remote::fetch(s, "origin", true, &Transfer::default())).unwrap();
    assert_eq!(push(&f, &mine, Force::No).unwrap(), Pushed::Behind);
    assert_eq!(push(&f, &mine, Force::Lease).unwrap(), Pushed::Done);
    assert_eq!(on_remote(&f, "main"), rewritten);

    commit(&f, &other, "o2.txt", "Other's again");
    f.at(&other, &["push", "-q", "--force"]);
    assert_eq!(push(&f, &mine, Force::Lease).unwrap(), Pushed::StaleLease);
    assert_eq!(push(&f, &mine, Force::Anyway).unwrap(), Pushed::Done);
    assert_eq!(on_remote(&f, "main"), rewritten);
}

#[test]
fn a_branch_without_upstream_is_published_and_tags_and_branches_are_pushed_and_deleted() {
    let f = Fixture::copy();
    let mine = clone(&f, "mine");
    f.at(&mine, &["checkout", "-q", "-b", "topic"]);
    let sha = commit(&f, &mine, "t.txt", "Topic");
    let t = Transfer::default();
    assert_eq!(write(&f, &mine, LockSet::push(), |s| remote::publish(s, "topic", "origin", "feature/topic", &t)).unwrap(), Pushed::Done);
    assert_eq!(on_remote(&f, "feature/topic"), sha);
    assert_eq!(f.at(&mine, &["rev-parse", "--abbrev-ref", "topic@{upstream}"]), "origin/feature/topic");

    f.at(&mine, &["branch", "-q", "unfetched"]);
    write(&f, &mine, LockSet::branch(), |s| remote::set_upstream(s, "unfetched", "origin", "later")).unwrap();
    assert_eq!(f.at(&mine, &["config", "branch.unfetched.merge"]), "refs/heads/later");

    f.at(&mine, &["tag", "v9"]);
    assert_eq!(write(&f, &mine, LockSet::push(), |s| remote::push_tag(s, "origin", "v9", &t)).unwrap(), Pushed::Done);
    assert_eq!(on_remote(&f, "refs/tags/v9"), sha);
    write(&f, &mine, LockSet::push(), |s| remote::delete_remote_ref(s, "origin", "refs/tags/v9", &t)).unwrap();
    write(&f, &mine, LockSet::push(), |s| remote::delete_remote_ref(s, "origin", "refs/heads/feature/topic", &t)).unwrap();
    assert!(!run_git(&f.git, &origin(&f), &["rev-parse", "-q", "--verify", "refs/tags/v9"], &[], None).success());
    assert!(!run_git(&f.git, &origin(&f), &["rev-parse", "-q", "--verify", "refs/heads/feature/topic"], &[], None).success());

    write(&f, &mine, LockSet::remote_config(), |s| remote::set_url(s, "origin", "C:/elsewhere.git")).unwrap();
    assert_eq!(f.at(&mine, &["remote", "get-url", "origin"]), "C:/elsewhere.git");
    write(&f, &mine, LockSet::remote_config(), |s| remote::remove_remote(s, "origin")).unwrap();
    assert!(remotes(&Repo::new(f.git.clone(), &mine)).unwrap().is_empty());
}

/// A fake ssh that says how far it got, notes its Windows pid, and waits on its child `cat` for ever.
fn slow_ssh(dir: &Path) -> (String, PathBuf) {
    let pid = dir.join("ssh.pid");
    let script = dir.join("slow-ssh.sh");
    let unix = |p: &Path| p.to_string_lossy().replace('\\', "/");
    std::fs::write(&script, format!("cat /proc/$$/winpid > '{}'\nprintf 'Receiving objects:  50%% (1/2)\\r' >&2\ncat > /dev/null\n", unix(&pid))).unwrap();
    (format!("'{}' '{}'", unix(&git_sh()), unix(&script)), pid)
}

/// The sh Git for Windows ships beside its git.
fn git_sh() -> PathBuf {
    let git = supraman_gitman::fixture::system_git().path;
    git.ancestors().flat_map(|a| [a.join("usr/bin/sh.exe"), a.join("bin/sh.exe")]).find(|p| p.is_file()).expect("Git for Windows' sh")
}

#[test]
fn a_transfer_shows_its_progress_and_cancel_kills_its_tree() {
    let f = Fixture::copy();
    f.git(&["remote", "add", "slow", "ssh://fake.invalid/repo.git"]);
    let (ssh, pid_file) = slow_ssh(f.temp.path());
    let (said, heard) = mpsc::channel::<Progress>();
    let said = Mutex::new(said);
    let t = Transfer {
        progress: Arc::new(move |p| {
            let _ = said.lock().unwrap().send(p);
        }),
        env: vec![("GIT_SSH_COMMAND".into(), ssh.into()), ("GIT_SSH_VARIANT".into(), "simple".into())],
        ..Transfer::default()
    };
    let cancel = t.cancel.clone();
    let writer = Writer::open(&f.repo()).unwrap();
    let fetching = std::thread::spawn(move || writer.run(LockSet::fetch(), |s| remote::fetch(s, "slow", true, &t)));
    let progress = heard.recv_timeout(Duration::from_secs(60)).expect("the fake ssh's progress");
    assert_eq!(progress, Progress { phase: "Receiving objects".into(), percent: 50 });
    let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    assert!(supraman_core::process::alive(pid), "the transfer waits on its ssh");
    cancel.store(true, Ordering::SeqCst);
    let ended = fetching.join().unwrap();
    assert!(matches!(ended, Err(WriteError::Cancelled)), "{ended:?}");
    assert!(supraman_core::platform::process::wait_any_exit(&[pid], Duration::from_secs(10)), "Cancel killed the ssh under git");
}

#[test]
fn a_remote_call_leaves_the_credential_helper_alone_and_asks_only_through_sshs_askpass() {
    let f = Fixture::copy();
    f.git(&["config", "credential.helper", "!echo kept"]);
    let writer = Writer::open(&f.repo()).unwrap();
    let (cmd, _) = writer.prepare(&remote::fetch_call("origin", true)).unwrap();
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert!(!args.iter().any(|a| a.contains("credential")), "{args:?}");
    assert!(!cmd.get_envs().any(|(k, _)| k == "GIT_ASKPASS" || k.to_string_lossy().starts_with("GIT_CONFIG")), "git's own prompts are left to the user's helper");
    write(&f, &f.dir, LockSet::fetch(), |s| remote::fetch(s, "origin", true, &Transfer::default())).unwrap();
    assert_eq!(f.git(&["config", "--get-all", "credential.helper"]), "!echo kept");
}

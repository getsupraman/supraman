//! A fixture built under a hook's `GIT_DIR`, `GIT_WORK_TREE` and `GIT_INDEX_FILE` acts on its own temp repo only (C151).

use std::path::Path;
use std::time::Duration;

use supraman_core::process::{self, Kill};
use supraman_gitman::fixture::{Fixture, git_in, system_git};

#[test]
#[ignore = "the child half of a_fixture_under_a_hooks_git_env_leaves_that_repo_alone"]
fn build_a_fixture() {
    Fixture::new();
}

fn state(dir: &Path) -> Vec<String> {
    let git = system_git();
    let reads: [&[&str]; 7] = [
        &["rev-parse", "HEAD"],
        &["for-each-ref"],
        &["count-objects", "-v"],
        &["worktree", "list", "--porcelain"],
        &["config", "--local", "--list"],
        &["--no-optional-locks", "status", "--porcelain"],
        &["--no-optional-locks", "ls-files", "--stage"],
    ];
    reads
        .iter()
        .map(|args| git_in(&git, dir, args, &[], None))
        .collect()
}

#[test]
fn a_fixture_under_a_hooks_git_env_leaves_that_repo_alone() {
    let git = system_git();
    let temp = tempfile::tempdir().unwrap();
    git_in(&git, temp.path(), &["init", "-q", "-b", "main", "victim"], &[], None);
    let victim = temp.path().join("victim");
    for (key, value) in [("user.name", "Victim"), ("user.email", "victim@example.test"), ("commit.gpgsign", "false")] {
        git_in(&git, &victim, &["config", key, value], &[], None);
    }
    std::fs::write(victim.join("v.txt"), b"victim\n").unwrap();
    git_in(&git, &victim, &["add", "v.txt"], &[], None);
    git_in(&git, &victim, &["commit", "-q", "--no-verify", "-m", "Victim"], &[], None);
    let before = state(&victim);

    let dot_git = victim.join(".git");
    let mut child = process::command(std::env::current_exe().unwrap());
    child.args(["--exact", "build_a_fixture", "--ignored", "--test-threads=1"]);
    child.env("GIT_DIR", &dot_git).env("GIT_WORK_TREE", &victim).env("GIT_INDEX_FILE", dot_git.join("index"));
    let out = process::run(&mut child, Duration::from_secs(300), Kill::Tree).unwrap();
    assert_eq!(state(&victim), before);
    assert_eq!(out.code, Some(0), "the fixture builds: {}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("1 passed"), "the child ran the fixture: {}", out.stdout);
}

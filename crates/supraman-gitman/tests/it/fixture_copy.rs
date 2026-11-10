//! A test's copy of the shared fixture is a whole repo of its own: its writes never reach the shared one.

use std::path::Path;

use supraman_gitman::fixture::{Fixture, Generated, git_in};

fn state(f: &Fixture) -> (Vec<String>, Vec<u8>) {
    let git = |dir: &Path, args: &[&str]| git_in(&f.git, dir, args, &[], None);
    let reads = vec![
        git(&f.dir, &["rev-parse", "HEAD"]),
        git(&f.dir, &["for-each-ref"]),
        git(&f.dir, &["worktree", "list", "--porcelain"]),
        git(&f.dir, &["--no-optional-locks", "status", "--porcelain"]),
        git(&f.worktree, &["rev-parse", "HEAD"]),
        git(&f.worktree, &["--no-optional-locks", "status", "--porcelain"]),
    ];
    (reads, std::fs::read(f.dir.join(".git").join("index")).unwrap())
}

#[test]
fn a_fixtures_copy_writes_only_itself_and_is_a_sound_repo_with_its_own_worktree() {
    let shared = Fixture::shared();
    let before = state(&shared);
    let copy = Fixture::copy();
    for label in ["root", "merge", "ahead", "stash_wip", "media"] {
        assert_eq!(copy.sha(label), shared.sha(label), "{label}");
    }

    let listed = copy.git(&["worktree", "list", "--porcelain"]);
    let top = |p: &Path| p.canonicalize().unwrap().to_string_lossy().replace('\\', "/").trim_start_matches("//?/").to_lowercase();
    assert!(listed.to_lowercase().contains(&top(&copy.worktree)) && listed.to_lowercase().contains(&top(&copy.dir)), "{listed}");
    assert!(!listed.to_lowercase().contains(&top(&shared.worktree)), "{listed}");
    assert_eq!(copy.git(&["config", "--get", "remote.origin.url"]), copy.temp.path().join("origin.git").to_string_lossy());
    assert_eq!(copy.git(&["fsck", "--no-progress", "--no-dangling"]), "");
    assert_eq!(copy.git(&["worktree", "prune", "--dry-run", "-v"]), "", "no worktree of the copy is stale");

    copy.git(&["commit", "-q", "--no-verify", "-m", "Only in the copy"]);
    copy.git(&["branch", "copy-only"]);
    copy.git(&["stash", "push", "-q", "-m", "copy stash"]);
    copy.git(&["push", "-q", "origin", "copy-only"]);
    copy.conflict();
    git_in(&copy.git, &copy.worktree, &["merge", "--abort"], &[], None);
    git_in(&copy.git, &copy.worktree, &["checkout", "-q", "--detach"], &[], None);

    assert_eq!(state(&shared), before, "the shared fixture is as it was");
    assert!(!git_in(&shared.git, &shared.temp.path().join("origin.git"), &["branch"], &[], None).contains("copy-only"), "its origin too");
    assert_eq!(copy.git(&["fsck", "--no-progress", "--no-dangling"]), "");
}

#[test]
fn a_fixtures_copy_leaves_out_the_locks_and_scratch_indexes_a_reader_holds() {
    let original = Fixture::new();
    let git_dir = original.dir.join(".git");
    let held = [git_dir.join("index.lock"), git_dir.join("HEAD.lock"), git_dir.join("worktrees").join("wt").join("index.lock"), git_dir.join("gitman-snapshot-abc123.index")];
    for file in &held {
        std::fs::write(file, b"held").unwrap();
    }
    let copy = Fixture::copy_of(&original);
    for file in &held {
        let copied = copy.temp.path().join(file.strip_prefix(original.temp.path()).unwrap());
        assert!(!copied.exists(), "{} is copied", copied.display());
    }
    copy.conflict();
    assert!(copy.git(&["--no-optional-locks", "status", "--porcelain"]).contains("renamed.txt"), "the copy's index is the original's");
}

#[test]
fn a_generated_copy_writes_only_itself() {
    let shared = Generated::shared(30);
    let refs = git_in(&shared.git, &shared.dir, &["for-each-ref"], &[], None);
    let copy = Generated::copy(30);
    assert_eq!(git_in(&copy.git, &copy.dir, &["for-each-ref"], &[], None), refs);
    let head = git_in(&copy.git, &copy.dir, &["rev-parse", "HEAD"], &[], None);
    git_in(&copy.git, &copy.dir, &["update-ref", "refs/heads/main", &format!("{head}^")], &[], None);
    assert_eq!(git_in(&shared.git, &shared.dir, &["for-each-ref"], &[], None), refs);
    assert_eq!(git_in(&copy.git, &copy.dir, &["fsck", "--no-progress", "--no-dangling"], &[], None), "");
}

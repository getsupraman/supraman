//! The stash's writes on fixture copies only (C196).

use supraman_gitman::fixture::{FEATURE_STASH, Fixture, MAIN_STASH, git_in, run_git};
use supraman_gitman::write::stash::{self, Applied, CONFLICTED, GONE, NO_CHANGES, OVERWRITE, STAGED};
use supraman_gitman::write::{LockSet, Session, WriteError};
use supraman_gitman::*;

fn change(kind: ChangeKind, path: &str, old: Option<&str>) -> FileChange {
    FileChange { kind, path: path.into(), old_path: old.map(Into::into), similarity: None }
}

fn write<T>(f: &Fixture, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Fixture::write_job(&f.repo(), LockSet::stash(), job)
}

fn refused<T: std::fmt::Debug>(r: Result<T, WriteError>) -> &'static str {
    match r {
        Err(WriteError::Refused(why)) => why,
        other => panic!("refused, not {other:?}"),
    }
}

fn status_lines(f: &Fixture) -> Vec<String> {
    let out = run_git(&f.git, &f.dir, &["status", "--porcelain=v1", "--untracked-files=all"], &[], None);
    let mut lines: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
    lines.sort();
    lines
}

fn staged(f: &Fixture) -> String {
    f.git(&["diff", "--cached", "--name-only"])
}

fn read(f: &Fixture, path: &str) -> Option<String> {
    std::fs::read_to_string(f.dir.join(path)).ok()
}

fn all(f: &Fixture) -> Vec<Stash> {
    stashes(&f.repo()).unwrap()
}

fn head_reflog(f: &Fixture) -> String {
    f.git(&["reflog", "show", "--format=%H %gs", "HEAD"])
}

/// No change left: the fixture's work in progress thrown away.
fn clean(f: &Fixture) {
    f.git(&["reset", "-q", "--hard"]);
    f.git(&["clean", "-fdq"]);
}

#[test]
fn stash_all_takes_untracked_files_too_and_pop_brings_everything_back_unstaged() {
    let f = Fixture::copy();
    let (before, reflog) = (status_lines(&f), head_reflog(&f));
    write(&f, |s| stash::stash_all(s, "", false)).unwrap();
    assert_eq!(status_lines(&f), Vec::<String>::new(), "nothing is left, untracked files neither");
    let newest = all(&f).remove(0);
    assert_eq!((newest.label(), all(&f).len()), ("WIP on main".to_string(), 4));
    assert_eq!(head_reflog(&f), reflog, "the stash's own reset leaves no line in HEAD's reflog");
    assert_eq!(refused(write(&f, |s| stash::stash_all(s, "", false))), NO_CHANGES);

    assert_eq!(write(&f, |s| stash::apply(s, &newest.sha, true)).unwrap(), Applied::Clean);
    assert_eq!(staged(&f), "", "everything comes back unstaged");
    assert_eq!(all(&f).len(), 3, "the pop dropped it");
    assert_eq!(head_reflog(&f), reflog, "the reset after the apply leaves no line either");
    let back = status_lines(&f);
    for line in ["?? u.txt", "?? s.txt", "?? renamed.txt", " D new.txt", " D b.txt", " M a.txt"] {
        assert!(back.contains(&line.to_string()), "{line} in {back:?}, before {before:?}");
    }
    assert_eq!(read(&f, "a.txt").as_deref(), Some("staged\nunstaged\n"));
}

#[test]
fn stash_all_with_a_message_or_only_what_is_staged() {
    let f = Fixture::copy();
    assert_eq!(refused(write(&f, |s| stash::stash_all(s, "", true))), stash::PARTLY_STAGED, "a.txt has unstaged changes too");
    f.git(&["add", "a.txt"]);
    write(&f, |s| stash::stash_all(s, "  Only staged ", true)).unwrap();
    assert_eq!(all(&f)[0].label(), "Only staged");
    assert_eq!(staged(&f), "", "the staged changes went");
    assert_eq!(status_lines(&f), [" D b.txt", "?? u.txt"].map(String::from), "the unstaged and untracked changes stay");
    assert_eq!(refused(write(&f, |s| stash::stash_all(s, "", true))), stash::NOTHING_STAGED);
}

#[test]
fn stashing_files_takes_a_rename_with_its_old_path_an_untracked_file_and_both_halves_of_a_partly_staged_one() {
    let f = Fixture::copy();
    let head_a = f.git(&["show", "HEAD:a.txt"]);
    let files = [change(ChangeKind::Renamed, "renamed.txt", Some("new.txt")), change(ChangeKind::Untracked, "u.txt", None), change(ChangeKind::Modified, "a.txt", None)];
    write(&f, |s| stash::stash_files(s, &files, "Some files")).unwrap();
    assert_eq!(all(&f)[0].label(), "Some files");
    assert_eq!(status_lines(&f), [" D b.txt", "A  s.txt"].map(String::from), "only the other files' changes stay");
    assert_eq!((read(&f, "renamed.txt"), read(&f, "u.txt")), (None, None));
    assert!(read(&f, "new.txt").is_some(), "the rename's old path is back");
    assert_eq!(read(&f, "a.txt").map(|t| t.trim_end().to_string()), Some(head_a));

    let sha = all(&f)[0].sha.clone();
    let tree = |rev: &str| f.git(&["ls-tree", "-r", "--name-only", rev]);
    let work = tree(&sha);
    assert!(work.contains("renamed.txt") && !work.contains("new.txt") && !work.contains("s.txt"), "only the picked files, the rename with its old path: {work}");
    assert_eq!((f.git(&["show", &format!("{sha}^2:a.txt")]), f.git(&["show", &format!("{sha}:a.txt")])), ("staged".into(), "staged\nunstaged".into()), "both halves of a.txt");
    assert_eq!(tree(&format!("{sha}^3")), "u.txt");
    assert_eq!(refused(write(&f, |s| stash::apply(s, &sha, true))), STAGED, "s.txt is staged");
    assert_eq!(all(&f).len(), 4);
    f.git(&["reset", "-q", "--", "s.txt"]);
    assert_eq!(write(&f, |s| stash::apply(s, &sha, true)).unwrap(), Applied::Clean);
    assert_eq!(staged(&f), "");
    let back = status_lines(&f);
    for line in ["?? renamed.txt", " D new.txt", "?? u.txt", " M a.txt"] {
        assert!(back.contains(&line.to_string()), "{line} in {back:?}");
    }
    assert_eq!(read(&f, "a.txt").as_deref(), Some("staged\nunstaged\n"));
    assert_eq!(refused(write(&f, |s| stash::stash_files(s, &[change(ChangeKind::Unmerged, "a.txt", None)], ""))), CONFLICTED);
}

#[test]
fn a_pop_that_conflicts_keeps_the_stash_and_an_overwrite_is_refused() {
    let f = Fixture::copy();
    clean(&f);
    let wip = f.sha("stash_wip").to_string();
    f.write("b.txt", b"b\nb2\nmine\n");
    assert_eq!(refused(write(&f, |s| stash::apply(s, &wip, true))), OVERWRITE);
    f.git(&["commit", "-q", "--no-verify", "-am", "Mine"]);
    assert_eq!(write(&f, |s| stash::apply(s, &wip, true)).unwrap(), Applied::Conflicted);
    assert!(all(&f).iter().any(|s| s.sha == wip), "the conflicted stash is not dropped");
    assert_eq!(all(&f).len(), 3);
    assert!(status_lines(&f).contains(&"UU b.txt".to_string()), "{:?}", status_lines(&f));
}

#[test]
fn delete_and_a_new_message_find_the_stash_by_its_commit() {
    let f = Fixture::copy();
    let (message, feature) = (f.sha("stash_message").to_string(), f.sha("stash_feature").to_string());
    write(&f, |s| stash::delete(s, &message)).unwrap();
    assert_eq!(all(&f).iter().map(Stash::label).collect::<Vec<_>>(), [all(&f)[0].label(), FEATURE_STASH.to_string()]);
    assert!(!all(&f).iter().any(|s| s.message == MAIN_STASH));
    assert_eq!(refused(write(&f, |s| stash::delete(s, &message))), GONE);

    write(&f, |s| stash::set_message(s, &feature, "New words")).unwrap();
    let newest = all(&f).remove(0);
    assert_eq!((newest.sha.as_str(), newest.label().as_str(), newest.branch.as_str()), (feature.as_str(), "New words", "feature"), "the same commit, now on top");
    assert_eq!(all(&f).len(), 2);
}

#[test]
fn applying_files_from_a_stash_writes_the_work_tree_only_a_rename_with_its_old_path() {
    let f = Fixture::copy();
    clean(&f);
    let feature = f.sha("stash_feature").to_string();
    write(&f, |s| stash::apply_files(s, &feature, &[change(ChangeKind::Modified, "new.txt", None)])).unwrap();
    assert_eq!(read(&f, "new.txt").as_deref(), Some("rename me\nline 2 on feature\nline 3\nline 4\n"));
    assert_eq!((staged(&f), status_lines(&f)), (String::new(), vec![" M new.txt".to_string()]));

    clean(&f);
    f.git(&["mv", "b.txt", "c.txt"]);
    git_in(&f.git, &f.dir, &["stash", "push", "-q", "-m", "Moved"], &[], None);
    let moved = all(&f)[0].sha.clone();
    write(&f, |s| stash::apply_files(s, &moved, &[change(ChangeKind::Renamed, "c.txt", Some("b.txt"))])).unwrap();
    assert_eq!((read(&f, "c.txt").is_some(), read(&f, "b.txt").is_some()), (true, false));
    assert_eq!(staged(&f), "", "the index is untouched");
}

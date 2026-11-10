//! The conflict panel's writes and reads, on fixture copies only (C196).

use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::conflicts::{self, Marked, RESOLVE_FIRST, Sequence, Side};
use supraman_gitman::write::wip::{self, Step};
use supraman_gitman::write::rebase::Rebased;
use supraman_gitman::write::{LockSet, Session, WriteError};
use supraman_gitman::*;

fn write<T>(f: &Fixture, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Fixture::write_job(&f.side(), LockSet::rebase(), job)
}

fn paths(p: &[&str]) -> Vec<String> {
    p.iter().map(|p| p.to_string()).collect()
}

fn read(f: &Fixture, path: &str) -> Option<String> {
    std::fs::read_to_string(f.worktree.join(path)).ok()
}

fn state(f: &Fixture) -> Status {
    status(&f.side()).unwrap()
}

#[test]
fn take_current_and_take_incoming_write_that_side_whole_and_stage_it() {
    for (taken, expected) in [(Side::Current, "side\n"), (Side::Incoming, "one\ntwo\nthree\n")] {
        let f = Fixture::copy();
        f.conflict();
        write(&f, |s| conflicts::take(s, &paths(&["a.txt"]), taken)).unwrap();
        assert_eq!(read(&f, "a.txt").as_deref(), Some(expected), "{taken:?}");
        assert_eq!(f.at(&f.worktree, &["ls-files", "-u"]), "", "no stage left");
        assert_eq!(f.at(&f.worktree, &["diff", "--name-only"]), "", "nothing unstaged");
        assert_eq!(f.at(&f.worktree, &["show", ":a.txt"]), expected.trim_end(), "staged");
    }
}

#[test]
fn take_a_side_that_deleted_the_file_deletes_it_and_the_other_side_keeps_it() {
    let f = Fixture::copy();
    f.delete_conflict();
    let b = state(&f).entries.into_iter().find(|e| e.path == "b.txt").unwrap();
    assert_eq!(b.conflict(), Some(Conflict::DeletedByThem));
    write(&f, |s| conflicts::take(s, &paths(&["b.txt"]), Side::Incoming)).unwrap();
    assert_eq!(read(&f, "b.txt"), None, "incoming deleted it");
    assert_eq!((f.at(&f.worktree, &["ls-files", "--", "b.txt"]), f.at(&f.worktree, &["ls-files", "-u"])), (String::new(), String::new()));

    let f = Fixture::copy();
    f.delete_conflict();
    write(&f, |s| conflicts::take(s, &paths(&["b.txt"]), Side::Current)).unwrap();
    assert_eq!(read(&f, "b.txt").as_deref(), Some("b on side\n"));
    assert_eq!((f.at(&f.worktree, &["ls-files", "-s", "--", "b.txt"]).split_whitespace().nth(2), f.at(&f.worktree, &["ls-files", "-u"])), (Some("0"), String::new()));
}

#[test]
fn mark_resolved_finds_markers_first_and_stages_only_the_conflicted_files() {
    let f = Fixture::copy();
    f.conflict();
    std::fs::write(f.worktree.join("extra.txt"), b"untracked\n").unwrap();
    let marked = write(&f, |s| conflicts::mark_resolved(s, None, false)).unwrap();
    assert_eq!(marked, Marked::Markers(paths(&["a.txt"])));
    assert!(!f.at(&f.worktree, &["ls-files", "-u"]).is_empty(), "nothing staged while markers remain");

    assert_eq!(write(&f, |s| conflicts::mark_resolved(s, None, true)).unwrap(), Marked::Staged(1));
    assert_eq!(f.at(&f.worktree, &["ls-files", "-u"]), "");
    assert_eq!(f.at(&f.worktree, &["ls-files", "--", "extra.txt"]), "", "mark all resolved stages only the conflicted files");

    let f = Fixture::copy();
    f.conflict();
    std::fs::write(f.worktree.join("a.txt"), b"resolved\n").unwrap();
    assert_eq!(write(&f, |s| conflicts::mark_resolved(s, Some(&paths(&["a.txt"])), false)).unwrap(), Marked::Staged(1), "no markers: no question");
}

fn mark_conflicted_restores_the_stages(f: &Fixture) {
    let before = f.at(&f.worktree, &["ls-files", "-u"]);
    assert!(!before.is_empty());
    std::fs::write(f.worktree.join("a.txt"), b"resolved\n").unwrap();
    write(f, |s| conflicts::mark_resolved(s, None, false)).unwrap();
    assert_eq!(f.at(&f.worktree, &["ls-files", "-u"]), "");
    write(f, |s| conflicts::mark_conflicted(s, &paths(&["a.txt"]))).unwrap();
    assert_eq!(f.at(&f.worktree, &["ls-files", "-u"]), before, "stages 1 to 3 back as git wrote them");
    assert_eq!(read(f, "a.txt").as_deref(), Some("resolved\n"), "the work tree is left alone");
    assert_eq!(f.at(&f.worktree, &["ls-files", "-s", "--", "a.txt"]).lines().count(), 3, "no stage 0 left");
}

#[test]
fn mark_conflicted_restores_stages_one_to_three_in_a_merge_a_cherry_pick_and_a_revert() {
    let conflicts: [fn(&Fixture); 3] = [Fixture::conflict, Fixture::cherry_pick_conflict, Fixture::revert_conflict];
    for conflict in conflicts {
        let f = Fixture::copy();
        conflict(&f);
        mark_conflicted_restores_the_stages(&f);
    }
}

#[test]
fn a_cherry_pick_continues_skips_and_aborts_with_gits_own_commands() {
    let f = Fixture::copy();
    f.cherry_pick_conflict();
    let pick = Sequence::CherryPick;
    assert_eq!(state(&f).state, OperationState::CherryPick { sequence: false });
    assert!(matches!(write(&f, |s| wip::step(s, Step::Continue(pick))), Err(WriteError::Refused(RESOLVE_FIRST))));
    write(&f, |s| conflicts::take(s, &paths(&["a.txt"]), Side::Incoming)).unwrap();
    let done = write(&f, |s| wip::step(s, Step::Continue(pick))).unwrap();
    assert!(matches!(&done, Rebased::Done { head, .. } if *head == f.at(&f.worktree, &["rev-parse", "HEAD"])));
    assert_eq!((state(&f).state, f.at(&f.worktree, &["log", "-1", "--format=%s"])), (OperationState::None, "Ahead of origin".to_string()));

    let f = Fixture::copy();
    f.cherry_pick_conflict();
    let before = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    write(&f, |s| wip::step(s, Step::Skip(pick))).unwrap();
    assert_eq!((state(&f).state, f.at(&f.worktree, &["rev-parse", "HEAD"])), (OperationState::None, before));

    let f = Fixture::copy();
    f.cherry_pick_conflict();
    let before = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    write(&f, |s| wip::step(s, Step::Abort(pick))).unwrap();
    assert_eq!((state(&f).state, f.at(&f.worktree, &["rev-parse", "HEAD"]), read(&f, "a.txt")), (OperationState::None, before, Some("side\n".into())));
}

#[test]
fn a_revert_continues_and_aborts_with_gits_own_commands() {
    let f = Fixture::copy();
    f.revert_conflict();
    let revert = Sequence::Revert;
    assert_eq!(state(&f).state, OperationState::Revert { sequence: false });
    let before = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    std::fs::write(f.worktree.join("a.txt"), b"one\n").unwrap();
    write(&f, |s| conflicts::mark_resolved(s, None, false)).unwrap();
    write(&f, |s| wip::step(s, Step::Continue(revert))).unwrap();
    assert_eq!(state(&f).state, OperationState::None);
    assert_eq!(f.at(&f.worktree, &["rev-parse", "HEAD^"]), before);
    assert!(f.at(&f.worktree, &["log", "-1", "--format=%s"]).starts_with("Revert \"Second\""));

    let f = Fixture::copy();
    f.revert_conflict();
    let before = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    write(&f, |s| wip::step(s, Step::Abort(revert))).unwrap();
    assert_eq!((state(&f).state, f.at(&f.worktree, &["rev-parse", "HEAD"])), (OperationState::None, before));
}

#[test]
fn a_merge_is_not_committed_while_a_file_is_conflicted() {
    let f = Fixture::copy();
    f.conflict();
    let committed = write(&f, |s| wip::commit(s, "Merge main into side\n", false, true));
    assert!(matches!(committed, Err(WriteError::Refused(RESOLVE_FIRST))), "{committed:?}");
    write(&f, |s| conflicts::take(s, &paths(&["a.txt"]), Side::Current)).unwrap();
    write(&f, |s| wip::commit(s, "Merge main into side\n", false, true)).unwrap();
    assert_eq!((state(&f).state, f.at(&f.worktree, &["rev-list", "--count", "--no-walk", "--merges", "HEAD"])), (OperationState::None, "1".to_string()));
}

#[test]
fn the_status_names_the_sides_of_each_operation() {
    let f = Fixture::copy();
    f.conflict();
    let merge = state(&f).under_way.unwrap();
    assert_eq!((merge.current.as_str(), merge.incoming.as_str()), ("side", "main"));
    assert!(merge.message.as_deref().is_some_and(|m| m.starts_with("Merge branch 'main' into side") && !m.contains('#')), "{merge:?}");

    let f = Fixture::copy();
    f.cherry_pick_conflict();
    let pick = state(&f).under_way.unwrap();
    assert_eq!((pick.current.as_str(), pick.incoming.as_str()), ("side", short_sha(f.sha("ahead"))));

    let f = Fixture::copy();
    stopped_rebase(&f);
    let rebase = state(&f).under_way.unwrap();
    assert_eq!((rebase.incoming.as_str(), rebase.step), ("side", Some((2, 2))));
    assert_eq!(status(&f.repo()).unwrap().under_way, None, "nothing under way");
}

/// `side` with W1 to W3 changing c.txt, stopped in GitMan's reword of W1 with W2 dropped: the pick of W3 conflicts.
fn stopped_rebase(f: &Fixture) {
    let commits = ["one", "two", "three"].map(|n| {
        std::fs::write(f.worktree.join("c.txt"), format!("{n}
")).unwrap();
        f.at(&f.worktree, &["add", "c.txt"]);
        f.at(&f.worktree, &["commit", "-q", "--no-verify", "-m", &format!("W {n}")]);
        f.at(&f.worktree, &["rev-parse", "HEAD"])
    });
    let todo = format!("reword {}
pick {}
", commits[0], commits[2]);
    assert_eq!(f.gitmans_rebase(&f.side(), &commits[0], &todo), Rebased::Conflicted);
}

#[test]
fn mark_conflicted_restores_the_stages_of_a_stopped_rebase() {
    let f = Fixture::copy();
    stopped_rebase(&f);
    let before = f.at(&f.worktree, &["ls-files", "-u"]);
    std::fs::write(f.worktree.join("c.txt"), b"three
").unwrap();
    write(&f, |s| conflicts::mark_resolved(s, None, false)).unwrap();
    write(&f, |s| conflicts::mark_conflicted(s, &paths(&["c.txt"]))).unwrap();
    assert_eq!(f.at(&f.worktree, &["ls-files", "-u"]), before);
}

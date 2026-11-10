//! A hunk's or some lines' stage, unstage and discard on fixture copies only (C196), for an LF file, a CRLF one
//! under `core.autocrlf=true` and a UTF-8 one with a BOM.

use std::collections::BTreeSet;

use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::journal;
use supraman_gitman::write::lines::{self, Change, Picked};
use supraman_gitman::write::{LockSet, Session, WriteError};

const PATH: &str = "picked.txt";
const BASE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
const WORK: &str = "one\nTWO\nthree\nfour\nfive\nsix\nSEVEN\neight\n";

/// How a case's file is stored and checked out.
#[derive(Clone, Copy)]
enum Case {
    Lf,
    AutoCrlf,
    Bom,
}

const CASES: [Case; 3] = [Case::Lf, Case::AutoCrlf, Case::Bom];

impl Case {
    /// The bytes git stores for `text`.
    fn stored(self, text: &str) -> Vec<u8> {
        match self {
            Case::Lf | Case::AutoCrlf => text.as_bytes().to_vec(),
            Case::Bom => [&b"\xEF\xBB\xBF"[..], text.as_bytes()].concat(),
        }
    }

    /// The bytes of the working file for `text`.
    fn checked_out(self, text: &str) -> Vec<u8> {
        match self {
            Case::AutoCrlf => text.replace('\n', "\r\n").into_bytes(),
            _ => self.stored(text),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Case::Lf => "LF",
            Case::AutoCrlf => "CRLF with core.autocrlf=true",
            Case::Bom => "UTF-8 with BOM",
        }
    }
}

/// A fixture copy with `PATH` committed as `BASE` and changed to `WORK` in the working file.
fn repo(case: Case) -> Fixture {
    let f = Fixture::copy();
    if let Case::AutoCrlf = case {
        f.git(&["config", "core.autocrlf", "true"]);
    }
    f.write(PATH, &case.checked_out(BASE));
    f.git(&["add", PATH]);
    f.git(&["commit", "-q", "--no-verify", "-m", "Add picked.txt"]);
    assert_eq!(show(&f, "HEAD"), case.stored(BASE), "{}: committed as stored", case.name());
    f.write(PATH, &case.checked_out(WORK));
    f
}

fn show(f: &Fixture, rev: &str) -> Vec<u8> {
    let out = supraman_gitman::fixture::run_git(&f.git, &f.dir, &["cat-file", "blob", &format!("{rev}:{PATH}")], &[], None);
    assert!(out.success(), "{rev}:{PATH} is there");
    out.stdout
}

fn write<T>(f: &Fixture, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
    Fixture::write_job(&f.repo(), locks, job)
}

/// The diff of `BASE` to `WORK` as the viewer shows it: its two changes.
fn changes() -> Vec<Change> {
    vec![Change { old: 1..2, new: 1..2 }, Change { old: 6..7, new: 6..8 }]
}

fn rows(r: &[u32]) -> BTreeSet<u32> {
    r.iter().copied().collect()
}

fn unstaged(hunk: Option<usize>, old: &[u32], new: &[u32]) -> Picked {
    let shown = (BASE.to_string(), WORK.to_string());
    match hunk {
        Some(ix) => Picked::hunk(PATH.into(), shown, changes(), ix),
        None => Picked { path: PATH.into(), shown, changes: changes(), old_rows: rows(old), new_rows: rows(new) },
    }
}

fn staged_diff(f: &Fixture) -> String {
    f.git(&["diff", "--cached", "--no-color", "-U0", "--", PATH])
}

#[test]
fn staging_a_hunk_or_lines_leaves_exactly_those_lines_staged() {
    for case in CASES {
        let f = repo(case);
        write(&f, LockSet::commit(), |s| lines::stage(s, &unstaged(Some(0), &[], &[]))).unwrap();
        assert_eq!(show(&f, ""), case.stored("one\nTWO\nthree\nfour\nfive\nsix\nseven\n"), "{}: the first hunk staged", case.name());
        let diff = staged_diff(&f);
        assert!(diff.contains("-two\n+TWO") && !diff.contains("SEVEN") && !diff.contains("eight"), "{}: {diff}", case.name());

        let picked = Picked { path: PATH.into(), shown: (show_text(&f, case, ""), WORK.into()), changes: vec![Change { old: 6..7, new: 6..8 }], old_rows: rows(&[]), new_rows: rows(&[7]) };
        write(&f, LockSet::commit(), |s| lines::stage(s, &picked)).unwrap();
        assert_eq!(show(&f, ""), case.stored("one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\n"), "{}: one added line staged, its neighbour's change not", case.name());

        write(&f, LockSet::commit(), |s| lines::stage(s, &Picked::hunk(PATH.into(), (show_text(&f, case, ""), WORK.into()), vec![Change { old: 6..7, new: 6..7 }], 0))).unwrap();
        assert_eq!(show(&f, ""), case.stored(WORK), "{}: the last line staged", case.name());
        assert_eq!(f.git(&["diff", "--name-only", "--", PATH]), "", "{}: nothing left unstaged", case.name());
    }
}

/// The index side (`rev` empty) or a commit's, as the viewer shows it.
fn show_text(f: &Fixture, case: Case, rev: &str) -> String {
    let bytes = show(f, rev);
    let bytes = if let Case::Bom = case { bytes[3..].to_vec() } else { bytes };
    String::from_utf8(bytes).unwrap().replace("\r\n", "\n")
}

#[test]
fn unstaging_a_hunk_or_lines_leaves_exactly_the_rest_staged() {
    for case in CASES {
        let f = repo(case);
        f.git(&["add", PATH]);
        assert_eq!(show(&f, ""), case.stored(WORK), "{}: all staged", case.name());
        let staged = |hunk: Option<usize>, old: &[u32], new: &[u32]| {
            let shown = (BASE.to_string(), show_text(&f, case, ""));
            match hunk {
                Some(ix) => Picked::hunk(PATH.into(), shown, changes(), ix),
                None => Picked { path: PATH.into(), shown, changes: changes(), old_rows: rows(old), new_rows: rows(new) },
            }
        };
        write(&f, LockSet::commit(), |s| lines::unstage(s, &staged(Some(0), &[], &[]))).unwrap();
        assert_eq!(show(&f, ""), case.stored("one\ntwo\nthree\nfour\nfive\nsix\nSEVEN\neight\n"), "{}: the first hunk unstaged", case.name());

        let picked = Picked { changes: vec![Change { old: 6..7, new: 6..8 }], ..staged(None, &[], &[7]) };
        write(&f, LockSet::commit(), |s| lines::unstage(s, &picked)).unwrap();
        assert_eq!(show(&f, ""), case.stored("one\ntwo\nthree\nfour\nfive\nsix\nSEVEN\n"), "{}: one added line unstaged", case.name());
        assert_eq!(f.git(&["diff", "--name-only", "--", PATH]), PATH, "{}: the working file keeps every change", case.name());
        assert_eq!(std::fs::read(f.dir.join(PATH)).unwrap(), case.checked_out(WORK));

        let rest = Picked { path: PATH.into(), shown: (BASE.into(), show_text(&f, case, "")), changes: vec![Change { old: 6..7, new: 6..7 }], old_rows: rows(&[6]), new_rows: rows(&[6]) };
        write(&f, LockSet::commit(), |s| lines::unstage(s, &rest)).unwrap();
        assert_eq!(staged_diff(&f), "", "{}: nothing left staged", case.name());
    }
}

#[test]
fn a_discarded_hunk_comes_back_byte_for_byte_with_undo_and_goes_again_with_redo() {
    for case in CASES {
        let f = repo(case);
        let before = std::fs::read(f.dir.join(PATH)).unwrap();
        let stashes = f.git(&["stash", "list"]);
        write(&f, LockSet::reset(), |s| lines::discard(s, &unstaged(Some(1), &[], &[]))).unwrap();
        let discarded = std::fs::read(f.dir.join(PATH)).unwrap();
        assert_eq!(discarded, case.checked_out("one\nTWO\nthree\nfour\nfive\nsix\nseven\n"), "{}: the last hunk discarded", case.name());
        assert_eq!(show(&f, ""), case.stored(BASE), "{}: the index untouched", case.name());

        let done = write(&f, LockSet::undo(), |s| Ok(journal::undo(s, false))).unwrap().unwrap();
        assert_eq!(std::fs::read(f.dir.join(PATH)).unwrap(), before, "{}: undo puts back every byte ({})", case.name(), done.toast);
        assert_eq!(f.git(&["stash", "list"]), stashes, "{}: nothing went to a stash", case.name());
        assert_eq!(show(&f, ""), case.stored(BASE));

        write(&f, LockSet::undo(), |s| Ok(journal::redo(s, false))).unwrap().unwrap();
        assert_eq!(std::fs::read(f.dir.join(PATH)).unwrap(), discarded, "{}: redo discards the same lines only", case.name());
    }
}

#[test]
fn discarding_one_line_keeps_the_others() {
    let f = repo(Case::Lf);
    write(&f, LockSet::reset(), |s| lines::discard(s, &unstaged(None, &[6], &[6]))).unwrap();
    assert_eq!(std::fs::read(f.dir.join(PATH)).unwrap(), b"one\nTWO\nthree\nfour\nfive\nsix\nseven\neight\n");
}

#[test]
fn a_pick_of_a_file_changed_since_it_was_shown_is_refused() {
    let f = repo(Case::Lf);
    f.write(PATH, b"one\nother\n");
    let refused = write(&f, LockSet::commit(), |s| lines::stage(s, &unstaged(Some(0), &[], &[])));
    assert!(matches!(refused, Err(WriteError::Refused(_))), "{refused:?}");
    assert_eq!(show(&f, ""), BASE.as_bytes(), "nothing staged");
}

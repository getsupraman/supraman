//! A hunk's or some lines' stage, unstage and discard (docs/research/gitkraken-12.6.1.md, "Staging"): no
//! `git apply`; the whole file's text is built from the two sides the viewer showed, with the picked lines
//! moved from one side to the other, and written as a blob into the index or, smudged, to the working file.

use std::collections::BTreeSet;
use std::ops::Range;

use supraman_core::text::{Encoding, decode, encode};

use super::journal::{self, Action};
use super::{Call, Session, WriteError};
use crate::changes::{ChangeKind, FileChange};
use crate::parse::{nul_records, text};

/// What a pick does to the lines it picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// From the worktree into the index.
    Stage,
    /// The index's lines back to HEAD's.
    Unstage,
    /// The worktree's lines back to the index's.
    Discard,
}

/// One change of the diff shown: the rows it covers in the old text and in the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub old: Range<u32>,
    pub new: Range<u32>,
}

/// Lines of the diff the viewer showed of `path`: its texts (`\r\n` read as `\n`, rows as the editor counts them,
/// a lone `\r` ending one), every change of it, and the rows picked of each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub path: String,
    pub shown: (String, String),
    pub changes: Vec<Change>,
    pub old_rows: BTreeSet<u32>,
    pub new_rows: BTreeSet<u32>,
}

impl Picked {
    /// The whole change `ix`.
    pub fn hunk(path: String, shown: (String, String), changes: Vec<Change>, ix: usize) -> Picked {
        let (old_rows, new_rows) = changes.get(ix).map_or_else(Default::default, |c| (c.old.clone().collect(), c.new.clone().collect()));
        Picked { path, shown, changes, old_rows, new_rows }
    }
}

/// A side of the file as text and its encoding.
struct Side {
    text: String,
    encoding: Encoding,
}

fn side(bytes: &[u8]) -> Result<Side, WriteError> {
    let (text, encoding) = decode(bytes);
    if encoding == Encoding::Unknown {
        return Err(WriteError::Refused("Lines can be picked only in a UTF-8 or UTF-16 text file"));
    }
    Ok(Side { text, encoding })
}

/// `text` as the viewer reads it: `\r\n` as `\n`.
fn as_shown(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Each editor row's line of `shown`: the editor ends a row at `\n`, `\r\n` and a lone `\r`, a line ends at `\n`.
fn line_of_rows(shown: &str) -> Vec<u32> {
    let (mut rows, mut line) = (vec![0], 0);
    let mut chars = shown.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => {
                line += 1;
                rows.push(line);
            }
            '\r' if chars.peek() != Some(&'\n') => rows.push(line),
            _ => {}
        }
    }
    rows
}

/// A change's rows as lines.
fn lines(rows: &Range<u32>, of: &[u32]) -> Range<usize> {
    let at = |row: u32| of.get(row as usize).or(of.last()).copied().unwrap_or(0) as usize;
    if rows.is_empty() { at(rows.start)..at(rows.start) } else { at(rows.start)..at(rows.end - 1) + 1 }
}

/// The text between `old` and `new`: `base`'s lines outside the changes, and inside each change the old lines
/// kept and the new lines taken. With `toward_new` the picked lines move from old to new; without, back.
fn build(old: &[&str], new: &[&str], changes: &[(Range<usize>, Range<usize>)], picked: (&BTreeSet<usize>, &BTreeSet<usize>), toward_new: bool) -> String {
    let base = if toward_new { old } else { new };
    let mut out = String::new();
    let mut push = |line: &str| {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(line);
    };
    let mut at = 0;
    for (o, n) in changes {
        let span = if toward_new { o } else { n };
        base[at.min(span.start)..span.start].iter().for_each(|l| push(l));
        at = span.end;
        o.clone().filter(|i| picked.0.contains(i) != toward_new).for_each(|i| push(old[i]));
        n.clone().filter(|i| picked.1.contains(i) == toward_new).for_each(|i| push(new[i]));
    }
    base[at.min(base.len())..].iter().for_each(|l| push(l));
    out
}

/// The texts of `picked` as `pick` reads them: the old side, the new side and which one is rewritten.
struct Sides {
    old: Side,
    new: Side,
    toward_new: bool,
}

/// The file's text with the picked lines moved, in the encoding of the side it rewrites.
fn moved(picked: &Picked, sides: &Sides) -> Result<Vec<u8>, WriteError> {
    if as_shown(&sides.old.text) != picked.shown.0 || as_shown(&sides.new.text) != picked.shown.1 {
        return Err(WriteError::Refused("The file changed since it was shown; look again"));
    }
    let (old_rows, new_rows) = (line_of_rows(&picked.shown.0), line_of_rows(&picked.shown.1));
    let changes: Vec<(Range<usize>, Range<usize>)> = picked.changes.iter().map(|c| (lines(&c.old, &old_rows), lines(&c.new, &new_rows))).collect();
    let pick = |rows: &BTreeSet<u32>, of: &[u32]| rows.iter().filter_map(|r| of.get(*r as usize)).map(|l| *l as usize).collect::<BTreeSet<usize>>();
    let (old_lines, new_lines) = (pick(&picked.old_rows, &old_rows), pick(&picked.new_rows, &new_rows));
    let old: Vec<&str> = sides.old.text.split_inclusive('\n').collect();
    let new: Vec<&str> = sides.new.text.split_inclusive('\n').collect();
    let built = build(&old, &new, &changes, (&old_lines, &new_lines), sides.toward_new);
    Ok(encode(&built, if sides.toward_new { sides.old.encoding } else { sides.new.encoding }))
}

fn index_bytes(s: &Session, path: &str) -> Result<Vec<u8>, WriteError> {
    s.output(&Call::new(&["cat-file", "blob", "--end-of-options", &format!(":0:{path}")]))
}

fn head_bytes(s: &Session, path: &str) -> Result<Vec<u8>, WriteError> {
    s.output(&Call::new(&["cat-file", "blob", "--end-of-options", &format!("HEAD:{path}")]))
}

/// The working file as git would store it: through its clean filter and line-ending conversion.
fn clean_worktree(s: &Session, path: &str) -> Result<Vec<u8>, WriteError> {
    let file = s.writer().dirs().top.join(path);
    let raw = std::fs::read(&file).map_err(|source| WriteError::Io { what: format!("read {}", file.display()), source })?;
    let sha = write_blob(s, raw, Some(path))?;
    s.output(&Call::new(&["cat-file", "blob", &sha]))
}

/// `bytes` written as a blob, through `path`'s clean filter when given; its sha.
fn write_blob(s: &Session, bytes: Vec<u8>, path: Option<&str>) -> Result<String, WriteError> {
    let filter = path.map_or_else(|| "--no-filters".to_string(), |p| format!("--path={p}"));
    Ok(text(&s.output(&Call::new(&["hash-object", "-w", "--stdin", &filter]).input(bytes))?).trim().to_string())
}

/// The index entry of `path`: its mode and whether it is skip-worktree or assume-unchanged.
struct Entry {
    mode: String,
    skip_worktree: bool,
    assume_unchanged: bool,
}

fn entry(s: &Session, path: &str) -> Result<Option<Entry>, WriteError> {
    let out = s.output(&Call::new(&["ls-files", "-s", "-v", "-z"]).paths([path]))?;
    Ok(nul_records(&out).map(text).find_map(|r| {
        let (meta, _) = r.split_once('\t')?;
        let mut fields = meta.split(' ');
        let tag = fields.next()?.chars().next()?;
        let mode = fields.next()?.to_string();
        Some(Entry { mode, skip_worktree: tag.eq_ignore_ascii_case(&'S'), assume_unchanged: tag.is_ascii_lowercase() })
    }))
}

/// `bytes` as `path`'s index entry, its mode and flags kept (100644 for a new entry).
fn into_index(s: &Session, path: &str, bytes: Vec<u8>) -> Result<(), WriteError> {
    let sha = write_blob(s, bytes, None)?;
    let entry = entry(s, path)?;
    let mode = entry.as_ref().map_or("100644", |e| e.mode.as_str());
    let info = format!("{mode},{sha},{path}");
    let args: Vec<&str> = if entry.is_some() { vec!["update-index", "--cacheinfo", &info] } else { vec!["update-index", "--add", "--cacheinfo", &info] };
    s.output(&Call::new(&args))?;
    let Some(entry) = entry else { return Ok(()) };
    for (flag, on) in [("--skip-worktree", entry.skip_worktree), ("--assume-unchanged", entry.assume_unchanged)] {
        if on {
            s.output(&Call::new(&["update-index", flag, "-z", "--stdin"]).input([path.as_bytes(), b"\0"].concat()))?;
        }
    }
    Ok(())
}

/// Whether `args` (a `diff`) still lists a change of `path`.
fn still_differs(s: &Session, args: &[&str], path: &str) -> Result<bool, WriteError> {
    let args: Vec<&str> = [&["diff", "--quiet"][..], args].concat();
    Ok(!s.run(&Call::new(&args).paths([path]))?.success())
}

fn modified(path: &str) -> FileChange {
    FileChange { kind: ChangeKind::Modified, path: path.to_string(), old_path: None, similarity: None }
}

/// Stages the picked lines of the Unstaged diff (index → worktree); the whole file once nothing is left.
pub fn stage(s: &Session, picked: &Picked) -> Result<(), WriteError> {
    let path = picked.path.as_str();
    let sides = Sides { old: side(&index_bytes(s, path)?)?, new: side(&clean_worktree(s, path)?)?, toward_new: true };
    into_index(s, path, moved(picked, &sides)?)?;
    if !still_differs(s, &[], path)? {
        super::wip::stage(s, &[modified(path)])?;
    }
    Ok(())
}

/// Unstages the picked lines of the Staged diff (HEAD → index); the whole file once nothing is left.
pub fn unstage(s: &Session, picked: &Picked) -> Result<(), WriteError> {
    let path = picked.path.as_str();
    let sides = Sides { old: side(&head_bytes(s, path)?)?, new: side(&index_bytes(s, path)?)?, toward_new: false };
    into_index(s, path, moved(picked, &sides)?)?;
    if !still_differs(s, &["--cached", "HEAD"], path)? {
        super::wip::unstage(s, &[modified(path)])?;
    }
    Ok(())
}

/// Puts the picked lines of the Unstaged diff back to the index's in the working file, a snapshot first and
/// recorded for Undo.
pub fn discard(s: &Session, picked: &Picked) -> Result<(), WriteError> {
    let path = picked.path.as_str();
    let sides = Sides { old: side(&index_bytes(s, path)?)?, new: side(&clean_worktree(s, path)?)?, toward_new: false };
    let bytes = moved(picked, &sides)?;
    let snapshot = s.snapshot(&[path], "Before discard")?;
    let sha = write_blob(s, bytes, None)?;
    write_smudged(s, path, &sha)?;
    let paths = vec![path.to_string()];
    journal::record(s, Action::Discard { paths: paths.clone(), from_index: paths, left: vec![(path.to_string(), sha)] }, Some(&snapshot))
}

/// The blob `sha` through `path`'s smudge filter and line-ending conversion, written as the working file.
fn write_smudged(s: &Session, path: &str, sha: &str) -> Result<(), WriteError> {
    let bytes = s.output(&Call::new(&["cat-file", "--filters", &format!("--path={path}"), sha]))?;
    let file = s.writer().dirs().top.join(path);
    std::fs::write(&file, bytes).map_err(|source| WriteError::Io { what: format!("write {}", file.display()), source })
}

/// Whether the working file of `path` still holds the blob `sha` a discard of its lines left.
pub(crate) fn holds(s: &Session, path: &str, sha: &str) -> Result<bool, WriteError> {
    let out = s.run(&Call::new(&["hash-object", "--", path]))?;
    Ok(out.success() && text(&out.stdout).trim() == sha)
}

/// A discard of lines done again: each file of `left` holds its blob once more, a snapshot first, recorded.
pub(crate) fn discard_again(s: &Session, paths: &[String], from_index: &[String], left: &[(String, String)]) -> Result<(), WriteError> {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let snapshot = s.snapshot(&refs, "Before discard")?;
    for (path, sha) in left {
        write_smudged(s, path, sha)?;
    }
    journal::record(s, Action::Discard { paths: paths.to_vec(), from_index: from_index.to_vec(), left: left.to_vec() }, Some(&snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(i: &[usize]) -> BTreeSet<usize> {
        i.iter().copied().collect()
    }

    #[test]
    fn rows_after_a_lone_cr_belong_to_its_line() {
        assert_eq!(line_of_rows("a\nb\rc\nd"), [0, 1, 1, 2]);
        assert_eq!(line_of_rows("a\r\nb\n"), [0, 1, 2]);
    }

    #[test]
    fn picked_lines_move_and_the_rest_stays() {
        let old = ["a\n", "b\n", "c\n"];
        let new = ["a\n", "B\n", "c\n", "d\n"];
        let changes = [(1..2, 1..2), (3..3, 3..4)];
        assert_eq!(build(&old, &new, &changes, (&set(&[1]), &set(&[1])), true), "a\nB\nc\n");
        assert_eq!(build(&old, &new, &changes, (&set(&[]), &set(&[3])), true), "a\nb\nc\nd\n");
        assert_eq!(build(&old, &new, &changes, (&set(&[]), &set(&[1])), true), "a\nb\nB\nc\n", "an added line without its removal keeps the old one");
        assert_eq!(build(&old, &new, &changes, (&set(&[1]), &set(&[1])), false), "a\nb\nc\nd\n", "back toward old");
        assert_eq!(build(&old, &new, &changes, (&set(&[]), &set(&[3])), false), "a\nB\nc\n");
    }

    #[test]
    fn a_line_after_one_without_its_newline_starts_its_own() {
        let (old, new) = (["a"], ["a\n", "b\n"]);
        assert_eq!(build(&old, &new, &[(0..1, 0..2)], (&set(&[]), &set(&[1])), true), "a\nb\n");
    }
}

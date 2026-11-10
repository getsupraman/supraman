//! A file's writes from the file menus (docs/research/gitkraken-12.6.1.md §B items 8, 9 and 11): a file restored
//! from a commit (undoable as a discard), a working file deleted, and Ignore: a pattern appended to the checkout's
//! `.gitignore`, its files untracked when asked.

use super::journal::{self, Action};
use super::wip::remove_file;
use super::{Call, Session, WriteError};
use crate::parse::text;

/// What an Ignore item ignores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ignore {
    /// This file, from the checkout's top: anchored with a leading `/`.
    File(String),
    /// Every file with this extension, anywhere.
    Extension(String),
    /// Everything in this folder, from the checkout's top.
    Folder(String),
}

/// `path` with gitignore's special characters taken literally.
fn escaped(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut out = String::with_capacity(path.len());
    for (i, c) in path.chars().enumerate() {
        if matches!(c, '*' | '?' | '[' | ']' | '@' | '{' | '}' | '+') || (i == 0 && matches!(c, '#' | '!')) {
            out.push('\\');
        }
        out.push(c);
    }
    if out.ends_with(' ') {
        out.insert(out.len() - 1, '\\');
    }
    out
}

impl Ignore {
    /// Its line in `.gitignore`.
    pub fn pattern(&self) -> String {
        match self {
            Ignore::File(path) => format!("/{}", escaped(path)),
            Ignore::Extension(ext) => format!("*.{}", escaped(ext)),
            Ignore::Folder(folder) => format!("{}/", escaped(folder.trim_end_matches('/'))),
        }
    }

    /// The Ignore ▸ items of `path` (a file, else a folder): this file, all with its extension, all in its folder.
    pub fn choices(path: &str, folder: bool) -> Vec<Ignore> {
        if folder {
            return vec![Ignore::Folder(path.to_string())];
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut out = vec![Ignore::File(path.to_string())];
        if let Some((_, ext)) = name.rsplit_once('.').filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty()) {
            out.push(Ignore::Extension(ext.to_string()));
        }
        if let Some((parent, _)) = path.rsplit_once('/') {
            out.push(Ignore::Folder(parent.to_string()));
        }
        out
    }

    /// What its item says.
    pub fn words(&self) -> String {
        match self {
            Ignore::File(_) => "Ignore this file".into(),
            Ignore::Extension(ext) => format!("Ignore all *.{ext} files"),
            Ignore::Folder(folder) => format!("Ignore all in {}/", folder.trim_end_matches('/')),
        }
    }
}

/// `.gitignore` and `.gitattributes` are never offered to Ignore.
pub fn ignorable(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name != ".gitignore" && name != ".gitattributes"
}

/// `ignore`'s pattern appended to the checkout's `.gitignore` unless a line holds it already; with `untrack` the
/// tracked files under it (`paths`) are taken out of the index, their working files kept.
pub fn ignore(s: &Session, ignore: &Ignore, untrack: &[String]) -> Result<(), WriteError> {
    let file = s.writer().dirs().top.join(".gitignore");
    let before = match std::fs::read(&file) {
        Ok(bytes) => text(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(WriteError::io("write", &file)(source)),
    };
    let pattern = ignore.pattern();
    if !before.lines().any(|l| l.trim_end() == pattern) {
        let gap = if before.is_empty() || before.ends_with('\n') { "" } else { "\n" };
        supraman_core::fs::write_atomic(&file, format!("{before}{gap}{pattern}\n")).map_err(WriteError::io("write", &file))?;
    }
    if !untrack.is_empty() {
        s.output(&Call::new(&["rm", "-q", "-r", "--cached", "--force", "--ignore-unmatch"]).paths(untrack.iter().cloned()))?;
    }
    Ok(())
}

/// `paths` put back as `sha` has them, index and working file (gone where `sha` has none), after a snapshot;
/// recorded as a discard for Undo.
pub fn restore(s: &Session, sha: &str, paths: &[String]) -> Result<(), WriteError> {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let snapshot = s.snapshot(&refs, "Before restore")?;
    let (kept, gone): (Vec<&String>, Vec<&String>) = paths.iter().partition(|p| s.run(&Call::new(&["cat-file", "-e", &format!("{sha}:{p}")])).is_ok_and(|o| o.success()));
    if !kept.is_empty() {
        s.output(&Call::new(&["restore", &format!("--source={sha}"), "--staged", "--worktree"]).paths(kept.into_iter().cloned()))?;
    }
    if !gone.is_empty() {
        s.output(&Call::new(&["rm", "-q", "--cached", "--force", "--ignore-unmatch"]).paths(gone.iter().map(|p| p.to_string())))?;
        for path in gone {
            remove_file(&s.writer().dirs().top, path)?;
        }
    }
    journal::record(s, Action::Discard { paths: paths.to_vec(), from_index: Vec::new(), left: Vec::new() }, Some(&snapshot))
}

/// The working file `path` deleted; its change shows in the work in progress.
pub fn delete(s: &Session, path: &str) -> Result<(), WriteError> {
    remove_file(&s.writer().dirs().top, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_escape_specials_and_anchor_this_file() {
        assert_eq!(Ignore::File("a.txt".into()).pattern(), "/a.txt");
        assert_eq!(Ignore::File("dir\\x[1]*.log".into()).pattern(), "/dir/x\\[1\\]\\*.log");
        assert_eq!(Ignore::File("#odd".into()).pattern(), "/\\#odd", "a leading # is escaped before the anchor");
        assert_eq!(Ignore::Extension("log".into()).pattern(), "*.log");
        assert_eq!(Ignore::Folder("build/out/".into()).pattern(), "build/out/");
        assert_eq!(Ignore::choices("src/a.rs", false), [Ignore::File("src/a.rs".into()), Ignore::Extension("rs".into()), Ignore::Folder("src".into())]);
        assert_eq!(Ignore::choices("Makefile", false), [Ignore::File("Makefile".into())], "no extension, no folder at the top");
        assert!(!ignorable("sub/.gitignore") && ignorable("a.txt"));
    }
}

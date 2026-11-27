//! The file and folder menus of the WIP and commit panels' lists (docs/research/gitkraken-12.6.1.md §B items 8, 9 and 11).

use gpui::{Pixels, Point, SharedString};
use supraman_gitman::write::conflicts::Side;
use supraman_gitman::write::files::{Ignore, ignorable};
use supraman_gitman::{ChangeKind, FileChange};
use supraman_ui::controls::menu::{MenuItem, item};

use super::CommitAct;
use crate::git_view::branch::menu::RefAct;
use crate::git_view::wip::Section;

/// Which list a right-click came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Listed {
    Wip(Section),
    /// A commit's files, or with `stash` a stash's (its commit).
    Commit { sha: String, stash: Option<String> },
}

/// A file or folder of a list right-clicked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileClick {
    pub(crate) from: Listed,
    pub(crate) path: String,
    pub(crate) folder: bool,
    /// The row's hit key, its menu's anchor.
    pub(crate) anchor: SharedString,
    pub(crate) position: Point<Pixels>,
}

impl Eq for FileClick {}

/// What a file item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileAct {
    Stage(Vec<FileChange>),
    Unstage(Vec<FileChange>),
    /// The Unstaged list's discard, back to the index: the WIP panel's.
    Discard(Vec<FileChange>),
    /// Staged and unstaged changes thrown away, asked first; undoable.
    DiscardAll(Vec<String>),
    Take(Vec<String>, Side),
    Resolve(Vec<String>),
    Unresolve(Vec<String>),
    Stash(Vec<FileChange>),
    /// `untrack` the tracked paths Ignore and stop tracking takes out of the index.
    Ignore { ignore: Ignore, untrack: Vec<String> },
    Open(String),
    ShowInFolder(String),
    CopyPath(String),
    Delete(String),
    Restore { sha: String, paths: Vec<String> },
    ApplyStashed { sha: String, files: Vec<FileChange> },
}

/// What the menu of a click reads: the files it covers and whether the path is in the work tree.
pub(crate) struct FileFacts<'a> {
    pub(crate) click: &'a FileClick,
    pub(crate) files: Vec<FileChange>,
    pub(crate) exists: bool,
    /// A merge, rebase, cherry-pick, revert or am is under way.
    pub(crate) under_way: bool,
    /// The tab's gate's why while it is busy.
    pub(crate) busy: Option<&'static str>,
}

fn file(key: String, label: impl Into<String>, act: FileAct) -> MenuItem<RefAct> {
    item(key, label, RefAct::Commit(CommitAct::File(act)))
}

/// A write item, dimmed with the gate's why while the tab's gate is busy.
fn write(f: &FileFacts, it: MenuItem<RefAct>) -> MenuItem<RefAct> {
    match f.busy {
        Some(why) => it.dimmed(why),
        None => it,
    }
}

fn paths(files: &[FileChange]) -> Vec<String> {
    files.iter().map(|f| f.path.clone()).collect()
}

/// The files of `all` a click on `path` covers: the file, or every one below the folder.
pub(crate) fn covered(all: &[FileChange], path: &str, folder: bool) -> Vec<FileChange> {
    let below = format!("{path}/");
    all.iter().filter(|f| if folder { f.path.starts_with(&below) } else { f.path == path }).cloned().collect()
}

/// The menu of a click, its items in GitKraken's groups.
pub(crate) fn items(f: &FileFacts) -> Vec<MenuItem<RefAct>> {
    let c = f.click;
    let mut out = match &c.from {
        Listed::Wip(section) => wip_items(*section, f),
        Listed::Commit { sha, stash } => commit_items(sha, stash.as_deref(), f),
    };
    let mut place = Vec::new();
    if f.exists {
        let open = if c.folder { "Open folder" } else { "Open with the default program" };
        place.push(file(format!("file-open-{}", c.path), open, FileAct::Open(c.path.clone())));
        place.push(file(format!("file-show-{}", c.path), "Show in folder", FileAct::ShowInFolder(c.path.clone())));
    }
    place.push(file(format!("file-copy-{}", c.path), "Copy path", FileAct::CopyPath(c.path.clone())));
    if f.exists && !c.folder {
        place.push(write(f, file(format!("file-delete-{}", c.path), "Delete file", FileAct::Delete(c.path.clone()))));
    }
    for (i, it) in place.into_iter().enumerate() {
        out.push(if i == 0 && !out.is_empty() { it.rule() } else { it });
    }
    out
}

fn wip_items(section: Section, f: &FileFacts) -> Vec<MenuItem<RefAct>> {
    let c = f.click;
    let (p, files) = (&c.path, &f.files);
    let mut out = Vec::new();
    if files.is_empty() {
        return out;
    }
    let what = if c.folder { "folder" } else { "file" };
    match section {
        Section::Conflicted => {
            if !c.folder {
                out.push(file(format!("file-take-current-{p}"), "Take current", FileAct::Take(paths(files), Side::Current)));
                out.push(file(format!("file-take-incoming-{p}"), "Take incoming", FileAct::Take(paths(files), Side::Incoming)));
            }
            let words = if c.folder { "Mark all resolved" } else { "Mark resolved" };
            out.push(file(format!("file-resolve-{p}"), words, FileAct::Resolve(paths(files))));
        }
        Section::Staged if f.under_way && !c.folder => out.push(file(format!("file-unresolve-{p}"), "Mark conflicted", FileAct::Unresolve(paths(files)))),
        Section::Staged => {
            out.push(file(format!("file-unstage-{p}"), format!("Unstage {what}"), FileAct::Unstage(files.clone())));
            let words = if c.folder { "Discard all in folder" } else { "Discard changes" };
            out.push(file(format!("file-discard-{p}"), words, FileAct::DiscardAll(paths(files))));
        }
        Section::Unstaged => {
            out.push(file(format!("file-stage-{p}"), format!("Stage {what}"), FileAct::Stage(files.clone())));
            let words = if c.folder { "Discard all in folder" } else { "Discard changes" };
            out.push(file(format!("file-discard-{p}"), words, FileAct::Discard(files.clone())));
        }
    }
    let conflicted = section == Section::Conflicted;
    if ignorable(p) {
        let tracked: Vec<String> = files.iter().filter(|f| f.kind != ChangeKind::Untracked && !conflicted).map(|f| f.path.clone()).collect();
        let choices = Ignore::choices(p, c.folder).into_iter().enumerate().map(|(i, ignore)| {
            let untrack = if c.folder && !tracked.is_empty() { vec![p.clone()] } else { tracked.clone() };
            file(format!("file-ignore-{i}-{p}"), ignore.words(), FileAct::Ignore { ignore, untrack })
        });
        out.extend(MenuItem::submenu(format!("file-ignore-{p}"), "Ignore", choices.collect()).map(MenuItem::rule));
    }
    if !conflicted && !f.under_way {
        let it = file(format!("file-stash-{p}"), format!("Stash {what}"), FileAct::Stash(files.clone()));
        out.push(if ignorable(p) { it.rule() } else { it });
    }
    out.into_iter().map(|it| write(f, it)).collect()
}

fn commit_items(sha: &str, stash: Option<&str>, f: &FileFacts) -> Vec<MenuItem<RefAct>> {
    let c = f.click;
    if f.files.is_empty() {
        return Vec::new();
    }
    let what = if c.folder { "folder" } else { "file" };
    match stash {
        Some(stash) => {
            let apply = file(format!("file-apply-{}", c.path), format!("Apply stashed {what}"), FileAct::ApplyStashed { sha: stash.to_string(), files: f.files.clone() });
            vec![write(f, apply)]
        }
        None => vec![write(f, file(format!("file-restore-{}", c.path), format!("Restore {what} from this commit"), FileAct::Restore { sha: sha.to_string(), paths: paths(&f.files) }))],
    }
}

#[cfg(test)]
mod tests {
    use gpui::point;

    use super::*;

    fn change(kind: ChangeKind, path: &str) -> FileChange {
        FileChange { kind, path: path.into(), old_path: None, similarity: None }
    }

    fn labels(from: Listed, path: &str, folder: bool, files: &[FileChange], exists: bool, under_way: bool) -> Vec<String> {
        let click = FileClick { from, path: path.into(), folder, anchor: "".into(), position: point(gpui::px(0.), gpui::px(0.)) };
        let facts = FileFacts { click: &click, files: covered(files, path, folder), exists, under_way, busy: None };
        let shown = |i: &MenuItem<RefAct>| format!("{}{}", i.section.as_ref().map_or(String::new(), |s| format!("| {s} ")), i.label);
        items(&facts).iter().map(|i| match i.submenu.as_slice() {
            [] => shown(i),
            children => format!("{} ▸ {}", shown(i), children.iter().map(shown).collect::<Vec<_>>().join(", ")),
        }).collect()
    }

    #[test]
    fn each_state_has_its_own_items() {
        let all = [change(ChangeKind::Modified, "src/a.rs"), change(ChangeKind::Untracked, "src/new.txt"), change(ChangeKind::Unmerged, "c.txt")];
        assert_eq!(labels(Listed::Wip(Section::Unstaged), "src/a.rs", false, &all, true, false), [
            "Stage file", "Discard changes",
            "|  Ignore ▸ Ignore this file, Ignore all *.rs files, Ignore all in src/",
            "|  Stash file",
            "|  Open with the default program", "Show in folder", "Copy path", "Delete file",
        ]);
        assert_eq!(labels(Listed::Wip(Section::Staged), "src", true, &all, true, false)[..2], ["Unstage folder", "Discard all in folder"]);
        assert_eq!(labels(Listed::Wip(Section::Staged), "src/a.rs", false, &all, true, true)[0], "Mark conflicted", "a resolved file during a merge");
        let conflicted = labels(Listed::Wip(Section::Conflicted), "c.txt", false, &all, true, true);
        assert_eq!(conflicted[..4], ["Take current", "Take incoming", "Mark resolved", "|  Ignore ▸ Ignore this file, Ignore all *.txt files"]);
        assert!(!conflicted.iter().any(|l| l.contains("Stash")), "no stash of a conflicted file");
        assert_eq!(labels(Listed::Wip(Section::Unstaged), "src/a.rs", false, &all, false, false).last().map(String::as_str), Some("|  Copy path"), "a deleted file has nothing to open or delete");
    }

    #[test]
    fn a_commits_file_restores_and_a_stashs_applies() {
        let all = [change(ChangeKind::Modified, "a.txt")];
        let commit = Listed::Commit { sha: "c".into(), stash: None };
        assert_eq!(labels(commit, "a.txt", false, &all, true, false), ["Restore file from this commit", "|  Open with the default program", "Show in folder", "Copy path", "Delete file"]);
        let stash = Listed::Commit { sha: "i".into(), stash: Some("s".into()) };
        assert_eq!(labels(stash, "a.txt", false, &all, false, false), ["Apply stashed file", "|  Copy path"]);
        assert_eq!(labels(Listed::Commit { sha: "c".into(), stash: None }, "other.txt", false, &all, true, false)[0], "Open with the default program", "an unchanged file of View all files");
    }

    #[test]
    fn a_stashs_apply_is_dimmed_with_the_gates_why_while_busy() {
        let all = [change(ChangeKind::Modified, "a.txt")];
        let click = FileClick { from: Listed::Commit { sha: "i".into(), stash: Some("s".into()) }, path: "a.txt".into(), folder: false, anchor: "".into(), position: point(gpui::px(0.), gpui::px(0.)) };
        let dimmed = |busy| items(&FileFacts { click: &click, files: all.to_vec(), exists: false, under_way: false, busy })[0].dimmed.clone();
        assert_eq!(dimmed(Some("Working…")).as_deref(), Some("Working…"));
        assert_eq!(dimmed(None), None);
    }

    #[test]
    fn every_write_item_is_dimmed_with_the_gates_why_while_busy_and_the_rest_stay_live() {
        let all = [change(ChangeKind::Modified, "src/a.rs"), change(ChangeKind::Unmerged, "c.txt")];
        let commit = Listed::Commit { sha: "c".into(), stash: None };
        let cases = [(Listed::Wip(Section::Unstaged), "src/a.rs", false), (Listed::Wip(Section::Staged), "src", true), (Listed::Wip(Section::Conflicted), "c.txt", false), (commit, "src/a.rs", false)];
        let live = ["Open with the default program", "Open folder", "Show in folder", "Copy path"];
        for (from, path, folder) in cases {
            let click = FileClick { from, path: path.into(), folder, anchor: "".into(), position: point(gpui::px(0.), gpui::px(0.)) };
            let shown = |busy| items(&FileFacts { click: &click, files: covered(&all, path, folder), exists: true, under_way: false, busy });
            assert!(shown(None).iter().all(|i| i.dimmed.is_none()), "{path}: nothing is dimmed while idle");
            for i in shown(Some("Working…")) {
                let want = (!live.contains(&i.label.as_str())).then_some("Working…");
                assert_eq!(i.dimmed.as_deref(), want, "{path}: {}", i.label);
            }
        }
    }
}

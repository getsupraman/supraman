//! `sm folder …` and `sm group …`: the sidebar's entries in `folders.toml`, through [`supraman_core::folders::edit`].

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use supraman_core::folders::{self, Entries, Entry, Place, Pos};
use supraman_core::paths::path_key;

use crate::{Failure, home, value};

fn index(raw: &str) -> Result<usize, Failure> {
    raw.parse().map_err(|_| Failure::Usage(format!("--at must be a whole number >= 0, got '{raw}'")))
}

/// `<dir>` made absolute; it must be a folder.
fn folder_arg(raw: &str) -> Result<PathBuf, Failure> {
    let path = std::path::absolute(raw).map_err(|e| Failure::Usage(format!("{raw}: {e}")))?;
    if !path.is_dir() {
        return Err(supraman_core::CoreError::NotADirectory(path).into());
    }
    Ok(path)
}

/// `[--group <name>] [--at <n>]` after the positional arguments.
fn place(args: &[&str], at_end: usize) -> Result<(Option<String>, usize), Failure> {
    let (mut group, mut at) = (None, at_end);
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--group" => group = Some(value(&mut it, "--group")?.to_string()),
            "--at" => at = index(value(&mut it, "--at")?)?,
            other => return Err(Failure::Usage(format!("unexpected argument '{other}'"))),
        }
    }
    Ok((group, at))
}

pub fn folder(args: &[&str]) -> Result<(), Failure> {
    match args {
        ["add", dir, rest @ ..] => {
            let path = folder_arg(dir)?;
            let (group, at) = place(rest, usize::MAX)?;
            let to = match group {
                Some(name) => Place::Group { name, index: at },
                None => Place::Top(at),
            };
            folders::edit(&home()?, |e| e.add_folder(&path, &to))?;
            println!("added {}", path.display());
            Ok(())
        }
        ["remove", dir, rest @ ..] => {
            let path = std::path::absolute(dir).map_err(|e| Failure::Usage(format!("{dir}: {e}")))?;
            let (group, _) = place(rest, 0)?;
            folders::edit(&home()?, |e| {
                let pos = position(e, &path, group.as_deref()).ok_or_else(|| supraman_core::CoreError::NoEntry(path.display().to_string()))?;
                e.remove_folder(pos)
            })?;
            println!("removed {}; no files were touched", path.display());
            Ok(())
        }
        ["list"] => {
            print!("{}", render(&folders::load(&home()?)?));
            Ok(())
        }
        _ => Err(Failure::Usage(format!("unrecognised arguments: folder {}", args.join(" ")))),
    }
}

pub fn group(args: &[&str]) -> Result<(), Failure> {
    let home = home()?;
    match args {
        ["add", name, rest @ ..] => {
            let (_, at) = place(rest, usize::MAX)?;
            folders::edit(&home, |e| e.add_group(name, at))?;
            println!("added group {name}");
        }
        ["rename", from, to] => {
            folders::edit(&home, |e| e.rename_group(from, to))?;
            println!("renamed group {from} to {to}");
        }
        ["remove", name, rest @ ..] => {
            let with_folders = match rest {
                [] => false,
                ["--with-folders"] => true,
                _ => return Err(Failure::Usage(format!("unexpected arguments: {}", rest.join(" ")))),
            };
            folders::edit(&home, |e| e.remove_group(name, with_folders))?;
            let folders = if with_folders { "with its folders" } else { "its folders moved to the top level" };
            println!("removed group {name}, {folders}");
        }
        _ => return Err(Failure::Usage(format!("unrecognised arguments: group {}", args.join(" ")))),
    }
    Ok(())
}

/// Where `path` is listed: in the group named, else at the top level, else in the first group listing it.
fn position(entries: &Entries, path: &Path, group: Option<&str>) -> Option<Pos> {
    let key = path_key(path);
    let same = |p: &Path| path_key(p) == key;
    let in_group = |(i, e): (usize, &Entry)| match e {
        Entry::Group { name, folders } if group.is_none_or(|g| g == name) => folders.iter().position(|f| same(&f.path)).map(|f| Pos::member(i, f)),
        _ => None,
    };
    let top = || entries.list.iter().position(|e| matches!(e, Entry::Folder(f) if same(&f.path))).map(Pos::top);
    match group {
        Some(_) => entries.list.iter().enumerate().find_map(in_group),
        None => top().or_else(|| entries.list.iter().enumerate().find_map(in_group)),
    }
}

/// The entries in order: a top-level folder by its path, a group by its name with its folders indented.
pub(crate) fn render(entries: &Entries) -> String {
    let mut out = String::new();
    if entries.list.is_empty() {
        out.push_str("no folders listed\n");
    }
    for entry in &entries.list {
        match entry {
            Entry::Folder(f) => {
                let _ = writeln!(out, "{}", f.path.display());
            }
            Entry::Group { name, folders } => {
                let _ = writeln!(out, "{name}/");
                for f in folders {
                    let _ = writeln!(out, "  {}", f.path.display());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use supraman_core::folders::FolderEntry;

    use super::*;

    #[test]
    fn a_path_is_found_where_it_is_listed() {
        let entries = Entries::new(vec![
            Entry::Group { name: "g".into(), folders: vec![FolderEntry::new(r"C:\a"), FolderEntry::new(r"C:\b")] },
            Entry::Folder(FolderEntry::new(r"C:\b")),
        ]);
        assert_eq!(position(&entries, Path::new(r"c:/B"), None), Some(Pos::top(1)), "the top level first");
        assert_eq!(position(&entries, Path::new(r"C:\b"), Some("g")), Some(Pos::member(0, 1)));
        assert_eq!(position(&entries, Path::new(r"C:\a"), None), Some(Pos::member(0, 0)));
        assert_eq!(position(&entries, Path::new(r"C:\x"), None), None);
        assert_eq!(render(&entries), "g/\n  C:\\a\n  C:\\b\nC:\\b\n");
        assert_eq!(render(&Entries::default()), "no folders listed\n");
    }
}

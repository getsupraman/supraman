//! `sm workspace new` and the edits of a registered workspace; the change itself is
//! [`supraman_core::Registry::edit`]'s, shared with the app's Edit folders.

use std::path::PathBuf;

use supraman_core::{CoreError, DEFAULT_DEPTH, Edit, Edited, Registry, Workspace, validate_name};

use crate::{Failure, parse_depth, print_workspace, value, with_owners};

pub fn new(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (mut name, mut home, mut folders) = (None, None, Vec::new());
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--home" => home = Some(PathBuf::from(value(&mut it, "--home")?)),
            "--folder" => folders.push(PathBuf::from(value(&mut it, "--folder")?)),
            flag if flag.starts_with("--") => return Err(Failure::Usage(format!("unknown option {flag}"))),
            positional if name.is_none() => name = Some(positional.to_string()),
            extra => return Err(Failure::Usage(format!("unexpected argument '{extra}'"))),
        }
    }
    let name = name.ok_or_else(|| Failure::Usage("workspace new needs a <name>".into()))?;
    validate_name(&name)?;
    for dir in home.iter().chain(&folders) {
        if !dir.is_dir() {
            return Err(CoreError::NotADirectory(dir.clone()).into());
        }
    }
    let mut empty = Workspace::new(name.as_str(), PathBuf::new(), Vec::new());
    empty.home = None;
    registry.check_new(&empty)?;
    registry.save(&empty, false)?;
    let mut last = Edited { workspace: empty, added: Vec::new(), removed: Vec::new(), claimed: Vec::new() };
    for dir in folders {
        last = registry.edit(&name, Edit::AddFolder { path: dir, enable: true })?;
        print_notices(&last);
    }
    if let Some(home) = home {
        last = registry.edit(&name, Edit::SetHome(Some(home)))?;
    }
    println!("registered workspace {name} at {}", registry.path_of(&name)?.display());
    print_workspace(&last.workspace, true, &with_owners(registry, &last.workspace)?);
    Ok(())
}

pub fn add_folder(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, dir, enable) = match args {
        [w, dir] => (*w, *dir, false),
        [w, dir, "--enable"] | [w, "--enable", dir] => (*w, *dir, true),
        _ => return Err(Failure::Usage("add-folder needs <name> <dir> [--enable]".into())),
    };
    let done = registry.edit(w, Edit::AddFolder { path: PathBuf::from(dir), enable })?;
    if done.added.is_empty() {
        println!("{dir} is already a folder of {w}");
    }
    finish(registry, done)
}

pub fn remove_folder(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let [w, folder] = args else {
        return Err(Failure::Usage("remove-folder needs <name> <folder>".into()));
    };
    finish(registry, registry.edit(w, Edit::RemoveFolder(folder.to_string()))?)
}

pub fn set_enabled(registry: &Registry, enabled: bool, args: &[&str]) -> Result<(), Failure> {
    let [w, folder] = args else {
        return Err(Failure::Usage(format!("{} needs <name> <folder>", if enabled { "enable" } else { "disable" })));
    };
    finish(registry, registry.edit(w, Edit::SetEnabled { folder: folder.to_string(), enabled })?)
}

pub fn set_home(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let [w, dir] = args else {
        return Err(Failure::Usage("set-home needs <name> <dir>".into()));
    };
    finish(registry, registry.edit(w, Edit::SetHome(Some(PathBuf::from(dir))))?)
}

pub fn add_scan(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, dir, depth) = match args {
        [w, dir] => (*w, *dir, DEFAULT_DEPTH),
        [w, dir, "--depth", n] => (*w, *dir, parse_depth(n)?),
        _ => return Err(Failure::Usage("add-scan needs <name> <dir> [--depth N]".into())),
    };
    finish(registry, registry.edit(w, Edit::AddScan { path: PathBuf::from(dir), depth })?)
}

pub fn remove_scan(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let [w, dir] = args else {
        return Err(Failure::Usage("remove-scan needs <name> <dir>".into()));
    };
    finish(registry, registry.edit(w, Edit::RemoveScan(PathBuf::from(dir)))?)
}

fn print_notices(done: &Edited) {
    for (folder, owner) in &done.claimed {
        println!("notice: {folder} is supervised in workspace {owner}; disabled here");
    }
}

fn finish(registry: &Registry, done: Edited) -> Result<(), Failure> {
    print_notices(&done);
    for f in &done.added {
        println!("added folder {f}");
    }
    for f in &done.removed {
        println!("removed folder {} ({})", f.name, f.path.display());
    }
    print_workspace(&done.workspace, true, &with_owners(registry, &done.workspace)?);
    Ok(())
}

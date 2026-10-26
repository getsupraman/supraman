//! `fm plugin install <dir>` and the app's start: put `fm.exe` at `<dir>/bin/fm.exe`, where the exec-form
//! hooks in `plugins/foreman/hooks/hooks.json` find it via `${CLAUDE_PLUGIN_ROOT}`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const HOOK_EXE: &str = "fm.exe";

/// A plugin dir is one with the hooks and the role templates.
fn is_plugin(dir: &Path) -> bool {
    dir.join("hooks").join("hooks.json").is_file() && dir.join("roles").is_dir()
}

/// The Foreman plugin for `exe` (`foreman.exe` or `fm.exe`), in this order (docs/RELEASE.md):
/// 1. `<exe dir>/plugins/foreman`, the release layout;
/// 2. the plugin the exe sits in, for the hooks' copy at `<plugin>/bin/fm.exe`;
/// 3. `plugins/foreman` in any folder above, a dev build at `<repo>/target/<profile>/`.
///
/// The error names every place it looked.
pub fn find_plugin_dir(exe: &Path) -> Result<PathBuf, String> {
    let Some(exe_dir) = exe.parent() else {
        return Err(format!("{} has no folder to look for plugins/foreman in", exe.display()));
    };
    let beside = exe_dir.join("plugins").join("foreman");
    if is_plugin(&beside) {
        return Ok(beside);
    }
    if exe_dir.file_name().is_some_and(|n| n == "bin")
        && let Some(own) = exe_dir.parent().filter(|d| is_plugin(d))
    {
        return Ok(own.to_path_buf());
    }
    exe_dir
        .ancestors()
        .skip(1)
        .map(|dir| dir.join("plugins").join("foreman"))
        .find(|dir| is_plugin(dir))
        .ok_or_else(|| {
            format!(
                r"No Foreman plugin for {}: looked next to it at {} (a release) and for plugins\foreman in every folder above it (a dev build)",
                exe.display(),
                beside.display()
            )
        })
}

/// Copies `exe` to `<plugin_dir>/bin/fm.exe` unless it is already byte-identical. A hook may be
/// running the old file, which Windows will not let us overwrite but will let us rename, so the new
/// copy is written beside it and swapped in by renames; the old one is left as `fm.exe.old`.
pub fn install_hook_exe(plugin_dir: &Path, exe: &Path) -> io::Result<(PathBuf, bool)> {
    if !plugin_dir.join("hooks").join("hooks.json").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} has no hooks/hooks.json", plugin_dir.display()),
        ));
    }
    let bin = plugin_dir.join("bin");
    fs::create_dir_all(&bin)?;
    let target = bin.join(HOOK_EXE);
    let bytes = fs::read(exe)?;
    if fs::read(&target).is_ok_and(|old| old == bytes) {
        return Ok((target, false));
    }
    let fresh = bin.join(format!("{HOOK_EXE}.new"));
    fs::write(&fresh, &bytes)?;
    if target.exists() {
        let old = bin.join(format!("{HOOK_EXE}.old"));
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old)?;
    }
    fs::rename(&fresh, &target)?;
    Ok((target, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_at(dir: &Path) {
        fs::create_dir_all(dir.join("hooks")).unwrap();
        fs::create_dir_all(dir.join("roles")).unwrap();
        fs::write(dir.join("hooks").join("hooks.json"), "{}").unwrap();
    }

    #[test]
    fn finds_the_plugin_beside_the_exe_first_then_its_own_then_above() {
        let root = tempfile::tempdir().unwrap();
        let repo_plugin = root.path().join("plugins").join("foreman");
        plugin_at(&repo_plugin);
        let release = root.path().join("target").join("dist").join("Foreman");
        let exe = release.join("foreman.exe");
        assert_eq!(find_plugin_dir(&exe).unwrap(), repo_plugin, "a dev build walks up");

        let beside = release.join("plugins").join("foreman");
        plugin_at(&beside);
        assert_eq!(find_plugin_dir(&exe).unwrap(), beside, "beside the exe wins over the walk");
        assert_eq!(find_plugin_dir(&beside.join("bin").join(HOOK_EXE)).unwrap(), beside, "the hooks' copy");
    }

    #[test]
    fn a_missing_plugin_names_both_places() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("plugins").join("foreman").join("hooks")).unwrap();
        let exe = root.path().join("Foreman").join("foreman.exe");
        let err = find_plugin_dir(&exe).unwrap_err();
        assert!(err.contains(&root.path().join("Foreman").join("plugins").join("foreman").display().to_string()), "{err}");
        assert!(err.contains("every folder above"), "{err}");
    }

    #[test]
    fn installs_replaces_and_skips_identical() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("p");
        let exe = dir.path().join("src.exe");
        fs::write(&exe, b"one").unwrap();
        assert!(install_hook_exe(&plugin, &exe).is_err());
        fs::create_dir_all(plugin.join("hooks")).unwrap();
        fs::write(plugin.join("hooks").join("hooks.json"), "{}").unwrap();

        let (target, copied) = install_hook_exe(&plugin, &exe).unwrap();
        assert!(copied);
        assert_eq!(fs::read(&target).unwrap(), b"one");
        assert!(!install_hook_exe(&plugin, &exe).unwrap().1);
        fs::write(&exe, b"two").unwrap();
        assert!(install_hook_exe(&plugin, &exe).unwrap().1);
        assert_eq!(fs::read(&target).unwrap(), b"two");
        assert_eq!(fs::read(plugin.join("bin").join("fm.exe.old")).unwrap(), b"one");
    }
}

//! `sm plugin install <dir>` and the app's start: put `sm.exe` at `<dir>/bin/sm.exe`, where the exec-form
//! hooks in `plugins/supraman/hooks/hooks.json` find it via `${CLAUDE_PLUGIN_ROOT}`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const HOOK_EXE: &str = "sm.exe";

/// A copy of [`HOOK_EXE`] for prompts, skills and sessions that still say `fm`; its end is in docs/RELEASE.md.
pub const COMPAT_HOOK_EXE: &str = "fm.exe";

/// So `sm` (and `fm`, [`COMPAT_HOOK_EXE`]) never parks: by name on the session's PATH, or by its absolute path.
pub fn sm_allowed_tools(plugin_dir: &Path) -> Vec<String> {
    let rule = |exe: &str| format!("Bash({}:*)", crate::paths::normalize(&plugin_dir.join("bin").join(exe)));
    vec!["Bash(sm:*)".into(), rule(HOOK_EXE), "Bash(fm:*)".into(), rule(COMPAT_HOOK_EXE)]
}

fn is_plugin(dir: &Path) -> bool {
    dir.join("hooks").join("hooks.json").is_file() && dir.join("roles").is_dir()
}

/// The Supraman plugin for `exe` (`supraman.exe` or `sm.exe`), in this order (docs/RELEASE.md):
/// 1. `<exe dir>/plugins/supraman`, the release layout;
/// 2. the plugin the exe sits in, for the hooks' copy at `<plugin>/bin/sm.exe`;
/// 3. `plugins/supraman` in any folder above, a dev build at `<repo>/target/<profile>/`.
///
/// The error names every place it looked.
pub fn find_plugin_dir(exe: &Path) -> Result<PathBuf, String> {
    let Some(exe_dir) = exe.parent() else {
        return Err(format!("{} has no folder to look for plugins/supraman in", exe.display()));
    };
    let beside = exe_dir.join("plugins").join("supraman");
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
        .map(|dir| dir.join("plugins").join("supraman"))
        .find(|dir| is_plugin(dir))
        .ok_or_else(|| {
            format!(
                r"No Supraman plugin for {}: looked next to it at {} (a release) and for plugins\supraman in every folder above it (a dev build)",
                exe.display(),
                beside.display()
            )
        })
}

/// Copies `exe` to `<plugin_dir>/bin/sm.exe` and `bin/fm.exe` ([`COMPAT_HOOK_EXE`]), each unless it is
/// already byte-identical; the path of `sm.exe`, and whether either was copied. A hook may be running
/// the old file, which Windows will not let us overwrite but will let us rename, so the new copy is
/// written beside it and swapped in by renames; the old one is left as `<name>.old`.
pub fn install_hook_exe(plugin_dir: &Path, exe: &Path) -> io::Result<(PathBuf, bool)> {
    if !plugin_dir.join("hooks").join("hooks.json").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} has no hooks/hooks.json", plugin_dir.display()),
        ));
    }
    let bin = plugin_dir.join("bin");
    fs::create_dir_all(&bin)?;
    let bytes = fs::read(exe)?;
    let copied = place(&bin, HOOK_EXE, &bytes)? | place(&bin, COMPAT_HOOK_EXE, &bytes)?;
    Ok((bin.join(HOOK_EXE), copied))
}

fn place(bin: &Path, name: &str, bytes: &[u8]) -> io::Result<bool> {
    let target = bin.join(name);
    if fs::read(&target).is_ok_and(|old| old == bytes) {
        return Ok(false);
    }
    let fresh = bin.join(format!("{name}.new"));
    fs::write(&fresh, bytes)?;
    if target.exists() {
        let old = bin.join(format!("{name}.old"));
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old)?;
    }
    fs::rename(&fresh, &target)?;
    Ok(true)
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
        let repo_plugin = root.path().join("plugins").join("supraman");
        plugin_at(&repo_plugin);
        let release = root.path().join("target").join("dist").join("Supraman");
        let exe = release.join("supraman.exe");
        assert_eq!(find_plugin_dir(&exe).unwrap(), repo_plugin, "a dev build walks up");

        let beside = release.join("plugins").join("supraman");
        plugin_at(&beside);
        assert_eq!(find_plugin_dir(&exe).unwrap(), beside, "beside the exe wins over the walk");
        assert_eq!(find_plugin_dir(&beside.join("bin").join(HOOK_EXE)).unwrap(), beside, "the hooks' copy");
    }

    #[test]
    fn a_missing_plugin_names_both_places() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("plugins").join("supraman").join("hooks")).unwrap();
        let exe = root.path().join("Supraman").join("supraman.exe");
        let err = find_plugin_dir(&exe).unwrap_err();
        assert!(err.starts_with("No Supraman plugin for "), "{err}");
        assert!(err.contains(&root.path().join("Supraman").join("plugins").join("supraman").display().to_string()), "{err}");
        assert!(err.contains("every folder above"), "{err}");
    }

    #[test]
    fn the_old_plugin_name_is_never_loaded() {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("Tools").join("supraman");
        plugin_at(&install.join("plugins").join("foreman"));
        plugin_at(&root.path().join("plugins").join("foreman"));
        assert!(find_plugin_dir(&install.join("supraman.exe")).is_err(), "an old install's plugins/foreman is the old sessions'");
        let new = install.join("plugins").join("supraman");
        plugin_at(&new);
        assert_eq!(find_plugin_dir(&install.join("supraman.exe")).unwrap(), new);
        assert_eq!(find_plugin_dir(&install.join("sm.exe")).unwrap(), new);
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
        assert_eq!(fs::read(plugin.join("bin").join("sm.exe.old")).unwrap(), b"one");
    }

    #[test]
    fn the_bin_holds_sm_and_the_compatibility_fm() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("p");
        plugin_at(&plugin);
        let exe = dir.path().join("sm.exe");
        fs::write(&exe, b"one").unwrap();
        let (target, _) = install_hook_exe(&plugin, &exe).unwrap();
        let bin = plugin.join("bin");
        assert_eq!(target, bin.join("sm.exe"));
        assert_eq!(fs::read(bin.join("sm.exe")).unwrap(), b"one");
        assert_eq!(fs::read(bin.join("fm.exe")).unwrap(), b"one");

        fs::remove_file(bin.join("fm.exe")).unwrap();
        assert!(install_hook_exe(&plugin, &exe).unwrap().1, "a missing fm.exe alone is copied again");
        assert_eq!(fs::read(bin.join("fm.exe")).unwrap(), b"one");
        fs::write(&exe, b"two").unwrap();
        install_hook_exe(&plugin, &exe).unwrap();
        assert_eq!((fs::read(bin.join("fm.exe")).unwrap(), fs::read(bin.join("fm.exe.old")).unwrap()), (b"two".to_vec(), b"one".to_vec()));
        assert_eq!(find_plugin_dir(&bin.join("fm.exe")).unwrap(), plugin, "the compatibility copy finds its plugin");
    }

    #[test]
    fn allowed_tools_name_sm_and_fm() {
        assert_eq!(
            sm_allowed_tools(Path::new(r"C:\x\plugins\supraman")),
            ["Bash(sm:*)", "Bash(C:/x/plugins/supraman/bin/sm.exe:*)", "Bash(fm:*)", "Bash(C:/x/plugins/supraman/bin/fm.exe:*)"]
        );
    }
}

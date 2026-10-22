//! `fm plugin install <dir>`: puts the running `fm.exe` at `<dir>/bin/fm.exe`, where the exec-form
//! hooks in `plugins/foreman/hooks/hooks.json` find it via `${CLAUDE_PLUGIN_ROOT}`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const HOOK_EXE: &str = "fm.exe";

/// Copies `exe` to `<plugin_dir>/bin/fm.exe` unless it is already byte-identical. A hook may be
/// running the old file, which Windows will not let us overwrite but will let us rename, so the new
/// copy is written beside it and swapped in by renames; the old one is left as `fm.exe.old`.
pub fn install(plugin_dir: &Path, exe: &Path) -> io::Result<(PathBuf, bool)> {
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

    #[test]
    fn installs_replaces_and_skips_identical() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("p");
        let exe = dir.path().join("src.exe");
        fs::write(&exe, b"one").unwrap();
        assert!(install(&plugin, &exe).is_err());
        fs::create_dir_all(plugin.join("hooks")).unwrap();
        fs::write(plugin.join("hooks").join("hooks.json"), "{}").unwrap();

        let (target, copied) = install(&plugin, &exe).unwrap();
        assert!(copied);
        assert_eq!(fs::read(&target).unwrap(), b"one");
        assert!(!install(&plugin, &exe).unwrap().1);
        fs::write(&exe, b"two").unwrap();
        assert!(install(&plugin, &exe).unwrap().1);
        assert_eq!(fs::read(&target).unwrap(), b"two");
        assert_eq!(fs::read(plugin.join("bin").join("fm.exe.old")).unwrap(), b"one");
    }
}

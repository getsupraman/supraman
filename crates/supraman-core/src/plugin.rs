//! `sm.exe` at `<plugin>/bin/sm.exe`, where `hooks/hooks.json` finds it via `${CLAUDE_PLUGIN_ROOT}`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `sm.exe`, the hooks' copy's file name.
pub fn hook_exe() -> String {
    crate::platform::fs::exe_name("sm")
}

/// The key of the workman plugin's MCP server in a supervised or scheduled session's own config.
pub const AGENTS_SERVER: &str = "agents";

/// The `agents` tools a session runs unattended: nested agents' launch, list and stop, never sign-in or sign-out.
pub const AGENT_TOOLS: [&str; 3] = ["agent_launch", "agent_list", "agent_stop"];

/// So `sm` never parks (by name on the session's PATH, or by its absolute path), nor a nested agent's launch.
pub fn sm_allowed_tools(plugin_dir: &Path) -> Vec<String> {
    let sm = ["Bash(sm:*)".to_string(), format!("Bash({}:*)", crate::paths::normalize(&plugin_dir.join("bin").join(hook_exe())))];
    sm.into_iter().chain(AGENT_TOOLS.map(|tool| format!("mcp__{AGENTS_SERVER}__{tool}"))).collect()
}

fn is_plugin(dir: &Path) -> bool {
    dir.join("hooks").join("hooks.json").is_file() && dir.join("roles").is_dir()
}

/// The Supraman plugin beside `exe`, else the one it sits in, else `plugins/supraman` above it (docs/RELEASE.md).
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

/// `exe` copied to `<plugin_dir>/bin/sm.exe` by renames (a running hook holds the old one, left as `.old`).
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
    let copied = place(&bin, &hook_exe(), &bytes)?;
    Ok((bin.join(hook_exe()), copied))
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
        assert_eq!(find_plugin_dir(&beside.join("bin").join(hook_exe())).unwrap(), beside, "the hooks' copy");
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
    fn both_exes_find_the_release_layouts_plugin() {
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("Tools").join("supraman");
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
    fn the_bin_holds_only_sm() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("p");
        plugin_at(&plugin);
        let exe = dir.path().join("sm.exe");
        fs::write(&exe, b"one").unwrap();
        let (target, _) = install_hook_exe(&plugin, &exe).unwrap();
        let bin = plugin.join("bin");
        assert_eq!(target, bin.join("sm.exe"));
        let names: Vec<_> = fs::read_dir(&bin).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["sm.exe"]);
        assert_eq!(find_plugin_dir(&target).unwrap(), plugin, "the hooks' copy finds its plugin");
    }

    #[test]
    fn allowed_tools_name_sm() {
        assert_eq!(
            sm_allowed_tools(Path::new(r"C:\x\plugins\supraman")),
            ["Bash(sm:*)", "Bash(C:/x/plugins/supraman/bin/sm.exe:*)", "mcp__agents__agent_launch", "mcp__agents__agent_list", "mcp__agents__agent_stop"]
        );
    }
}

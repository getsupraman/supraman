//! The Windows shell's lookups: `PATHEXT`, `%NAME%`, PowerShell, Git for Windows; [`detect`] lists a tab's shells.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::paths::{executable, folders_holding, process_path};

mod detect;
pub mod launch;

pub use detect::{Machine, PROBE_TIMEOUT, VsInstance, detect, detect_on, parse_vswhere, parse_wsl};

/// `s` as a single-quoted PowerShell string.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `%NAME%` replaced by that environment variable; a name that is not set stays as written.
pub fn expand_env(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('%') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('%').map(|close| (&after[..close], close)) {
            Some((name, close)) if !name.is_empty() && let Some(value) = std::env::var_os(name) => {
                out.push_str(&value.to_string_lossy());
                rest = &after[close + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// This process's `PATHEXT`, Windows' default when unset or empty.
pub fn pathext() -> String {
    pathext_or_default(std::env::var("PATHEXT").ok())
}

fn pathext_or_default(value: Option<String>) -> String {
    value.filter(|v| !v.trim().is_empty()).unwrap_or_else(|| DEFAULT_PATHEXT.into())
}

/// A PowerShell to start: its absolute `path`, and its short `name` (`pwsh` or `powershell`) for a tab's title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerShell {
    pub path: PathBuf,
    pub name: &'static str,
    /// An App Execution Alias: the process it starts is no descendant of its caller, so a job object misses it.
    pub alias: bool,
}

/// Windows PowerShell under `system_root` (`%SystemRoot%`, `C:\Windows` when unset).
pub fn windows_powershell(system_root: Option<&std::ffi::OsStr>) -> PowerShell {
    let root = system_root.filter(|r| !r.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    PowerShell { path: root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"), name: "powershell", alias: false }
}

/// The first `PATH` folder's `pwsh` with a `PATHEXT` extension, else [`windows_powershell`]; never a bare name.
pub fn powershell(path: &std::ffi::OsStr, pathext: &str, system_root: Option<&std::ffi::OsStr>) -> PowerShell {
    match executable("pwsh", path, pathext) {
        Some(path) => PowerShell { alias: crate::platform::fs::is_app_alias(&path), path, name: "pwsh" },
        None => windows_powershell(system_root),
    }
}

/// [`powershell`], with Windows PowerShell in place of a pwsh alias: for a child whose tree a timeout kills.
pub fn job_powershell(path: &std::ffi::OsStr, pathext: &str, system_root: Option<&std::ffi::OsStr>) -> PowerShell {
    let found = powershell(path, pathext, system_root);
    if found.alias { windows_powershell(system_root) } else { found }
}

/// This process's `PATH`, [`pathext`] and `SystemRoot`.
pub fn process_env() -> (OsString, String, Option<OsString>) {
    (process_path(), pathext(), std::env::var_os("SystemRoot"))
}

/// This process's `ProgramFiles(x86)`.
#[cfg(windows)]
pub fn program_files_x86() -> Option<OsString> {
    std::env::var_os("ProgramFiles(x86)")
}

#[cfg(not(windows))]
pub fn program_files_x86() -> Option<OsString> {
    None
}

/// Where Git for Windows may be installed: the usual folders and the one holding `git.exe` on this process's `PATH`.
#[cfg(windows)]
pub fn git_roots() -> Vec<PathBuf> {
    let program_files = [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramW6432"), program_files_x86()];
    let mut roots: Vec<PathBuf> = program_files.into_iter().flatten().map(|d| PathBuf::from(d).join("Git")).collect();
    roots.extend(std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs").join("Git")));
    roots.extend(std::env::var_os("EXEPATH").map(PathBuf::from));
    for dir in folders_holding(&crate::platform::fs::exe_name("git"), &process_path()) {
        let up = dir.parent().map(Path::to_path_buf);
        let mingw = up.as_deref().filter(|u| u.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("mingw")));
        roots.extend(mingw.and_then(Path::parent).map(Path::to_path_buf).or(up));
    }
    roots
}

#[cfg(not(windows))]
pub fn git_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// Whether this process runs under Git Bash (`MSYSTEM` set), whose argument conversion rewrites `/name` into a path.
#[cfg(windows)]
pub fn in_git_bash() -> bool {
    std::env::var_os("MSYSTEM").is_some()
}

#[cfg(not(windows))]
pub fn in_git_bash() -> bool {
    false
}

/// [`powershell`] for this process's `PATH`, `PATHEXT` and `SystemRoot`, looked up once: every terminal tab's.
pub fn terminal_powershell() -> &'static PowerShell {
    static SHELL: std::sync::OnceLock<PowerShell> = std::sync::OnceLock::new();
    SHELL.get_or_init(|| {
        let (path, pathext, root) = process_env();
        powershell(&path, &pathext, root.as_deref())
    })
}

/// [`job_powershell`] for this process's `PATH`, `PATHEXT` and `SystemRoot`: a schedule's `.ps1` script's.
pub fn script_powershell() -> PowerShell {
    let (path, pathext, root) = process_env();
    job_powershell(&path, &pathext, root.as_deref())
}

/// The command that runs the script at `path` per its extension (the script contract's table).
pub fn script_command(path: &Path, args: &[String]) -> std::process::Command {
    use crate::process;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "ps1" => {
            let mut cmd = process::command(script_powershell().path);
            cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(path).args(args);
            cmd
        }
        "cmd" | "bat" => {
            let mut cmd = process::command("cmd");
            let line =
                std::iter::once(path.to_string_lossy().into_owned()).chain(args.iter().cloned()).map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(" ");
            raw(&mut cmd, &format!("/d /s /c \"{line}\""));
            cmd
        }
        _ => {
            let mut cmd = process::command(path);
            cmd.args(args);
            cmd
        }
    }
}

/// cmd's `/s /c "…"` line is not parsed by the MSVC rules `Command::arg` quotes for.
#[cfg(windows)]
fn raw(cmd: &mut std::process::Command, line: &str) {
    use std::os::windows::process::CommandExt as _;
    cmd.raw_arg(line);
}

#[cfg(not(windows))]
fn raw(cmd: &mut std::process::Command, line: &str) {
    cmd.arg(line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn quote_doubles_each_single_quote_inside_single_quotes() {
        assert_eq!(quote(r"C:\it's here"), r"'C:\it''s here'");    }

    #[test]
    fn powershell_is_pwsh_only_when_path_holds_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (seven, script, empty) = (tmp.path().join("seven"), tmp.path().join("script"), tmp.path().join("empty"));
        for d in [&seven, &script, &empty] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(seven.join("pwsh.exe"), "").unwrap();
        fs::write(script.join("pwsh.ps1"), "").unwrap();
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
        let ext = ".COM;.EXE;.BAT;.CMD";
        let root = tmp.path().join("Windows");
        let windows = PowerShell { path: root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"), name: "powershell", alias: false };
        let shell = |p: &std::ffi::OsStr, ext: &str| powershell(p, ext, Some(root.as_os_str()));
        assert_eq!(shell(&path(&[&empty, &seven]), ext), PowerShell { path: seven.join("pwsh.exe"), name: "pwsh", alias: false });
        assert_eq!(shell(&path(&[&empty]), ext), windows);
        assert_eq!(shell(&path(&[&script]), ext), windows, "a pwsh.ps1 is not PowerShell 7");
        assert_eq!(shell(&path(&[&seven]), ".CMD"), windows, "only PATHEXT extensions");
        assert_eq!(job_powershell(&path(&[&seven]), ext, Some(root.as_os_str())).path, seven.join("pwsh.exe"), "a real pwsh runs a script too");
    }

    #[test]
    fn powershell_without_pwsh_is_windows_powershell_by_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Windows");
        let expected = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let ext = ".COM;.EXE;.BAT;.CMD";
        let found = powershell(std::ffi::OsStr::new(""), ext, Some(root.as_os_str()));
        assert_eq!((found.path.as_path(), found.name), (expected.as_path(), "powershell"), "an empty PATH");
        assert!(found.path.is_absolute());
        assert_eq!(powershell(std::ffi::OsStr::new(""), ext, None).path, Path::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"));
        assert_eq!(powershell(std::ffi::OsStr::new(""), ext, Some(std::ffi::OsStr::new(""))).path, Path::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"));
    }

    #[test]
    #[cfg(windows)]
    fn a_relative_path_folder_is_not_searched_for_pwsh() {
        let tmp = tempfile::tempdir().unwrap();
        let seven = tmp.path().join("seven");
        fs::create_dir_all(&seven).unwrap();
        fs::write(seven.join("pwsh.exe"), "").unwrap();
        let driveless: PathBuf = seven.components().skip(1).collect();
        assert!(!driveless.is_absolute() && driveless.join("pwsh.exe").is_file(), "{} resolves on the current drive", driveless.display());
        let found = powershell(driveless.as_os_str(), ".EXE", Some(tmp.path().join("Windows").as_os_str()));
        assert_eq!(found.name, "powershell");
    }

    #[test]
    fn pathext_is_windows_default_when_unset_or_empty() {
        assert_eq!(pathext_or_default(None), ".COM;.EXE;.BAT;.CMD");
        assert_eq!(pathext_or_default(Some(" ".into())), ".COM;.EXE;.BAT;.CMD");
        assert_eq!(pathext_or_default(Some(".EXE;.PS1".into())), ".EXE;.PS1");
    }
}

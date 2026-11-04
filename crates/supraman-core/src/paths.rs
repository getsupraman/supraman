use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// `USERPROFILE`, else `HOME`; an empty value counts as unset.
pub fn user_home() -> Option<PathBuf> {
    home_of(std::env::var_os("USERPROFILE"), std::env::var_os("HOME"))
}

fn home_of(userprofile: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let set = |v: Option<OsString>| v.filter(|v| !v.is_empty());
    set(userprofile).or_else(|| set(home)).map(PathBuf::from)
}

/// claude's config dir: `CLAUDE_CONFIG_DIR`, else `<user_home>/.claude`.
pub fn claude_dir() -> Option<PathBuf> {
    claude_dir_of(std::env::var_os("CLAUDE_CONFIG_DIR"), user_home())
}

fn claude_dir_of(config_dir: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    match config_dir.filter(|d| !d.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home.map(|h| h.join(".claude")),
    }
}

/// `/`-separated, no trailing `/`, `.`/`..` folded, no `\\?\`; case kept.
pub fn normalize(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s);
    let mut parts: Vec<&str> = Vec::new();
    for (i, part) in s.split('/').enumerate() {
        match part {
            "" if i > 0 => {}
            "." => {}
            ".." if parts.len() > 1 => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// [`normalize`], lowercased on Windows: equal keys name the same path.
pub fn path_key(path: &Path) -> String {
    let normal = normalize(path);
    if cfg!(windows) { normal.to_lowercase() } else { normal }
}

/// `dir` is `root` or below it, compared by [`path_key`]; a sibling sharing a prefix is not.
pub fn within(dir: &Path, root: &Path) -> bool {
    key_within(&path_key(dir), &path_key(root))
}

/// [`within`] on two [`path_key`]s.
pub fn key_within(dir: &str, root: &str) -> bool {
    dir == root || dir.strip_prefix(root).is_some_and(|rest| rest.starts_with('/') || root.ends_with('/'))
}

/// The git dir a linked worktree's `.git` file names (`gitdir: <path>`, relative to the file's folder).
pub fn linked_git_dir(git_file: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(git_file).ok()?;
    let dir = text.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))?.trim();
    Some(git_file.parent()?.join(dir))
}

/// The nearest `.git` at or above `dir`: a repo's folder, or a linked worktree's file. Never runs git.
pub fn git_at(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().map(|d| d.join(".git")).find(|g| g.exists())
}

/// The main checkout `dir` is in, [`normalize`]d: the folder of the [`git_at`] dir, or the
/// `<repo>` of a linked worktree's `gitdir: <repo>/.git/worktrees/<n>`. Never runs git.
pub fn main_checkout(dir: &Path) -> Option<PathBuf> {
    let git = git_at(dir)?;
    if git.is_dir() {
        return Some(PathBuf::from(normalize(git.parent()?)));
    }
    let mut gitdir = normalize(&linked_git_dir(&git)?);
    gitdir.truncate(gitdir.rfind("/.git/worktrees/")?);
    Some(PathBuf::from(gitdir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn within_is_the_folder_or_below_it_never_a_sibling_with_a_common_prefix() {
        let root = Path::new(r"P:\Projects\Gaia");
        assert!(within(root, root));
        assert!(within(Path::new(r"P:\Projects\Gaia\src\x"), root));
        assert!(!within(Path::new(r"P:\Projects\Gaia.Ext"), root));
        assert!(!within(Path::new(r"P:\Projects"), root));
        #[cfg(windows)]
        assert!(within(Path::new("p:/projects/gaia/SRC/"), root));
    }

    #[test]
    fn user_home_is_userprofile_else_home_and_empty_is_unset() {
        assert_eq!(home_of(os(r"C:\Users\x"), os("/home/x")), Some(PathBuf::from(r"C:\Users\x")));
        assert_eq!(home_of(None, os("/home/x")), Some(PathBuf::from("/home/x")));
        assert_eq!(home_of(os(""), os("/home/x")), Some(PathBuf::from("/home/x")));
        assert_eq!(home_of(os(""), os("")), None);
        assert_eq!(home_of(None, None), None);
    }

    #[test]
    fn claude_dir_without_the_variable_is_byte_identical_to_home_dot_claude() {
        let home = PathBuf::from(r"C:\Users\x");
        let dir = claude_dir_of(None, Some(home.clone())).unwrap();
        assert_eq!(dir.as_os_str().as_encoded_bytes(), home.join(".claude").as_os_str().as_encoded_bytes());
        assert_eq!(claude_dir_of(os(""), Some(home.clone())), Some(home.join(".claude")), "empty is unset");
        assert_eq!(
            dir.join("sessions").as_os_str().as_encoded_bytes(),
            PathBuf::from(r"C:\Users\x").join(".claude").join("sessions").as_os_str().as_encoded_bytes()
        );
        assert_eq!(claude_dir_of(None, None), None);
    }

    #[test]
    fn claude_dir_with_the_variable_set_is_that_dir() {
        let dir = claude_dir_of(os(r"D:\claude-config"), Some(PathBuf::from(r"C:\Users\x"))).unwrap();
        assert_eq!(dir, PathBuf::from(r"D:\claude-config"));
        assert_eq!(dir.join("sessions"), PathBuf::from(r"D:\claude-config").join("sessions"));
        assert_eq!(claude_dir_of(os(r"D:\claude-config"), None), Some(PathBuf::from(r"D:\claude-config")), "no home needed");
    }

    #[test]
    fn normalize_folds_separators_dots_and_the_verbatim_prefix() {
        for (path, want) in [
            (r"C:\a\b\", "C:/a/b"),
            (r"C:\a\.\b\..\c", "C:/a/c"),
            (r"\\?\C:\a\b", "C:/a/b"),
            ("C:/a//b/", "C:/a/b"),
            (r"C:\..\a", "C:/../a"),
            ("/x/y/../z", "/x/z"),
            (r"C:\Mixed\Case", "C:/Mixed/Case"),
        ] {
            assert_eq!(normalize(Path::new(path)), want, "{path}");
        }
        assert_eq!(path_key(Path::new(r"C:\A\b\")) == path_key(Path::new("c:/a/B")), cfg!(windows));
    }

    #[test]
    fn main_checkout_of_a_repo_and_of_a_linked_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("Alpha");
        fs::create_dir_all(main.join(".git").join("worktrees").join("feat")).unwrap();
        fs::create_dir_all(main.join("src")).unwrap();
        let wt = tmp.path().join("Alpha.worktrees").join("feat");
        fs::create_dir_all(wt.join("deep")).unwrap();
        fs::write(wt.join(".git"), "gitdir: ../../Alpha/.git/worktrees/feat\n").unwrap();
        let abs = tmp.path().join("abs");
        fs::create_dir_all(&abs).unwrap();
        fs::write(abs.join(".git"), format!("gitdir: {}\n", main.join(".git").join("worktrees").join("abs").display())).unwrap();
        let want = PathBuf::from(normalize(&main));
        assert_eq!(main_checkout(&main.join("src")), Some(want.clone()));
        assert_eq!(main_checkout(&wt.join("deep")), Some(want.clone()), "relative gitdir");
        assert_eq!(main_checkout(&abs), Some(want), "absolute gitdir");
        let odd = tmp.path().join("odd");
        fs::create_dir_all(&odd).unwrap();
        fs::write(odd.join(".git"), "gitdir: /elsewhere/modules/x\n").unwrap();
        assert_eq!(main_checkout(&odd), None, "a gitdir that is not a worktree's");
        assert_eq!(linked_git_dir(&wt.join(".git")), Some(wt.join("../../Alpha/.git/worktrees/feat")));
    }

    #[test]
    fn git_at_is_the_nearest_git_dir_or_worktree_file() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("Alpha");
        fs::create_dir_all(main.join(".git")).unwrap();
        fs::create_dir_all(main.join("src").join("deep")).unwrap();
        let wt = main.join("src").join("wt");
        fs::create_dir_all(wt.join("deep")).unwrap();
        fs::write(wt.join(".git"), "gitdir: ../../.git/worktrees/wt\n").unwrap();
        let plain = tmp.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        assert_eq!(git_at(&main), Some(main.join(".git")));
        assert_eq!(git_at(&main.join("src").join("deep")), Some(main.join(".git")));
        assert_eq!(git_at(&wt.join("deep")), Some(wt.join(".git")), "the worktree's file, not the repo above it");
        assert!(git_at(&wt).unwrap().is_file());
        assert_eq!(git_at(&plain).filter(|g| g.starts_with(tmp.path())), None);
    }
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

fn path_folders(path: &std::ffi::OsStr) -> impl Iterator<Item = PathBuf> {
    std::env::split_paths(path).filter(|dir| !dir.as_os_str().is_empty())
}

/// The folders of `path` (a `PATH` value) that hold the file `file`, in `PATH` order.
pub fn folders_holding(file: &str, path: &std::ffi::OsStr) -> Vec<PathBuf> {
    path_folders(path).filter(|dir| dir.join(file).is_file()).collect()
}

/// The file pwsh runs for a bare `name`: the first `PATH` folder holding `name.ps1` or `name` with a `PATHEXT` extension, `.ps1` first.
pub fn pwsh_command(name: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let exts: Vec<&str> = std::iter::once(".ps1").chain(pathext.split(';').map(str::trim).filter(|e| !e.is_empty())).collect();
    path_folders(path).find_map(|dir| {
        exts.iter().map(|ext| dir.join(format!("{name}{}", ext.to_ascii_lowercase()))).find(|file| file.is_file())
    })
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
    let exts: Vec<String> = pathext.split(';').map(str::trim).filter(|e| !e.is_empty()).map(str::to_ascii_lowercase).collect();
    let pwsh = path_folders(path)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| exts.iter().map(|ext| dir.join(format!("pwsh{ext}"))).find(|file| file.is_file()));
    match pwsh {
        Some(path) => PowerShell { alias: is_app_alias(&path), path, name: "pwsh" },
        None => windows_powershell(system_root),
    }
}

/// [`powershell`], with Windows PowerShell in place of a pwsh alias: for a child whose tree a timeout kills.
pub fn job_powershell(path: &std::ffi::OsStr, pathext: &str, system_root: Option<&std::ffi::OsStr>) -> PowerShell {
    let found = powershell(path, pathext, system_root);
    if found.alias { windows_powershell(system_root) } else { found }
}

#[cfg(windows)]
fn is_app_alias(file: &Path) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    fs::symlink_metadata(file).is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !m.file_type().is_symlink())
}

#[cfg(not(windows))]
fn is_app_alias(_: &Path) -> bool {
    false
}

fn process_env() -> (std::ffi::OsString, String, Option<std::ffi::OsString>) {
    let path = std::env::var_os("PATH").unwrap_or_default();
    (path, pathext(), std::env::var_os("SystemRoot"))
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

#[cfg(test)]
mod command_tests {
    use super::*;

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

    #[test]
    fn folders_holding_a_file_come_in_path_order() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b, c) = (tmp.path().join("a"), tmp.path().join("b"), tmp.path().join("c"));
        for (d, git) in [(&a, true), (&b, false), (&c, true)] {
            fs::create_dir_all(d).unwrap();
            if git {
                fs::write(d.join("git.exe"), "").unwrap();
            }
        }
        let path = std::env::join_paths([&c, &b, &a]).unwrap();
        assert_eq!(folders_holding("git.exe", &path), [c, a]);
        assert!(folders_holding("git.exe", std::ffi::OsStr::new("")).is_empty());
    }

    #[test]
    fn pwsh_runs_the_first_folders_ps1_then_its_pathext_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = |n: &str, files: &[&str]| {
            let d = tmp.path().join(n);
            fs::create_dir_all(&d).unwrap();
            files.iter().for_each(|f| fs::write(d.join(f), "").unwrap());
            d
        };
        let (npm, native, bare) = (dir("npm", &["claude", "claude.cmd", "claude.ps1"]), dir("native", &["claude.exe"]), dir("bare", &["claude.cmd", "claude"]));
        let ext = ".COM;.EXE;.BAT;.CMD";
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
        assert_eq!(pwsh_command("claude", &path(&[&npm, &native]), ext), Some(npm.join("claude.ps1")));
        assert_eq!(pwsh_command("claude", &path(&[&native, &npm]), ext), Some(native.join("claude.exe")));
        assert_eq!(pwsh_command("claude", &path(&[&bare, &native]), ext), Some(bare.join("claude.cmd")), "a folder earlier on PATH wins");
        assert_eq!(pwsh_command("claude", &path(&[&bare]), ".EXE"), None, "only PATHEXT extensions and .ps1");
    }
}

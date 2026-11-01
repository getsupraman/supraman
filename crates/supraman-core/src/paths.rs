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

/// The main checkout `dir` is in, [`normalize`]d: the folder of the first `.git` dir up from `dir`, or the
/// `<repo>` of a linked worktree's `gitdir: <repo>/.git/worktrees/<n>`. Never runs git.
pub fn main_checkout(dir: &Path) -> Option<PathBuf> {
    for ancestor in dir.ancestors() {
        let git = ancestor.join(".git");
        if git.is_dir() {
            return Some(PathBuf::from(normalize(ancestor)));
        }
        if !git.is_file() {
            continue;
        }
        let mut gitdir = normalize(&linked_git_dir(&git)?);
        gitdir.truncate(gitdir.rfind("/.git/worktrees/")?);
        return Some(PathBuf::from(gitdir));
    }
    None
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

/// The file pwsh runs for a bare `name`: the first `PATH` folder holding `name.ps1` or `name` with a `PATHEXT` extension, `.ps1` first.
pub fn pwsh_command(name: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let exts: Vec<&str> = std::iter::once(".ps1").chain(pathext.split(';').map(str::trim).filter(|e| !e.is_empty())).collect();
    std::env::split_paths(path).filter(|dir| !dir.as_os_str().is_empty()).find_map(|dir| {
        exts.iter().map(|ext| dir.join(format!("{name}{}", ext.to_ascii_lowercase()))).find(|file| file.is_file())
    })
}

#[cfg(test)]
mod command_tests {
    use super::*;

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

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// `USERPROFILE`, else `HOME`; an empty value counts as unset.
pub fn user_home() -> Option<PathBuf> {
    home_of(crate::platform::paths::user_profile(), std::env::var_os("HOME"))
}

fn home_of(userprofile: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let set = |v: Option<OsString>| v.filter(|v| !v.is_empty());
    set(userprofile).or_else(|| set(home)).map(PathBuf::from)
}

/// A user file a selftest replaces with its own, set once before the first read.
struct SelftestPath {
    what: &'static str,
    path: OnceLock<PathBuf>,
    read: AtomicBool,
}

impl SelftestPath {
    const fn new(what: &'static str) -> Self {
        Self { what, path: OnceLock::new(), read: AtomicBool::new(false) }
    }

    fn get(&self, user: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
        self.read.store(true, Ordering::SeqCst);
        self.path.get().cloned().or_else(user)
    }

    fn set(&self, path: PathBuf) -> Result<(), String> {
        let what = self.what;
        if self.read.load(Ordering::SeqCst) {
            return Err(format!("the {what} was read before the selftest set its own"));
        }
        let shown = path.display().to_string();
        self.path.set(path).map_err(|_| format!("the selftest's {what} is set already, not to {shown}"))
    }
}

static WORKMAN_SERVER_JSON: SelftestPath = SelftestPath::new("Workman server.json");
static CLAUDE_JSON: SelftestPath = SelftestPath::new("claude .claude.json");

/// `~/.workman/server.json` ([`crate::workman_server`]), which `/workman:setup` writes; `None` without a user home.
pub fn workman_server_json() -> Option<PathBuf> {
    WORKMAN_SERVER_JSON.get(|| user_home().map(|h| h.join(".workman").join("server.json")))
}

/// A selftest's own `server.json` in place of the user's: once per process, before [`workman_server_json`] is asked.
pub fn set_workman_server_json_for_selftest(path: PathBuf) -> Result<(), String> {
    WORKMAN_SERVER_JSON.set(path)
}

/// claude's user config: `<CLAUDE_CONFIG_DIR, else user_home>/.claude.json`, or the legacy `.config.json` while only it has content.
pub fn claude_json() -> Option<PathBuf> {
    CLAUDE_JSON.get(|| claude_json_of(claude_config_dir_var(), user_home(), |p| fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0)))
}

/// A selftest's own `.claude.json` in place of the user's: once per process, before [`claude_json`] is asked.
pub fn set_claude_json_for_selftest(path: PathBuf) -> Result<(), String> {
    CLAUDE_JSON.set(path)
}

fn claude_json_of(config_dir: Option<OsString>, home: Option<PathBuf>, has_content: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let current = config_dir.clone().filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| home.clone())?.join(".claude.json");
    let legacy = claude_dir_of(config_dir, home).map(|d| d.join(".config.json"));
    match legacy {
        Some(legacy) if has_content(&legacy) && !has_content(&current) => Some(legacy),
        _ => Some(current),
    }
}

/// What claude's user config says of one user-scope MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserMcpServer {
    NoFile,
    /// No top-level `mcpServers` entry of that name (a project's own `mcpServers` do not count).
    Missing,
    /// The entry's `url`, `None` when it has none (a stdio server).
    Url(Option<String>),
}

/// Only each entry's `url` is kept; `headers` and every other key are skipped unread into any value.
#[derive(serde::Deserialize)]
struct ClaudeJson {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: std::collections::HashMap<String, McpEntry>,
}

#[derive(serde::Deserialize)]
struct McpEntry {
    #[serde(default)]
    url: Option<serde_json::Value>,
}

/// The user-scope MCP server `name` in claude's config at `path` ([`claude_json`]); blocking.
pub fn user_mcp_server(path: &Path, name: &str) -> Result<UserMcpServer, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(UserMcpServer::NoFile),
        Err(e) => return Err(format!("{} cannot be read ({e})", path.display())),
    };
    let config: ClaudeJson =
        serde_json::from_str(&crate::text::decode(&bytes).0).map_err(|e| format!("{} is not valid JSON ({e})", path.display()))?;
    Ok(match config.mcp_servers.get(name) {
        Some(entry) => UserMcpServer::Url(entry.url.as_ref().and_then(|u| u.as_str()).map(str::to_string)),
        None => UserMcpServer::Missing,
    })
}

fn claude_config_dir_var() -> Option<OsString> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
}

/// claude's config dir: `CLAUDE_CONFIG_DIR`, else `<user_home>/.claude`.
pub fn claude_dir() -> Option<PathBuf> {
    claude_dir_of(claude_config_dir_var(), user_home())
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
    crate::platform::paths::compared_case(normalize(path))
}

/// `dir` is `root` or below it, compared by [`path_key`]; a sibling sharing a prefix is not.
pub fn within(dir: &Path, root: &Path) -> bool {
    key_within(&path_key(dir), &path_key(root))
}

/// The folder's own name as the user reads it: its last part, the whole path when it has none.
pub fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
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

/// The main checkout `dir` is in, [`normalize`]d, read from the `.git` walk. Never runs git.
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
    fn claude_json_is_in_the_config_dir_else_home_and_the_legacy_file_only_while_it_alone_has_content() {
        let home = PathBuf::from(r"C:\Users\x");
        let none = |_: &Path| false;
        assert_eq!(claude_json_of(None, Some(home.clone()), none), Some(home.join(".claude.json")));
        assert_eq!(claude_json_of(os(""), Some(home.clone()), none), Some(home.join(".claude.json")), "empty is unset");
        assert_eq!(claude_json_of(os(r"D:\cfg"), Some(home.clone()), none), Some(PathBuf::from(r"D:\cfg").join(".claude.json")));
        assert_eq!(claude_json_of(None, None, none), None);
        let legacy = home.join(".claude").join(".config.json");
        assert_eq!(claude_json_of(None, Some(home.clone()), |p| p == legacy), Some(legacy.clone()));
        assert_eq!(claude_json_of(None, Some(home.clone()), |_| true), Some(home.join(".claude.json")), "both: the current one");
        let cfg_legacy = PathBuf::from(r"D:\cfg").join(".config.json");
        assert_eq!(claude_json_of(os(r"D:\cfg"), Some(home), |p| p == cfg_legacy), Some(cfg_legacy.clone()));
    }

    const SECRET: &str = "SENTINEL-HEADER-VALUE";

    #[test]
    fn user_mcp_server_reads_a_top_level_entrys_url_only() {
        let tmp = tempfile::tempdir().unwrap();
        let read = |name: &str, text: &str| {
            let path = tmp.path().join(name);
            fs::write(&path, text).unwrap();
            user_mcp_server(&path, "workman")
        };
        assert_eq!(user_mcp_server(&tmp.path().join("absent.json"), "workman"), Ok(UserMcpServer::NoFile));
        assert_eq!(read("empty.json", "{}"), Ok(UserMcpServer::Missing));
        assert_eq!(read("other.json", r#"{"mcpServers":{"github":{"type":"http","url":"https://api.example.com/mcp"}}}"#), Ok(UserMcpServer::Missing));
        let project_only = r#"{"projects":{"P:/x":{"mcpServers":{"workman":{"type":"http","url":"https://work.example/mcp"}}}}}"#;
        assert_eq!(read("project.json", project_only), Ok(UserMcpServer::Missing), "a project's entry is not user scope");
        let user = format!(
            r#"{{"oauthAccount":{{"emailAddress":"{SECRET}"}},"mcpServers":{{"workman":{{"type":"http","url":"https://work.example/mcp","headers":{{"Authorization":"Bearer {SECRET}"}},"headersHelper":"{SECRET}"}}}}}}"#
        );
        assert_eq!(read("user.json", &user), Ok(UserMcpServer::Url(Some("https://work.example/mcp".into()))));
        assert_eq!(read("bom.json", "\u{feff}{\"mcpServers\":{\"workman\":{\"command\":\"x\"}}}"), Ok(UserMcpServer::Url(None)));
        let bad = read("bad.json", &format!(r#"{{"mcpServers": {{"workman": {{"headers": "{SECRET}""#)).unwrap_err();
        assert!(bad.contains("not valid JSON") && !bad.contains(SECRET), "{bad}");
    }

    #[test]
    fn the_readers_entry_type_holds_no_headers_so_their_values_never_reach_memory() {
        let entry: McpEntry = serde_json::from_str(&format!(r#"{{"url":"https://w.example/mcp","headers":{{"Authorization":"Bearer {SECRET}"}}}}"#)).unwrap();
        let McpEntry { url } = entry;
        assert_eq!(url, Some(serde_json::Value::String("https://w.example/mcp".into())));
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

fn path_folders(path: &std::ffi::OsStr) -> impl Iterator<Item = PathBuf> {
    std::env::split_paths(path).filter(|dir| !dir.as_os_str().is_empty())
}

/// The folders of `path` (a `PATH` value) that hold the file `file`, in `PATH` order.
pub fn folders_holding(file: &str, path: &std::ffi::OsStr) -> Vec<PathBuf> {
    path_folders(path).filter(|dir| dir.join(file).is_file()).collect()
}

/// The file pwsh runs for a bare `name`: the first `PATH` folder holding `name.ps1` or `name` with a `PATHEXT` extension, `.ps1` first.
pub fn pwsh_command(name: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    first_on_path(name, path, std::iter::once(".ps1").chain(pathext_list(pathext)), false)
}

/// The program a bare `name` starts: the first absolute `PATH` folder holding `name` with a `PATHEXT` extension.
pub fn executable(name: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    first_on_path(name, path, pathext_list(pathext), true)
}

/// [`executable`] on this process's `PATH` and `PATHEXT`.
pub fn executable_on_path(name: &str) -> Option<PathBuf> {
    let (path, pathext, _) = crate::platform::shell::process_env();
    executable(name, &path, &pathext)
}

/// The extensions of a `PATHEXT` value, each with its leading dot, as written.
pub fn pathext_list(pathext: &str) -> impl Iterator<Item = &str> {
    pathext.split(';').map(str::trim).filter(|e| !e.is_empty())
}

fn first_on_path<'a>(name: &str, path: &std::ffi::OsStr, exts: impl Iterator<Item = &'a str>, absolute_only: bool) -> Option<PathBuf> {
    let exts: Vec<String> = exts.map(str::to_ascii_lowercase).collect();
    path_folders(path)
        .filter(|dir| !absolute_only || dir.is_absolute())
        .find_map(|dir| exts.iter().map(|ext| dir.join(format!("{name}{ext}"))).find(|file| file.is_file()))
}

/// This process's `PATH`, empty when unset.
pub fn process_path() -> OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn a_relative_path_folder_is_searched_by_pwsh_but_never_for_a_program() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("tool.exe"), "").unwrap();
        let driveless: PathBuf = bin.components().skip(1).collect();
        assert!(!driveless.is_absolute() && driveless.join("tool.exe").is_file(), "{} resolves on the current drive", driveless.display());
        let path = std::env::join_paths([&driveless]).unwrap();
        assert_eq!(pwsh_command("tool", &path, ".EXE"), Some(driveless.join("tool.exe")), "pwsh runs from a relative folder");
        assert_eq!(executable("tool", &path, ".EXE"), None);
        let both = std::env::join_paths([&driveless, &bin]).unwrap();
        assert_eq!(executable("tool", &both, ".EXE"), Some(bin.join("tool.exe")), "the absolute folder after it");
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
        assert_eq!(executable("claude", &path(&[&npm, &native]), ext), Some(npm.join("claude.cmd")), "never a .ps1");
    }
}

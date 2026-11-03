//! The write side: `claude --bg`, `stop`, `rm`, and `--bg --resume`. Facts behind each shape are in
//! `docs/LEARNINGS.md` (m1/5 trust refusal, m2/1 launcher probes).

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use supraman_core::process::{self, Kill, Output, RunError};

use crate::{ClaudeError, Result, Session, parse};

/// `--bg` returns in ~0.8 s (m0/2); `stop`/`rm` likewise. Generous, the daemon may be cold.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// What `claude --bg` should start. Fields map 1:1 onto CLI flags, see [`LaunchSpec::argv`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchSpec {
    pub name: String,
    pub cwd: PathBuf,
    pub prompt: String,
    pub permission_mode: String,
    pub mcp_config: Option<PathBuf>,
    pub strict_mcp_config: bool,
    pub plugin_dirs: Vec<PathBuf>,
    /// `--add-dir`: dirs outside `cwd` the session's tools may use.
    pub add_dirs: Vec<PathBuf>,
    /// Set on the `claude --bg` process only. The session does NOT inherit it: the daemon spawns
    /// the session (m2/1). Use `--settings {"env":…}` in `extra_args` for env the session must see.
    pub env: Vec<(String, String)>,
    pub extra_args: Vec<String>,
    /// `--resume <sessionId>`: beside the other flags it starts a copy under a new id that carries that
    /// session's transcript; the original keeps its id and name (m5/61). claude cannot rename a session.
    pub resume: Option<String>,
}

impl LaunchSpec {
    pub fn new(name: impl Into<String>, cwd: impl Into<PathBuf>, prompt: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cwd: cwd.into(),
            prompt: prompt.into(),
            permission_mode: "auto".into(),
            ..Self::default()
        }
    }

    /// A per-session `wss_` token in an owner-only `<dir>/workstate-<agent>.json`, under `--strict-mcp-config`.
    pub fn with_minted_token(mut self, dir: &Path, agent: &str, url: &str, token: &str) -> std::io::Result<Self> {
        self.mcp_config = Some(write_mcp_config(dir, agent, url, token, &[])?);
        self.strict_mcp_config = true;
        Ok(self)
    }

    /// Everything after the program name. The prompt goes after `--` because `--mcp-config` is
    /// variadic and would otherwise swallow it.
    pub fn argv(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["--bg".into(), "--name".into(), self.name.clone().into()];
        if let Some(session_id) = &self.resume {
            args.push("--resume".into());
            args.push(session_id.clone().into());
        }
        if !self.permission_mode.is_empty() {
            args.push("--permission-mode".into());
            args.push(self.permission_mode.clone().into());
        }
        if let Some(config) = &self.mcp_config {
            args.push("--mcp-config".into());
            args.push(config.clone().into());
        }
        if self.strict_mcp_config {
            args.push("--strict-mcp-config".into());
        }
        for dir in &self.plugin_dirs {
            args.push("--plugin-dir".into());
            args.push(dir.clone().into());
        }
        for dir in &self.add_dirs {
            args.push("--add-dir".into());
            args.push(dir.clone().into());
        }
        args.extend(self.extra_args.iter().map(OsString::from));
        args.push("--".into());
        args.push(self.prompt.clone().into());
        args
    }
}

/// A fresh v4 uuid for `--session-id`. A foreground `claude` takes it; `claude --bg` accepts the flag
/// and ignores it (m5/40 probe), so a background session's id is known only from its row.
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// `--bg --resume <sessionId> [-- <prompt>]` and nothing else: with any other flag the CLI starts a
/// copy under a new id; without, it wakes the same session "with its saved options" (m2/1).
pub fn resume_argv(session_id: &str, prompt: Option<&str>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--bg".into(), "--resume".into(), session_id.into()];
    if let Some(prompt) = prompt {
        args.push("--".into());
        args.push(prompt.into());
    }
    args
}

/// Who a session is, as its mint request tells Workman. A label, never an authority: the role still comes from the token.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionIdentity {
    /// The session's `--name`; also the config file's name.
    pub agent: String,
    /// The claude session uuid, when it is known at launch.
    pub session: Option<String>,
    /// E.g. `foreman/nomos`.
    pub launched_by: String,
    /// E.g. `foreman/nomos/Nomos.Extract`.
    pub origin: String,
    pub cwd: String,
    /// Foreman's role (workspace, repo, …), not the token's.
    pub foreman_role: String,
}

/// The `mcpServers` key of a session's Workman server: its tools are `mcp__workman__*`, and a [+] session's entry
/// overrides the user-scope server of the same name (`/workman:setup` registers it).
pub const MCP_SERVER: &str = "workman";

/// The [`MCP_SERVER`] entry with the token and `headers` written in, shaped like the workman plugin's `plugin.mcp.json`.
pub fn workstate_mcp_config(url: &str, token: &str, headers: &[(&str, String)]) -> String {
    let mut map = serde_json::Map::new();
    map.insert("Authorization".into(), format!("Bearer {token}").into());
    for (k, v) in headers {
        map.insert((*k).into(), v.clone().into());
    }
    serde_json::json!({
        "mcpServers": {
            (MCP_SERVER): {
                "type": "http",
                "url": url,
                "headers": map
            }
        }
    })
    .to_string()
}

/// Writes `<dir>/workstate-<agent>.json` with `headers` beside the `Authorization` one. The file is
/// created empty, restricted to the current user (Windows: inheritance removed, the user alone granted
/// full control; unix: 0600), and only then filled, so the token is never readable by anyone else.
pub fn write_mcp_config(dir: &Path, agent: &str, url: &str, token: &str, headers: &[(&str, String)]) -> std::io::Result<PathBuf> {
    let path = mcp_config_path(dir, agent)?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, b"")?;
    restrict_to_owner(&path)?;
    std::fs::write(&path, workstate_mcp_config(url, token, headers))?;
    Ok(path)
}

/// `<dir>/workstate-<agent>.json`, the one file of `agent`'s Workman config; `agent` must be a plain session name.
pub fn mcp_config_path(dir: &Path, agent: &str) -> std::io::Result<PathBuf> {
    let plain = !agent.is_empty()
        && !agent.starts_with('.')
        && agent.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !plain {
        return Err(std::io::Error::new(ErrorKind::InvalidInput, format!("not a session name: {agent:?}")));
    }
    Ok(dir.join(format!("workstate-{agent}.json")))
}

/// The Workman URL in the config [`write_mcp_config`] wrote for `agent`; `None` when it is gone or unreadable.
pub fn mcp_config_url(dir: &Path, agent: &str) -> Option<String> {
    let text = std::fs::read_to_string(mcp_config_path(dir, agent).ok()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["mcpServers"][MCP_SERVER]["url"].as_str().map(str::to_string)
}

#[cfg(windows)]
const ICACLS_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(windows)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    let user = std::env::var("USERNAME").map_err(|_| std::io::Error::other("USERNAME is not set"))?;
    let out = process::run(
        // an elevated process's file also carries explicit SYSTEM and Administrators entries
        process::command("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &format!("{user}:F"), "/remove:g", "*S-1-5-18", "*S-1-5-32-544"]),
        ICACLS_TIMEOUT,
        Kill::Tree,
    )
    .map_err(|e| std::io::Error::other(format!("icacls {}: {e}", path.display())))?;
    if out.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "icacls {}: {}",
            path.display(),
            out.stdout.trim()
        )))
    }
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// The `claude` program. Injectable so tests run a fake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claude {
    pub program: PathBuf,
}

impl Default for Claude {
    fn default() -> Self {
        Self {
            program: PathBuf::from("claude"),
        }
    }
}

impl Claude {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into() }
    }

    /// Refuses with [`ClaudeError::NameTaken`] when any row (stopped ones included) already carries
    /// the name — the CLI itself accepts duplicates (m2/1), but names are Foreman's join key — then
    /// [`Claude::launch_unchecked`].
    pub fn launch(&self, spec: &LaunchSpec) -> Result<String> {
        if name_taken(&self.list_sessions()?, &spec.name) {
            return Err(ClaudeError::NameTaken { name: spec.name.clone() });
        }
        self.launch_unchecked(spec)
    }

    /// Starts the session and returns `--bg`'s stdout (`backgrounded · <id8> · <name>`). Resolve the
    /// session from its `agents --json --all` row by name ([`Claude::find_by_name`]), never from this text.
    pub fn launch_unchecked(&self, spec: &LaunchSpec) -> Result<String> {
        let out = self.exec(spec.argv(), Some(&spec.cwd), &spec.env, WRITE_TIMEOUT)?;
        match classify(&out) {
            Some(e) => Err(e),
            None => Ok(out.stdout.trim().to_string()),
        }
    }

    /// Wakes a stopped session under the same `id`/`sessionId`, transcript and launch options kept
    /// (`--mcp-config` included). Run from the session's `cwd`.
    pub fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> Result<String> {
        let out = self.exec(resume_argv(session_id, prompt), Some(cwd), &[], WRITE_TIMEOUT)?;
        match classify(&out) {
            Some(e) => Err(e),
            None => Ok(out.stdout.trim().to_string()),
        }
    }

    pub fn stop(&self, id: &str) -> Result<String> {
        self.simple(&["stop", id])
    }

    pub fn rm(&self, id: &str) -> Result<String> {
        self.simple(&["rm", id])
    }

    /// `claude rm <id>`, with `--discard-unpushed <value>` when `discard` is given (the value rm's own
    /// refusal printed). A refusal to lose a worktree's commits is [`Removal::Kept`], not an error.
    pub fn remove(&self, id: &str, discard: Option<&str>) -> Result<Removal> {
        let mut args = vec!["rm", id];
        if let Some(value) = discard {
            args.extend(["--discard-unpushed", value]);
        }
        match self.simple(&args) {
            Ok(out) => Ok(Removal::Removed(out.trim().to_string())),
            Err(ClaudeError::CliFailed { code, stderr }) => match RmRefusal::parse(&stderr) {
                Some(refusal) => Ok(Removal::Kept(refusal)),
                None => Err(ClaudeError::CliFailed { code, stderr }),
            },
            Err(e) => Err(e),
        }
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        self.list_sessions_within(WRITE_TIMEOUT)
    }

    pub(crate) fn list_sessions_within(&self, timeout: Duration) -> Result<Vec<Session>> {
        parse(&self.run_within(&["agents", "--json", "--all"], timeout)?)
    }

    /// Every row named `name`, newest first. Polls until one exists or `within` passes, because the
    /// row trails `--bg` by about a second (m0/2).
    pub fn find_by_name(&self, name: &str, within: Duration) -> Result<Vec<Session>> {
        let deadline = Instant::now() + within;
        loop {
            let mut rows: Vec<Session> = self.list_sessions()?.into_iter().filter(|s| s.name == name).collect();
            if !rows.is_empty() || Instant::now() >= deadline {
                rows.sort_by(|a, b| b.started_at.cmp(&a.started_at));
                return Ok(rows);
            }
            thread::sleep(Duration::from_millis(500));
        }
    }

    fn simple(&self, args: &[&str]) -> Result<String> {
        self.run_within(args, WRITE_TIMEOUT)
    }

    pub(crate) fn run_within(&self, args: &[&str], timeout: Duration) -> Result<String> {
        self.run_in(args, None, timeout)
    }

    /// `claude <args>` from `cwd`, no flag of Foreman's added: stdout, or the [`classify`]d failure.
    pub fn run_in(&self, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<String> {
        let out = self.exec(args.iter().map(OsString::from).collect(), cwd, &[], timeout)?;
        match classify(&out) {
            Some(e) => Err(e),
            None => Ok(out.stdout),
        }
    }

    fn exec(&self, args: Vec<OsString>, cwd: Option<&Path>, env: &[(String, String)], timeout: Duration) -> Result<Output> {
        let command = format!(
            "{} {}",
            self.program.display(),
            args.first().map(|a| a.to_string_lossy()).unwrap_or_default()
        );
        let mut cmd = process::command(&self.program);
        cmd.args(&args);
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        process::run(&mut cmd, timeout, Kill::Child).map_err(|e| match e {
            RunError::Spawn(source) if source.kind() == ErrorKind::NotFound => ClaudeError::NotInstalled,
            RunError::Spawn(source) => ClaudeError::Spawn { command, source },
            RunError::Timeout(after) => ClaudeError::Timeout { command, after },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    /// rm's stdout, e.g. `removed <id>`.
    Removed(String),
    Kept(RmRefusal),
}

/// rm's "kept <id> — its worktree is still at “<path>” / N unpushed commits on “<branch>” … claude rm
/// <id> --discard-unpushed <sha>@<token>" (m2/2, CLI 2.1.309).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmRefusal {
    /// The whole text, trimmed.
    pub text: String,
    pub unpushed: Option<u32>,
    /// What `--discard-unpushed` takes, as printed.
    pub discard: Option<String>,
    pub worktree: Option<PathBuf>,
}

impl RmRefusal {
    /// `None` unless the text is rm keeping a session.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if !text.split_whitespace().any(|w| w == "kept") {
            return None;
        }
        let unpushed = text.split_once(" unpushed commit").and_then(|(before, _)| before.split_whitespace().last()?.parse().ok());
        let discard = text
            .split_once("--discard-unpushed ")
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .filter(|t| t.contains('@'))
            .map(str::to_string);
        let worktree = text.split_once("worktree is still at ").and_then(|(_, rest)| {
            let rest = rest.trim_start_matches(['“', '"']);
            rest.find(['”', '"']).map(|end| PathBuf::from(&rest[..end]))
        });
        Some(Self { text: text.to_string(), unpushed, discard, worktree })
    }
}

pub fn name_taken(rows: &[Session], name: &str) -> bool {
    rows.iter().any(|s| s.name == name)
}

/// The typed error for a failed `claude` write command, or `None` when it succeeded.
pub fn classify(out: &Output) -> Option<ClaudeError> {
    if out.code == Some(0) {
        return None;
    }
    if let Some(cwd) = trust_refusal(&format!("{}\n{}", out.stderr, out.stdout)) {
        return Some(ClaudeError::TrustNeeded { cwd });
    }
    let text = if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr };
    if format!("{}
{}", out.stderr, out.stdout).contains(crate::UNREACHABLE) {
        return Some(ClaudeError::DaemonUnreachable {
            code: out.code,
            stderr: text.trim().to_string(),
        });
    }
    Some(ClaudeError::CliFailed {
        code: out.code,
        stderr: text.trim().to_string(),
    })
}

/// m1/5: ``Workspace not trusted. Run `claude` in <dir> once and accept the trust prompt, then retry.``
fn trust_refusal(text: &str) -> Option<PathBuf> {
    let line = text.lines().find(|l| l.contains("Workspace not trusted"))?;
    let rest = line.split_once("` in ")?.1;
    let dir = rest.split_once(" once and accept the trust prompt")?.0;
    Some(PathBuf::from(dir.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// m1/5, verbatim.
    const TRUST: &str =
        "Workspace not trusted. Run `claude` in P:\\Projects\\x\\nested once and accept the trust prompt, then retry.\n";

    fn strs(args: &[OsString]) -> Vec<String> {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn full() -> LaunchSpec {
        LaunchSpec {
            name: "fm-demo-a".into(),
            cwd: PathBuf::from(r"C:\demo"),
            prompt: "--looks-like-a-flag but is the prompt".into(),
            permission_mode: "auto".into(),
            mcp_config: Some(PathBuf::from(r"C:\fm\ws.json")),
            strict_mcp_config: true,
            plugin_dirs: vec![PathBuf::from(r"C:\p1"), PathBuf::from(r"C:\p2")],
            add_dirs: vec![PathBuf::from(r"C:\fm\relay\demo")],
            env: vec![("A".into(), "1".into())],
            extra_args: vec!["--model".into(), "opus".into()],
            resume: None,
        }
    }

    #[test]
    fn argv_has_every_field_and_the_prompt_last_after_dashdash() {
        assert_eq!(
            strs(&full().argv()),
            [
                "--bg", "--name", "fm-demo-a", "--permission-mode", "auto", "--mcp-config", r"C:\fm\ws.json",
                "--strict-mcp-config", "--plugin-dir", r"C:\p1", "--plugin-dir", r"C:\p2", "--add-dir",
                r"C:\fm\relay\demo", "--model", "opus", "--",
                "--looks-like-a-flag but is the prompt",
            ]
        );
    }

    #[test]
    fn a_launch_that_resumes_names_the_session_right_after_its_own_name() {
        let spec = LaunchSpec { resume: Some("fc5bf027-48e3-4510-90b2-63a00eb76df8".into()), ..full() };
        let argv = strs(&spec.argv());
        assert_eq!(argv[..5], ["--bg", "--name", "fm-demo-a", "--resume", "fc5bf027-48e3-4510-90b2-63a00eb76df8"]);
        assert_eq!(argv[5..], strs(&full().argv())[3..]);
    }

    #[test]
    fn argv_minimal_omits_optional_flags() {
        let spec = LaunchSpec {
            permission_mode: String::new(),
            ..LaunchSpec::new("n", "C:\\", "hi")
        };
        assert_eq!(strs(&spec.argv()), ["--bg", "--name", "n", "--", "hi"]);
        assert_eq!(
            strs(&LaunchSpec::new("n", "C:\\", "hi").argv()),
            ["--bg", "--name", "n", "--permission-mode", "auto", "--", "hi"]
        );
    }

    #[test]
    fn resume_argv_carries_no_launch_flags() {
        assert_eq!(strs(&resume_argv("1234-uuid", None)), ["--bg", "--resume", "1234-uuid"]);
        assert_eq!(strs(&resume_argv("1234-uuid", Some("go on"))), ["--bg", "--resume", "1234-uuid", "--", "go on"]);
    }

    #[test]
    fn minted_token_lands_in_an_owner_only_file_per_session_not_in_argv() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let spec = LaunchSpec::new("fm-demo-alpha", "C:\\", "hi")
            .with_minted_token(&mcp, "fm-demo-alpha", "https://ws.example/mcp", "fake-token")
            .unwrap();
        let path = mcp.join("workstate-fm-demo-alpha.json");
        assert_eq!(spec.mcp_config.as_deref(), Some(path.as_path()));
        assert!(spec.strict_mcp_config);
        assert!(spec.env.is_empty());
        let argv = strs(&spec.argv()).join(" ");
        assert!(!argv.contains("fake-token"), "{argv}");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let ws = &v["mcpServers"]["workman"];
        assert!(v["mcpServers"].get("workstate").is_none(), "{v}");
        assert_eq!(ws["type"], "http");
        assert_eq!(ws["url"], "https://ws.example/mcp");
        assert_eq!(ws["headers"], serde_json::json!({ "Authorization": "Bearer fake-token" }));
        assert_eq!(mcp_config_url(&mcp, "fm-demo-alpha").as_deref(), Some("https://ws.example/mcp"));
        assert_eq!(mcp_config_url(&mcp, "fm-demo-beta"), None);
        #[cfg(windows)]
        {
            let acl = supraman_core::process::command("icacls").arg(&path).output().unwrap();
            let acl = String::from_utf8_lossy(&acl.stdout);
            let user = std::env::var("USERNAME").unwrap();
            let grants: Vec<&str> = acl.lines().filter(|l| l.contains(":(")).collect();
            assert_eq!(grants.len(), 1, "{acl}");
            assert!(grants[0].contains(&format!("{user}:(F)")), "{acl}");
        }
        let path_b = write_mcp_config(&mcp, "fm-demo-beta", "http://x/mcp", "fake-token", &[("X-Extra", "1".into())]).unwrap();
        assert_ne!(path_b, path, "one file per session");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path_b).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["workman"]["headers"]["X-Extra"], "1");
        for bad in ["", "..", "../x", r"a\b", "a b"] {
            assert!(write_mcp_config(&mcp, bad, "u", "fake-token", &[]).is_err(), "{bad:?}");
            assert!(mcp_config_path(&mcp, bad).is_err(), "{bad:?}");
        }
        assert_eq!(mcp_config_path(&mcp, "fm-demo-beta").unwrap(), path_b);
    }

    fn out(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn success_is_not_an_error() {
        assert!(classify(&out(0, "backgrounded · abcd1234 · n\n", "")).is_none());
    }

    #[test]
    fn trust_refusal_names_the_dir() {
        for o in [out(1, "", TRUST), out(1, TRUST, "")] {
            match classify(&o) {
                Some(ClaudeError::TrustNeeded { cwd }) => assert_eq!(cwd, PathBuf::from(r"P:\Projects\x\nested")),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn name_taken_reads_every_row_including_stopped() {
        let rows = parse(
            r#"[{"id":"a1b14b94","sessionId":"a1b14b94-b03a-4cb2-8360-65740402a70d","name":"aw-x","cwd":"C:\\d","kind":"background","startedAt":1,"state":"stopped"},
                {"sessionId":"bbbb0004-x","cwd":"C:\\other","kind":"interactive","startedAt":1,"status":"idle","pid":4}]"#,
        )
        .unwrap();
        assert!(name_taken(&rows, "aw-x"));
        assert!(!name_taken(&rows, "aw-y"));
    }

    /// René's Inbox row on 2ae06cd (m5/25), verbatim.
    #[test]
    fn unreachable_background_service_is_its_own_error() {
        let text = "Starting background service… Couldn't reach the background service (background service did not become reachable within 45s) — run 'claude daemon status'\n";
        for o in [out(1, "", text), out(1, text, "")] {
            match classify(&o) {
                Some(e @ ClaudeError::DaemonUnreachable { .. }) => {
                    assert!(e.to_string().starts_with("claude failed (exit 1): Starting background service…"), "{e}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// `claude rm` on a session whose worktree has commits (m2/2, CLI 2.1.309), verbatim.
    const KEPT: &str = "kept 202c9998 — its worktree is still at “P:\\Projects\\LarchSys\\AutoWork\\.claude\\worktrees\\iso-probe”\n  39 unpushed commits on “worktree-iso-probe”: 3ccd48a “probe: add iso.txt” and 38 more. They exist on no remote, so deleting the worktree would lose them.\n  push them and run 'claude rm 202c9998' again, or discard the worktree and its commits: claude rm 202c9998 --discard-unpushed 3ccd48a1bb9d660d38c0fabd2f5e63bd64be737a@d71f54d85da4468315a1efd780272116\n";

    #[test]
    fn rm_keeping_a_session_is_read_with_its_count_worktree_and_discard_value() {
        let r = RmRefusal::parse(KEPT).unwrap();
        assert_eq!(r.unpushed, Some(39));
        assert_eq!(r.discard.as_deref(), Some("3ccd48a1bb9d660d38c0fabd2f5e63bd64be737a@d71f54d85da4468315a1efd780272116"));
        assert_eq!(r.worktree, Some(PathBuf::from(r"P:\Projects\LarchSys\AutoWork\.claude\worktrees\iso-probe")));
        assert!(r.text.starts_with("kept 202c9998") && !r.text.ends_with('\n'));
        let one = RmRefusal::parse("kept abcd1234 — its worktree is still at “C:\\w”\n  1 unpushed commit on “b”: …").unwrap();
        assert_eq!((one.unpushed, one.discard), (Some(1), None));
        assert_eq!(RmRefusal::parse("removed 202c9998"), None);
        assert_eq!(RmRefusal::parse("error: no session 202c9998"), None);
    }

    #[test]
    fn anything_else_is_cli_failed_with_code_and_text() {
        match classify(&out(3, "", "  boom\n")) {
            Some(ClaudeError::CliFailed { code, stderr }) => assert_eq!((code, stderr.as_str()), (Some(3), "boom")),
            other => panic!("{other:?}"),
        }
        match classify(&out(1, "only stdout", "")) {
            Some(ClaudeError::CliFailed { stderr, .. }) => assert_eq!(stderr, "only stdout"),
            other => panic!("{other:?}"),
        }
    }
}

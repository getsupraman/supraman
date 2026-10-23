//! The write side: `claude --bg`, `stop`, `rm`, and `--bg --resume`. Facts behind each shape are in
//! `docs/LEARNINGS.md` (m1/5 trust refusal, m2/1 launcher probes).

use std::ffi::OsString;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

    /// The decided role-token route (m2/1): `<dir>/workstate-<role>.json` holds the role's token
    /// (owner-only ACL, see [`write_role_mcp_config`]) and the session gets it via
    /// `--mcp-config <file> --strict-mcp-config`, so the user-scope `workstate` is never loaded.
    /// `dir` is `<FOREMAN_HOME>/mcp`; the caller reads `token`.
    pub fn with_role_token(mut self, dir: &Path, role: &str, url: &str, token: &str) -> std::io::Result<Self> {
        self.mcp_config = Some(write_role_mcp_config(dir, role, url, token)?);
        self.strict_mcp_config = true;
        Ok(self)
    }

    /// Everything after the program name. The prompt goes after `--` because `--mcp-config` is
    /// variadic and would otherwise swallow it.
    pub fn argv(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["--bg".into(), "--name".into(), self.name.clone().into()];
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

/// The `workstate` server entry with the token written in, shaped like Skills' `plugin.mcp.json`.
pub fn workstate_mcp_config(url: &str, token: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "workstate": {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    })
    .to_string()
}

/// Writes `<dir>/workstate-<role>.json`. The file is created empty, restricted to the current user
/// (Windows: inheritance removed, the user alone granted full control; unix: 0600), and only then
/// filled, so the token is never readable by anyone else.
pub fn write_role_mcp_config(dir: &Path, role: &str, url: &str, token: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("workstate-{role}.json"));
    std::fs::write(&path, b"")?;
    restrict_to_owner(&path)?;
    std::fs::write(&path, workstate_mcp_config(url, token))?;
    Ok(path)
}

#[cfg(windows)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    let user = std::env::var("USERNAME").map_err(|_| std::io::Error::other("USERNAME is not set"))?;
    let out = Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r", &format!("{user}:F")])
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "icacls {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stdout).trim()
        )))
    }
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// What a finished `claude` process left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
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

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        parse(&self.simple(&["agents", "--json", "--all"])?)
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
        let out = self.exec(args.iter().map(OsString::from).collect(), None, &[], WRITE_TIMEOUT)?;
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
        let mut cmd = Command::new(&self.program);
        cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|source| match source.kind() {
            ErrorKind::NotFound => ClaudeError::NotInstalled,
            _ => ClaudeError::Spawn {
                command: command.clone(),
                source,
            },
        })?;
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ClaudeError::Timeout { command, after: timeout });
                }
                Err(source) => return Err(ClaudeError::Spawn { command, source }),
            }
        };
        Ok(Output {
            code: status.code(),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        })
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

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
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
    fn role_token_lands_in_an_owner_only_file_not_in_argv() {
        let dir = tempfile::tempdir().unwrap();
        let spec = LaunchSpec::new("n", "C:\\", "hi")
            .with_role_token(&dir.path().join("mcp"), "worker", "http://127.0.0.1:5290/mcp", "fake-token")
            .unwrap();
        let path = dir.path().join("mcp").join("workstate-worker.json");
        assert_eq!(spec.mcp_config.as_deref(), Some(path.as_path()));
        assert!(spec.strict_mcp_config);
        assert!(spec.env.is_empty());
        let argv = strs(&spec.argv()).join(" ");
        assert!(!argv.contains("fake-token"), "{argv}");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let ws = &v["mcpServers"]["workstate"];
        assert_eq!(ws["type"], "http");
        assert_eq!(ws["url"], "http://127.0.0.1:5290/mcp");
        assert_eq!(ws["headers"]["Authorization"], "Bearer fake-token");
        #[cfg(windows)]
        {
            let acl = Command::new("icacls").arg(&path).output().unwrap();
            let acl = String::from_utf8_lossy(&acl.stdout);
            let user = std::env::var("USERNAME").unwrap();
            let grants: Vec<&str> = acl.lines().filter(|l| l.contains(":(")).collect();
            assert_eq!(grants.len(), 1, "{acl}");
            assert!(grants[0].contains(&format!("{user}:(F)")), "{acl}");
        }
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

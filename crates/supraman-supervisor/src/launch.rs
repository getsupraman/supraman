use std::path::{Path, PathBuf};

use supraman_claude::{Claude, Health, LaunchSpec, Session, SessionState, Status};
use supraman_core::home::SessionEnv;
use supraman_core::{AGENTS_SERVER, FM_PREFIX};

/// Epoch milliseconds, as `claude agents` reports them.
pub type Millis = i64;

/// A session René opens with [+].
pub const HAND_ROLE: &str = "hand";

/// One `claude --bg` Supraman starts (a scheduled run), before its Workman token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgSession {
    pub name: String,
    pub role: String,
    pub cwd: PathBuf,
    pub repo: Option<String>,
    pub prompt: String,
    pub permission_mode: String,
    pub add_dirs: Vec<PathBuf>,
}

/// A row with a pid, or one whose health says it runs or waits on a prompt.
pub fn is_live(s: &Session, now: Millis) -> bool {
    s.pid.is_some() || matches!(s.health(now), Health::Running | Health::Blocked { .. })
}

/// `status=busy`: a session waiting for messages stays `state=working` with `status=idle` (LEARNINGS m3/2).
/// Only a live row with no `status` yet (right after `--bg`) falls back to `state=working`.
pub fn mid_turn(s: &Session, now: Millis) -> bool {
    is_live(s, now)
        && match &s.status {
            Some(status) => *status == Status::Busy,
            None => s.state == Some(SessionState::Working),
        }
}

/// Live `fm-` rows that are mid-turn: what `[sessions] max_working` caps.
pub fn working<'a>(observed: impl IntoIterator<Item = &'a Session>, now: Millis) -> usize {
    observed.into_iter().filter(|s| s.name.starts_with(FM_PREFIX) && mid_turn(s, now)).count()
}

/// An `fm-` session that names none of `workspaces` (`fm-<workspace>-…`): another `SUPRAMAN_HOME`'s, e.g. a selftest's.
pub fn is_foreign<'a>(name: &str, mut workspaces: impl Iterator<Item = &'a str>) -> bool {
    name.starts_with(FM_PREFIX) && !workspaces.any(|w| name.starts_with(&format!("{FM_PREFIX}{w}-")))
}

/// The `claude` writes of a scheduled run.
pub trait Launcher {
    fn launch(&self, spec: &LaunchSpec) -> supraman_claude::Result<String>;
    /// Must pass no flag but `--bg --resume <session_id> [-- <prompt>]` (any flag forks a copy).
    fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> supraman_claude::Result<String>;
    fn stop(&self, id: &str) -> supraman_claude::Result<String>;
}

impl Launcher for Claude {
    fn launch(&self, spec: &LaunchSpec) -> supraman_claude::Result<String> {
        Claude::launch(self, spec)
    }

    fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> supraman_claude::Result<String> {
        Claude::resume(self, session_id, cwd, prompt)
    }

    fn stop(&self, id: &str) -> supraman_claude::Result<String> {
        Claude::stop(self, id)
    }
}

/// The bearer of a session token's mint, PATCH and DELETE: the app's OAuth access token, never written anywhere.
#[derive(Clone, PartialEq, Eq)]
pub struct RoleToken {
    /// The Workman MCP URL; its origin is the API's base.
    pub url: String,
    pub token: String,
    /// The Workman role the session is minted with (controller, orchestrator, worker).
    pub role: String,
}

impl std::fmt::Debug for RoleToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoleToken").field("url", &self.url).field("token", &"<redacted>").field("role", &self.role).finish()
    }
}

/// Where the bearer comes from: the app's sign-in.
pub trait TokenProvider {
    /// `Ok(None)`: not signed in or Workman not set up (a token-less config); `Err`: no token now, which fails the launch.
    fn token(&self, role: &str) -> Result<Option<RoleToken>, String>;

    /// The MCP URL a session's config names while it has no token: [`crate::mcp_url`]; `Err` is Workman not set up.
    fn mcp_url(&self) -> Result<String, String>;
}

/// Launch options every session gets; every launch is `--strict-mcp-config`, never the user-scope Workman.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `<SUPRAMAN_HOME>/mcp`, where [`crate::Keeper`] writes each session's helped config.
    pub mcp_dir: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
    /// The session's `SUPRAMAN_HOME`, through `--settings {"env":…}`: env on `claude --bg` itself does not reach it.
    pub supraman_home: Option<PathBuf>,
    /// `bgIsolation: none`, else a `--bg` session in a main checkout writes into a worktree `claude rm` refuses to delete.
    pub bg_isolation_none: bool,
    /// `--allowedTools` every session gets, so `sm` never parks (`Bash(sm:*)` matches; PowerShell rules do not).
    pub allowed_tools: Vec<String>,
    /// Claude Code's config dir, for [`supraman_claude::plugin_server`]; `None` gives no plugin server.
    pub claude_dir: Option<PathBuf>,
}

impl LaunchConfig {
    /// The `claude --bg` of `w` before its Workman token; `env` adds to its env.
    pub fn launch_spec(&self, workspace: &str, w: &BgSession, env: &[(&str, &str)]) -> LaunchSpec {
        let mut spec = LaunchSpec::new(&w.name, &w.cwd, &w.prompt);
        spec.permission_mode = w.permission_mode.clone();
        spec.strict_mcp_config = true;
        spec.plugin_dirs = self.plugin_dirs.clone();
        spec.mcp_servers = self.plugin_servers(&w.name);
        spec.add_dirs = w.add_dirs.clone();
        if let Some(settings) = self.settings_with(workspace, &w.name, &w.role, w.repo.as_deref(), env) {
            spec.extra_args = vec!["--settings".into(), settings];
        }
        spec.extra_args.extend(self.allowed_tools_args());
        spec
    }

    /// The plugin servers `--strict-mcp-config` would leave out, for `name`'s own config; a miss is logged, not failed.
    pub fn plugin_servers(&self, name: &str) -> Vec<(String, String)> {
        let Some(dir) = &self.claude_dir else {
            return Vec::new();
        };
        match supraman_claude::plugin_server(dir, AGENTS_PLUGIN, AGENTS_SERVER) {
            Ok(server) => vec![(AGENTS_SERVER.to_string(), server)],
            Err(why) => {
                log::info!("{name}: no {AGENTS_SERVER} MCP server in its config: {why}");
                Vec::new()
            }
        }
    }

    fn allowed_tools_args(&self) -> Vec<String> {
        if self.allowed_tools.is_empty() {
            return Vec::new();
        }
        std::iter::once("--allowedTools".to_string()).chain(self.allowed_tools.iter().cloned()).collect()
    }

    /// The plugin and `--allowedTools` flags of a foreground `claude` (a [+] session); its env is [`Self::session_env`].
    pub fn interactive_args(&self) -> Vec<String> {
        let plugins = self.plugin_dirs.iter().flat_map(|d| ["--plugin-dir".to_string(), d.to_string_lossy().into_owned()]);
        plugins.chain(self.allowed_tools_args()).collect()
    }

    /// The session's `SUPRAMAN_HOME` and `SUPRAMAN_*` vars; empty without a home.
    pub fn session_env(&self, workspace: &str, name: &str, role: &str, repo: Option<&str>, extra: &[(&str, &str)]) -> Vec<(String, String)> {
        let Some(home) = &self.supraman_home else {
            return Vec::new();
        };
        let session = SessionEnv {
            role: Some(role.to_string()),
            workspace: Some(workspace.to_string()),
            session: Some(name.to_string()),
            repo: repo.map(str::to_string),
        };
        let mut env: Vec<(String, String)> = session.vars().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        env.extend(supraman_core::home::home_env(home).map(|(k, v)| (k.to_string(), v)));
        for (k, v) in extra {
            env.retain(|(have, _)| have != k);
            env.push((k.to_string(), v.to_string()));
        }
        env
    }

    fn settings_with(&self, workspace: &str, name: &str, role: &str, repo: Option<&str>, extra: &[(&str, &str)]) -> Option<String> {
        let mut settings = serde_json::Map::new();
        let mut env = self.session_env(workspace, name, role, repo, extra);
        if !env.is_empty() {
            env.extend(supraman_core::install::sm_first_path().map(|path| ("PATH".to_string(), path)));
            let env: serde_json::Map<String, serde_json::Value> = env.into_iter().map(|(k, v)| (k, v.into())).collect();
            settings.insert("env".into(), env.into());
        }
        if self.bg_isolation_none {
            settings.insert("worktree".into(), serde_json::json!({ "bgIsolation": "none" }));
        }
        (!settings.is_empty()).then(|| serde_json::Value::Object(settings).to_string())
    }
}

/// The plugin whose MCP server ([`AGENTS_SERVER`]) every session Supraman launches gets in its own config.
const AGENTS_PLUGIN: &str = "workman";

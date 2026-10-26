//! Role tokens for the sessions the app launches: `settings.toml` may name a WorkState `.env`
//! (`workstate_env`) and URL (`workstate_url`); the token of role `r` is its `WORKSTATE_<R>_TOKEN`
//! line, else the line of the WorkState role it acts as ([`workstate_role`]), read when the session
//! launches. No file, no line: the session launches without WorkState
//! (`--strict-mcp-config`, so never with the user-scope server). A token is never logged. The token
//! is only the bearer of the session's own `wss_` token's mint ([`foreman_supervisor::mint`]): it
//! never reaches a file.

use std::path::PathBuf;

pub use foreman_core::{DEFAULT_WORKSTATE_URL, read_env_value};
use foreman_core::Settings;
use foreman_supervisor::{RoleToken, TokenProvider};

pub struct AppTokens {
    pub home: PathBuf,
}

impl TokenProvider for AppTokens {
    fn token(&self, role: &str) -> Option<RoleToken> {
        let settings = Settings::load(&self.home).ok()?;
        token_from(&settings, role)
    }
}

pub fn token_from(settings: &Settings, role: &str) -> Option<RoleToken> {
    let env = settings.workstate_env.as_deref()?;
    let token = match read_env_value(env, &token_key(role)).ok()? {
        Some(token) => token,
        None => read_env_value(env, &token_key(workstate_role(role)?)).ok()??,
    };
    Some(RoleToken {
        url: settings.workstate_url.clone().unwrap_or_else(|| DEFAULT_WORKSTATE_URL.into()),
        token,
        role: workstate_role(role).unwrap_or(role).to_string(),
    })
}

/// WorkState knows controller, orchestrator and worker. A workspace session runs the workspace's
/// queue (controller); a repo session runs one repo's units (orchestrator).
pub fn workstate_role(role: &str) -> Option<&'static str> {
    match role {
        "workspace" => Some("controller"),
        "repo" | "repo-controller" => Some("orchestrator"),
        _ => None,
    }
}

pub fn token_key(role: &str) -> String {
    format!("WORKSTATE_{}_TOKEN", role.to_uppercase().replace('-', "_"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const FAKE: &str = "# fake WorkState env for tests\n\
WORKSTATE_CONTROLLER_TOKEN=fake-controller\n\
export WORKSTATE_WORKSPACE_TOKEN = \"fake-workspace\"\n\
WORKSTATE_REPO_CONTROLLER_TOKEN='fake-repo'\n\
WORKSTATE_EMPTY_TOKEN=\n\
not a line\n";

    #[test]
    fn provider_uses_settings_and_falls_back_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let tokens = AppTokens { home: home.clone() };
        assert_eq!(tokens.token("workspace"), None, "no settings file");
        let env = dir.path().join(".env");
        std::fs::write(&env, FAKE).unwrap();
        let mut settings = Settings {
            workstate_env: Some(env),
            ..Settings::default()
        };
        settings.save(&home).unwrap();
        assert_eq!(
            tokens.token("workspace"),
            Some(RoleToken {
                url: DEFAULT_WORKSTATE_URL.into(),
                token: "fake-workspace".into(),
                role: "controller".into(),
            })
        );
        assert_eq!(tokens.token("worker"), None, "no line for the role");
        assert_eq!(
            tokens.token("workspace").unwrap().token,
            "fake-workspace",
            "the role's own line wins over the WorkState role's"
        );
        let mapped = dir.path().join("mapped.env");
        std::fs::write(&mapped, "WORKSTATE_CONTROLLER_TOKEN=fake-controller
WORKSTATE_ORCHESTRATOR_TOKEN=fake-orch
").unwrap();
        settings.workstate_env = Some(mapped);
        settings.save(&home).unwrap();
        assert_eq!(tokens.token("workspace").unwrap().token, "fake-controller");
        assert_eq!(tokens.token("repo").unwrap().token, "fake-orch");
        assert_eq!(tokens.token("repo").unwrap().role, "orchestrator", "minted as the WorkState role it acts as");
        assert_eq!(tokens.token("worker"), None);
        settings.workstate_env = Some(dir.path().join(".env"));
        settings.workstate_url = Some("http://localhost:9/mcp".into());
        settings.save(&home).unwrap();
        assert_eq!(tokens.token("workspace").unwrap().url, "http://localhost:9/mcp");
        settings.workstate_env = Some(dir.path().join("gone.env"));
        settings.save(&home).unwrap();
        assert_eq!(tokens.token("workspace"), None, "unreadable file");
    }

    /// A WorkState stand-in on 127.0.0.1: answers each request by its method and sends back its
    /// request line, `Authorization` header and body.
    fn fake_workstate() -> (String, std::sync::mpsc::Receiver<(String, String, String)>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut line, mut auth, mut len) = (String::new(), String::new(), 0);
                reader.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let Some((k, v)) = h.trim_end().split_once(':') else { break };
                    match k.to_ascii_lowercase().as_str() {
                        "authorization" => auth = v.trim().to_string(),
                        "content-length" => len = v.trim().parse().unwrap(),
                        _ => {}
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let answer = if line.starts_with("POST") {
                    r#"{"sessionId":"91","name":"fm-demo-workspace","role":"controller","token":"wss_selftest","expiresAt":"2026-10-29T00:00:00Z"}"#
                } else {
                    ""
                };
                let status = if line.starts_with("POST") { "201 Created" } else { "204 No Content" };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len()).unwrap();
                if tx.send((line.trim_end().to_string(), auth, String::from_utf8(body).unwrap())).is_err() {
                    break;
                }
            }
        });
        (url, rx)
    }

    struct NoClaude(std::cell::RefCell<Vec<Vec<String>>>);

    impl foreman_supervisor::Launcher for NoClaude {
        fn launch(&self, spec: &foreman_claude::LaunchSpec) -> foreman_claude::Result<String> {
            self.0.borrow_mut().push(spec.argv().iter().map(|a| a.to_string_lossy().into_owned()).collect());
            Ok("backgrounded · aaaa0001 · fm-demo-workspace".into())
        }
        fn resume(&self, _: &str, _: &Path, _: Option<&str>) -> foreman_claude::Result<String> {
            unreachable!()
        }
        fn stop(&self, _: &str) -> foreman_claude::Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn mint_from_the_env_file_over_http_then_revoke_on_stop() {
        use foreman_supervisor::{Budget, DesiredState, LaunchConfig, Supervisor};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let (url, seen) = fake_workstate();
        let env = dir.path().join(".env");
        std::fs::write(&env, "WORKSTATE_CONTROLLER_TOKEN=fake-static-controller\n").unwrap();
        Settings { workstate_env: Some(env), workstate_url: Some(url.clone()), ..Settings::default() }.save(&home).unwrap();
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo", "be the controller");
        desired.started = true;
        let config = LaunchConfig { mcp_dir: home.join("mcp"), foreman_home: Some(home.clone()), ..LaunchConfig::default() };
        let mut sup = Supervisor::new(desired, config, NoClaude(Default::default()), AppTokens { home: home.clone() });
        let now = 1_800_000_000_000;
        sup.tick(&[], &mut Budget { max: 1, free: 1 }, now);

        let (line, auth, body) = seen.recv().unwrap();
        assert_eq!(line, "POST /api/agent-sessions HTTP/1.1");
        assert_eq!(auth, "Bearer fake-static-controller");
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!((&body["role"], &body["externalRole"], &body["replace"]), (&"controller".into(), &"workspace".into(), &true.into()));
        let file = home.join("mcp").join("workstate-fm-demo-workspace.json");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("Bearer wss_selftest") && text.contains(&url), "{text}");
        assert!(!text.contains("fake-static-controller"), "{text}");
        let argv = sup.launcher().0.borrow().concat().join(" ");
        assert!(!argv.contains("fake-static-controller") && !argv.contains("wss_"), "{argv}");

        sup.desired.started = false;
        sup.tick(&[], &mut Budget { max: 1, free: 1 }, now + 1_000);
        assert_eq!(seen.recv().unwrap().0, "DELETE /api/agent-sessions/91 HTTP/1.1");
        assert!(!file.exists());
        assert!(sup.take_notices().is_empty());
    }
}

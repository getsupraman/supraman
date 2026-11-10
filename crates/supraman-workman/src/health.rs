//! `GET /health` is unauthenticated (Skills `docs/workstate/12-install-harness-identity.md`).

use std::time::Duration;


/// `Err` is one short line: `HTTP 503`, `connection refused`, `timed out`.
pub fn health(mcp_url: &str, timeout: Duration) -> Result<(), String> {
    let agent = crate::http::agent(timeout);
    let res = agent.get(format!("{}/health", supraman_core::url::origin(mcp_url))).call().map_err(|e| transport(&e))?;
    let status = res.status().as_u16();
    if (200..300).contains(&status) { Ok(()) } else { Err(format!("HTTP {status}")) }
}

/// Whether the MCP endpoint at `mcp_url` refuses `token` (401 or 403) to a sessionless `ping`; any other outcome is no.
pub fn refuses(mcp_url: &str, token: &str, timeout: Duration) -> bool {
    let res = crate::http::agent(timeout)
        .post(mcp_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json, text/event-stream")
        .content_type("application/json")
        .send(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#);
    res.is_ok_and(|r| matches!(r.status().as_u16(), 401 | 403))
}

fn transport(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => "connection refused".into(),
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => "timed out".into(),
        ureq::Error::Timeout(_) => "timed out".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::closed_base;
    use crate::fake::{Answer, FakeWorkman};

    const QUICK: Duration = Duration::from_secs(3);

    #[test]
    fn a_2xx_is_healthy_anything_else_says_why() {
        let fake = FakeWorkman::start(vec![("/health".into(), Answer::text("Healthy"))]);
        assert_eq!(health(&fake.mcp_url(), QUICK), Ok(()));
        let seen = fake.seen();
        assert_eq!(seen.last().map(|s| (s.target.as_str(), s.authorization.clone())), Some(("/health", None)), "no token sent");
        fake.set("/health", Answer::status(503, "Unhealthy"));
        assert_eq!(health(&fake.mcp_url(), QUICK), Err("HTTP 503".into()));
    }

    #[test]
    fn refuses_is_a_401_or_403_of_the_mcp_endpoint_and_nothing_else() {
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        assert!(refuses(&fake.mcp_url(), "wss_unknown", QUICK), "a token Workman does not know");
        let minted = crate::http::read(crate::http::agent(QUICK).post(format!("{}/api/agent-sessions", fake.base())).send(r#"{"name":"x","launchedBy":"t"}"#)).unwrap();
        let token = serde_json::from_str::<serde_json::Value>(&minted.body).unwrap()["token"].as_str().unwrap().to_string();
        assert!(sessions.live(&token));
        assert!(!refuses(&fake.mcp_url(), &token, QUICK), "a live token");
        let seen = fake.seen();
        let ping = seen.last().unwrap();
        assert_eq!((ping.method.as_str(), ping.target.as_str()), ("POST", "/mcp"));
        assert_eq!(ping.authorization.as_deref(), Some(format!("Bearer {token}").as_str()));
        assert!(!refuses(&format!("{}/mcp", closed_base()), &token, QUICK), "down is no refusal");
    }

    #[test]
    fn a_closed_port_is_connection_refused() {
        assert_eq!(health(&format!("{}/mcp", closed_base()), QUICK), Err("connection refused".into()));
    }

    #[test]
    fn a_server_that_never_answers_is_timed_out() {
        let silent = FakeWorkman::start(vec![("/health".into(), Answer::silent())]);
        assert_eq!(health(&silent.mcp_url(), Duration::from_millis(300)), Err("timed out".into()));
    }
}

//! Is a WorkState server listening: one unauthenticated `GET /health` (Skills
//! `docs/workstate/12-install-harness-identity.md`), so no token is involved.

use std::time::Duration;

use crate::client::api_base;

/// `GET <origin of mcp_url>/health` answered 2xx within `timeout`; else what went wrong, in one
/// short line (`HTTP 503`, `connection refused`, `timed out`).
pub fn health(mcp_url: &str, timeout: Duration) -> Result<(), String> {
    let agent = crate::http::agent(timeout);
    let res = agent.get(format!("{}/health", api_base(mcp_url))).call().map_err(|e| transport(&e))?;
    let status = res.status().as_u16();
    if (200..300).contains(&status) { Ok(()) } else { Err(format!("HTTP {status}")) }
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
    use crate::fake::{Answer, FakeWorkState};

    const QUICK: Duration = Duration::from_secs(3);

    #[test]
    fn a_2xx_is_healthy_anything_else_says_why() {
        let fake = FakeWorkState::start(vec![("/health".into(), Answer::text("Healthy"))]);
        assert_eq!(health(&fake.mcp_url(), QUICK), Ok(()));
        let seen = fake.seen();
        assert_eq!(seen.last().map(|s| (s.target.as_str(), s.authorization.clone())), Some(("/health", None)), "no token sent");
        fake.set("/health", Answer::status(503, "Unhealthy"));
        assert_eq!(health(&fake.mcp_url(), QUICK), Err("HTTP 503".into()));
    }

    #[test]
    fn a_closed_port_is_connection_refused() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(health(&format!("http://127.0.0.1:{port}/mcp"), QUICK), Err("connection refused".into()));
    }

    #[test]
    fn a_server_that_never_answers_is_timed_out() {
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", silent.local_addr().unwrap());
        assert_eq!(health(&url, Duration::from_millis(300)), Err("timed out".into()));
    }
}

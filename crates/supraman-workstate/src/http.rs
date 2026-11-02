//! The one HTTP agent every WorkState call and the update check use, and how its answers are read.

use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub fn agent(timeout: Duration) -> ureq::Agent {
    agent_following(timeout, 0)
}

/// [`agent`] that follows up to `redirects` redirects: a release asset's download URL answers with one.
pub fn agent_following(timeout: Duration, redirects: u32) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(redirects)
        .timeout_global(Some(timeout))
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
}

/// An answer read whole.
pub struct Reply {
    pub status: u16,
    pub headers: ureq::http::HeaderMap,
    pub body: String,
}

/// The status, headers and body of `res`; `Err` is why sending it or reading its body failed.
pub fn read(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Reply, String> {
    let mut res = res.map_err(|e| e.to_string())?;
    let body = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(Reply { status: res.status().as_u16(), headers: res.headers().clone(), body })
}

/// RFC 3986 unreserved bytes kept, every other byte `%XX`: a path segment or a query value.
pub fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// What a refusal says: `error`/`title`/`message`/`detail` of a JSON body, else its start; at most 200 characters.
pub fn server_reason(body: &str) -> String {
    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| ["error", "title", "message", "detail"].iter().find_map(|k| v.get(*k)?.as_str().map(str::to_string)))
        .unwrap_or_else(|| body.trim().to_string());
    reason.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_encode_keeps_unreserved_bytes_only() {
        assert_eq!(pct_encode("aZ09-_.~"), "aZ09-_.~");
        assert_eq!(pct_encode("m4/1 x"), "m4%2F1%20x");
        assert_eq!(pct_encode("ü"), "%C3%BC");
    }

    #[test]
    fn server_reason_takes_the_first_known_field_else_the_body_cut_short() {
        assert_eq!(server_reason(r#"{"title":"t","error":"e"}"#), "e");
        assert_eq!(server_reason(r#"{"detail":"d"}"#), "d");
        assert_eq!(server_reason("  plain  "), "plain");
        assert_eq!(server_reason(&"x".repeat(300)).len(), 200);
    }
}

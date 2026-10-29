//! The one HTTP agent every WorkState call uses: blocking HTTP/1.1, TLS through the OS (schannel,
//! the machine's own root store) for an `https://` server, no proxy, no redirects, statuses as values.

use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
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

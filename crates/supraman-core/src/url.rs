/// A URL's origin, `scheme://authority`: cut at the first `/`, `?` or `#` after the scheme, spelling
/// kept. The key of `servers.json` and of every minted session's origin.
pub fn origin(url: &str) -> String {
    let url = url.trim();
    let start = url.find("://").map_or(0, |i| i + 3);
    match url[start..].find(['/', '?', '#']) {
        Some(i) => url[..start + i].to_string(),
        None => url.trim_end_matches('/').to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `/`-only cut that the keys already in `servers.json` and the minted records use.
    fn old_api_base(mcp_url: &str) -> String {
        let start = mcp_url.find("://").map_or(0, |i| i + 3);
        match mcp_url[start..].find('/') {
            Some(i) => mcp_url[..start + i].to_string(),
            None => mcp_url.trim_end_matches('/').to_string(),
        }
    }

    #[test]
    fn every_origin_the_tests_use_is_unchanged() {
        for (url, want) in [
            ("http://127.0.0.1:1/mcp", "http://127.0.0.1:1"),
            ("http://127.0.0.1:1/", "http://127.0.0.1:1"),
            ("http://127.0.0.1:1", "http://127.0.0.1:1"),
            ("http://127.0.0.1:9", "http://127.0.0.1:9"),
            ("http://127.0.0.1:5/callback", "http://127.0.0.1:5"),
            ("http://127.0.0.1/callback", "http://127.0.0.1"),
            ("http://127.0.0.1:41000/mcp/${FM_PROBE}", "http://127.0.0.1:41000"),
            ("http://127.0.0.1:41000/connect/revoke", "http://127.0.0.1:41000"),
            ("http://[::1]:9", "http://[::1]:9"),
            ("http://localhost:9", "http://localhost:9"),
            ("http://localhost:9/", "http://localhost:9"),
            ("http://localhost:9/api", "http://localhost:9"),
            ("http://x/mcp", "http://x"),
            ("http://work.supraman.ai", "http://work.supraman.ai"),
            ("https://work.supraman.ai", "https://work.supraman.ai"),
            ("https://work.supraman.ai/mcp", "https://work.supraman.ai"),
            ("https://workstate.example", "https://workstate.example"),
            ("https://workstate.example/mcp", "https://workstate.example"),
            ("https://WorkState.Example/mcp", "https://WorkState.Example"),
            ("https://b.example", "https://b.example"),
            ("https://b.example/", "https://b.example"),
            ("https://ws.example", "https://ws.example"),
            ("https://ws.example/tenant", "https://ws.example"),
            ("https://issuer.invalid/", "https://issuer.invalid"),
            ("https://other.example", "https://other.example"),
            ("https://", "https:"),
            ("http://127.0.0.1:", "http://127.0.0.1:"),
        ] {
            assert_eq!(origin(url), want, "{url}");
            assert_eq!(origin(url), old_api_base(url), "{url}: as api_base keyed it");
        }
    }

    #[test]
    fn a_query_fragment_or_surrounding_space_is_not_part_of_the_origin() {
        for (url, want) in [
            ("https://ws.example?x=1", "https://ws.example"),
            ("https://ws.example#top", "https://ws.example"),
            ("https://ws.example:8443?x=/y", "https://ws.example:8443"),
            ("  https://ws.example/mcp \n", "https://ws.example"),
            ("ws.example/mcp", "ws.example"),
        ] {
            assert_eq!(origin(url), want, "{url}");
        }
    }
}

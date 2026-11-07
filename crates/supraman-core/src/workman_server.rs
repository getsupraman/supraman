//! `~/.workman/server.json` (Workman rule C183): the one source of the Workman server's URL.

use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::UserMcpServer;

/// What a missing or unusable file tells the user to do.
pub const NOT_SET_UP: &str = "Workman is not set up: run /workman:setup";
/// The MCP server name `/workman:setup` registers.
pub const MCP_NAME: &str = "workman";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerJson {
    pub url: String,
    #[serde(rename = "mcpName", default = "default_mcp_name")]
    pub mcp_name: String,
}

fn default_mcp_name() -> String {
    MCP_NAME.into()
}

impl ServerJson {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into(), mcp_name: MCP_NAME.into() }
    }

    pub fn origin(&self) -> String {
        crate::url::origin(&self.url)
    }

    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.origin())
    }

    /// Usable, but claude knows the server under another name than the one Supraman's sessions are told.
    pub fn warning(&self) -> Option<String> {
        (self.mcp_name != MCP_NAME).then(|| {
            format!("~/.workman/server.json names the Workman MCP server \"{}\", not \"{MCP_NAME}\"; run /workman:setup to register it as {MCP_NAME}", self.mcp_name)
        })
    }
}

/// What claude's user config ([`crate::paths::claude_json`] at `path`) lacks for `server`: a user-scope `workman`
/// entry whose url is `<origin>/mcp`, as `/workman:setup` registers it. Never quotes claude's url.
pub fn claude_warning(server: &ServerJson, path: &Path, found: Result<UserMcpServer, String>) -> Option<String> {
    let shown = path.display();
    let fix = "run /workman:setup to register it, then restart claude";
    match found {
        Err(why) => Some(format!("claude's Workman server is not checked: {why}.")),
        Ok(UserMcpServer::NoFile) => Some(format!("claude has no Workman server: {shown} does not exist; {fix}.")),
        Ok(UserMcpServer::Missing) => Some(format!("claude has no user-scope MCP server named \"{MCP_NAME}\" in {shown}; {fix}.")),
        Ok(UserMcpServer::Url(url)) => {
            let base = url.as_deref().map(|u| u.trim().trim_end_matches('/')).and_then(|u| u.strip_suffix("/mcp"));
            (!base.is_some_and(|b| b.eq_ignore_ascii_case(&server.origin()))).then(|| {
                format!("claude's user-scope \"{MCP_NAME}\" server in {shown} is not {}, the one ~/.workman/server.json names; {fix}.", server.mcp_url())
            })
        }
    }
}

/// `path` read and checked; every refusal says to run `/workman:setup` and nothing is defaulted.
pub fn load(path: &Path) -> Result<ServerJson, String> {
    let text = match std::fs::read(path) {
        Ok(bytes) => crate::text::decode(&bytes).0,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(format!("{NOT_SET_UP} (it writes {}).", path.display())),
        Err(e) => return Err(format!("{NOT_SET_UP}: {} cannot be read ({e}).", path.display())),
    };
    let server: ServerJson = serde_json::from_str(&text).map_err(|e| format!("{NOT_SET_UP}: {} is not valid ({e}).", path.display()))?;
    let url = validate_workman_url(&server.url).map_err(|why| format!("{NOT_SET_UP}: the url in {} is not usable ({why}).", path.display()))?;
    Ok(ServerJson { url, ..server })
}

/// [`load`] of [`crate::paths::workman_server_json`]'s answer; `None` is a machine with no user home.
pub fn load_from(path: Option<&Path>) -> Result<ServerJson, String> {
    match path {
        Some(path) => load(path),
        None => Err(format!("{NOT_SET_UP} (no user home: USERPROFILE is not set).")),
    }
}

pub fn save(path: &Path, server: &ServerJson) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(server).map_err(std::io::Error::other)?;
    crate::fs::write_atomic(path, text)
}

/// A server URL as the origin to keep: `http(s)://host[:port]`, any path dropped. Plain `http` only for this machine
/// (a token must not cross a network in the clear).
pub fn validate_workman_url(text: &str) -> Result<String, String> {
    let text = text.trim();
    let (scheme, rest) = text.split_once("://").ok_or("a URL starts with https:// (or http:// for this machine)")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("{scheme}:// is not http or https"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace) {
        return Err("the URL names no host".into());
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        let (h, port) = v6.split_once(']').ok_or("an IPv6 host is closed with ]")?;
        if !port.is_empty() && port.strip_prefix(':').is_none_or(|p| p.parse::<u16>().is_err()) {
            return Err(format!("{port} is not a port"));
        }
        h.to_string()
    } else {
        let (h, port) = authority.rsplit_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
        if let Some(p) = port
            && p.parse::<u16>().is_err()
        {
            return Err(format!("{p} is not a port"));
        }
        if h.is_empty() || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return Err(format!("{h} is not a host name"));
        }
        h.to_ascii_lowercase()
    };
    let local = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1");
    if scheme == "http" && !local {
        return Err("use https:// for a server that is not on this machine".into());
    }
    Ok(format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_says_run_workman_setup_and_defaults_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".workman").join("server.json");
        let e = load(&path).unwrap_err();
        assert!(e.starts_with(NOT_SET_UP) && e.contains(&path.display().to_string()), "{e}");
        let e = load_from(None).unwrap_err();
        assert!(e.starts_with(NOT_SET_UP), "{e}");
    }

    #[test]
    fn save_then_load_round_trips_with_mcp_name_spelt_as_setup_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".workman").join("server.json");
        save(&path, &ServerJson::new("http://127.0.0.1:9")).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((raw["url"].as_str(), raw["mcpName"].as_str()), (Some("http://127.0.0.1:9"), Some("workman")));
        let back = load(&path).unwrap();
        assert_eq!((back.origin(), back.mcp_url(), back.warning()), ("http://127.0.0.1:9".into(), "http://127.0.0.1:9/mcp".into(), None));
    }

    #[test]
    fn setups_own_file_with_a_path_and_a_bom_loads_as_its_origin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        std::fs::write(&path, "\u{feff}{\"url\": \"https://Work.Example/mcp\", \"mcpName\": \"workman\", \"extra\": 1}").unwrap();
        assert_eq!(load(&path).unwrap(), ServerJson::new("https://work.example"));
    }

    #[test]
    fn another_mcp_name_is_a_warning_and_still_usable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        std::fs::write(&path, r#"{"url":"https://work.example","mcpName":"ws"}"#).unwrap();
        let server = load(&path).unwrap();
        assert_eq!(server.origin(), "https://work.example");
        assert!(server.warning().is_some_and(|w| w.contains("\"ws\"") && w.contains("/workman:setup")), "{:?}", server.warning());
    }

    #[test]
    fn a_damaged_file_or_a_bad_url_is_a_refusal_not_a_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        for bad in ["{", "{}", r#"{"url":"http://work.example"}"#, r#"{"url":"ftp://x"}"#, r#"{"url":5}"#] {
            std::fs::write(&path, bad).unwrap();
            let e = load(&path).unwrap_err();
            assert!(e.starts_with(NOT_SET_UP), "{bad}: {e}");
        }
    }

    #[test]
    fn claude_is_warned_of_until_its_user_workman_entry_is_this_servers_mcp_url() {
        let server = ServerJson::new("https://work.example");
        let path = Path::new(r"C:\u\.claude.json");
        let warn = |found| claude_warning(&server, path, found);
        for (found, says) in [
            (Ok(UserMcpServer::NoFile), "does not exist"),
            (Ok(UserMcpServer::Missing), "no user-scope MCP server named \"workman\""),
            (Ok(UserMcpServer::Url(None)), "is not https://work.example/mcp"),
            (Ok(UserMcpServer::Url(Some("https://other.example/mcp?token=SECRET".into()))), "is not https://work.example/mcp"),
            (Ok(UserMcpServer::Url(Some("https://work.example/x/mcp".into()))), "is not https://work.example/mcp"),
            (Err(r"C:\u\.claude.json is not valid JSON (eof)".into()), "is not checked"),
        ] {
            let w = warn(found).unwrap_or_default();
            assert!(w.contains(says) && !w.contains("SECRET"), "{says}: {w}");
        }
        assert!(warn(Ok(UserMcpServer::Missing)).unwrap().contains("/workman:setup"));
        for same in ["https://work.example/mcp", "https://Work.Example/mcp/", " https://work.example/mcp "] {
            assert_eq!(warn(Ok(UserMcpServer::Url(Some(same.into())))), None, "{same}");
        }
    }

    #[test]
    fn validate_workman_url_keeps_an_origin_and_https_off_this_machine() {
        assert_eq!(validate_workman_url(" https://Work.Supraman.ai/ ").as_deref(), Ok("https://work.supraman.ai"));
        assert_eq!(validate_workman_url("https://work.supraman.ai/mcp").as_deref(), Ok("https://work.supraman.ai"));
        assert_eq!(validate_workman_url("http://127.0.0.1:1").as_deref(), Ok("http://127.0.0.1:1"));
        assert_eq!(validate_workman_url("http://localhost:9/api").as_deref(), Ok("http://localhost:9"));
        assert_eq!(validate_workman_url("http://[::1]:9").as_deref(), Ok("http://[::1]:9"));
        for bad in ["work.supraman.ai", "ftp://x", "http://work.supraman.ai", "https://", "https://a b", "https://x:port", "https://u@x"] {
            assert!(validate_workman_url(bad).is_err(), "{bad}");
        }
    }
}

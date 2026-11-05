//! Step 2's own claude session loads no MCP server of the user's: a signed-out user-scope `workman` would answer
//! its connect with a 401 that claude caches by server name for every later session (workman/backlog/launch-mcp-needs-auth).

use std::path::Path;

/// The argv of `claude` for the session `name`, with `config` as its only MCP config.
pub(crate) fn args<'a>(name: &'a str, config: &'a str, prompt: &'a str) -> [&'a str; 9] {
    ["--bg", "--name", name, "--permission-mode", "auto", "--mcp-config", config, "--strict-mcp-config", prompt]
}

/// `<home>/no-mcp.json`, an MCP config of no servers; its path.
pub(crate) fn no_servers(home: &Path) -> std::io::Result<String> {
    let path = home.join("no-mcp.json");
    std::fs::write(&path, r#"{"mcpServers":{}}"#)?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_reads_only_an_empty_config() {
        let home = std::env::temp_dir().join(format!("fm-selftest-launch-{}", supraman_core::time::hex6()));
        std::fs::create_dir_all(&home).unwrap();
        let config = no_servers(&home).unwrap();
        let servers: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(servers, serde_json::json!({ "mcpServers": {} }));

        let argv = args("fm-selftest-1", &config, "hi");
        assert!(argv.contains(&"--strict-mcp-config"), "{argv:?}");
        let at = argv.iter().position(|a| *a == "--mcp-config").expect("--mcp-config");
        assert_eq!(argv[at + 1], config);
        assert!(argv[at + 2].starts_with("--"), "--mcp-config takes every value up to the next flag: {argv:?}");
        assert_eq!(argv.last(), Some(&"hi"));
        std::fs::remove_dir_all(&home).ok();
    }
}

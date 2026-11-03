//! An MCP server a Claude Code plugin declares, for a session whose `--strict-mcp-config` starts no plugin server
//! (LEARNINGS workman-2/6).

use std::path::{Path, PathBuf};

const ROOT_VAR: &str = "${CLAUDE_PLUGIN_ROOT}";

/// `server` as the installed plugin `plugin` declares it in its `.mcp.json`, `${CLAUDE_PLUGIN_ROOT}` expanded, as a
/// JSON object for an `--mcp-config`'s `mcpServers`. `Err` says which file was read and why nothing came of it.
pub fn plugin_server(claude_dir: &Path, plugin: &str, server: &str) -> Result<String, String> {
    let installed = claude_dir.join("plugins").join("installed_plugins.json");
    let root = install_path(&installed, plugin)?;
    let declared = root.join(".mcp.json");
    let text = std::fs::read_to_string(&declared).map_err(|e| format!("{}: {e}", declared.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: not JSON ({e})", declared.display()))?;
    let servers = json.get("mcpServers").unwrap_or(&json);
    let Some(entry) = servers.get(server).filter(|e| e.is_object()) else {
        return Err(format!("{} declares no server {server:?}", declared.display()));
    };
    let mut entry = entry.clone();
    let root_text = root.to_string_lossy().into_owned();
    expand(&mut entry, &root_text);
    let text = entry.to_string();
    if let Some(at) = text.find("${") {
        let var: String = text[at..].chars().take_while(|c| *c != '}').chain(std::iter::once('}')).collect();
        return Err(format!("{}: server {server:?} uses {var}, which Supraman does not set", declared.display()));
    }
    Ok(text)
}

/// The install folder of `plugin` (`<name>@<marketplace>`'s name part) that exists on disk.
fn install_path(installed: &Path, plugin: &str) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(installed).map_err(|e| format!("{}: {e}", installed.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: not JSON ({e})", installed.display()))?;
    let Some(plugins) = json.get("plugins").and_then(|p| p.as_object()) else {
        return Err(format!("{}: no \"plugins\" object", installed.display()));
    };
    let installs = plugins.iter().filter(|(key, _)| key.split('@').next() == Some(plugin)).flat_map(|(_, v)| match v {
        serde_json::Value::Array(all) => all.clone(),
        one => vec![one.clone()],
    });
    let paths: Vec<PathBuf> = installs.filter_map(|i| i.get("installPath").and_then(|p| p.as_str()).map(PathBuf::from)).collect();
    if paths.is_empty() {
        return Err(format!("{}: plugin {plugin:?} is not installed", installed.display()));
    }
    paths.into_iter().find(|p| p.is_dir()).ok_or_else(|| format!("{}: plugin {plugin:?}'s install folder is gone", installed.display()))
}

fn expand(value: &mut serde_json::Value, root: &str) {
    match value {
        serde_json::Value::String(s) => *s = s.replace(ROOT_VAR, root),
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| expand(v, root)),
        serde_json::Value::Object(map) => map.values_mut().for_each(|v| expand(v, root)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_with(installed: serde_json::Value, mcp: Option<&str>) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache").join("workman").join("2.3.0");
        std::fs::create_dir_all(&root).unwrap();
        if let Some(mcp) = mcp {
            std::fs::write(root.join(".mcp.json"), mcp).unwrap();
        }
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(claude.join("plugins")).unwrap();
        let installed = installed.to_string().replace("<root>", &root.to_string_lossy().replace('\\', "\\\\"));
        std::fs::write(claude.join("plugins").join("installed_plugins.json"), installed).unwrap();
        (dir, root)
    }

    const AGENTS: &str = r#"{"mcpServers":{"agents":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/skills/agents/server/agents.mjs"]}}}"#;

    fn installed() -> serde_json::Value {
        serde_json::json!({ "version": 2, "plugins": { "workman@supraman": [{ "scope": "user", "installPath": "<root>", "version": "2.3.0" }] } })
    }

    #[test]
    fn the_plugins_own_declaration_comes_back_with_its_root_expanded() {
        let (dir, root) = claude_with(installed(), Some(AGENTS));
        let got: serde_json::Value = serde_json::from_str(&plugin_server(&dir.path().join("claude"), "workman", "agents").unwrap()).unwrap();
        let script = format!("{}/skills/agents/server/agents.mjs", root.display());
        assert_eq!(got, serde_json::json!({ "command": "node", "args": [script] }));
    }

    #[test]
    fn a_miss_names_the_file_it_read_and_why() {
        let (dir, root) = claude_with(installed(), Some(AGENTS));
        let claude = dir.path().join("claude");
        let installed_file = claude.join("plugins").join("installed_plugins.json").display().to_string();
        let e = plugin_server(&claude, "other", "agents").unwrap_err();
        assert_eq!(e, format!("{installed_file}: plugin \"other\" is not installed"));
        let e = plugin_server(&claude, "workman", "nope").unwrap_err();
        assert_eq!(e, format!("{} declares no server \"nope\"", root.join(".mcp.json").display()));

        std::fs::write(root.join(".mcp.json"), r#"{"mcpServers":{"agents":{"command":"node","args":["${CLAUDE_PLUGIN_DATA}/x.mjs"]}}}"#).unwrap();
        let e = plugin_server(&claude, "workman", "agents").unwrap_err();
        assert!(e.ends_with("uses ${CLAUDE_PLUGIN_DATA}, which Supraman does not set"), "{e}");

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(plugin_server(&claude, "workman", "agents").unwrap_err(), format!("{installed_file}: plugin \"workman\"'s install folder is gone"));
        let e = plugin_server(&dir.path().join("none"), "workman", "agents").unwrap_err();
        assert!(e.contains("installed_plugins.json"), "{e}");
    }
}

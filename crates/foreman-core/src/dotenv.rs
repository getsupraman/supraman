//! Reading one line of a WorkState `.env` (`settings.toml` `workstate_env`). Shared by the app's
//! launch tokens and the read API's reader token; a value is never logged.

use std::io;
use std::path::Path;

/// The value of `key` in a dotenv file: `KEY=value`, optionally `export `-prefixed and quoted;
/// `#` lines and blank lines skipped. `None` when the key is absent or empty.
pub fn read_env_value(path: &Path, key: &str) -> io::Result<Option<String>> {
    Ok(value_in(&std::fs::read_to_string(path)?, key).map(str::to_string))
}

/// Whether `key` has a non-empty value in the file; the value itself never leaves this function.
pub fn has_env_key(path: &Path, key: &str) -> io::Result<bool> {
    Ok(value_in(&std::fs::read_to_string(path)?, key).is_some())
}

fn value_in<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        let v = ['"', '\'']
            .iter()
            .find_map(|q| v.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
            .unwrap_or(v);
        return (!v.is_empty()).then_some(v);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_exported_and_quoted_values() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(".env");
        std::fs::write(
            &env,
            "# fake WorkState env for tests\n\
WORKSTATE_CONTROLLER_TOKEN=fake-controller\n\
export WORKSTATE_WORKSPACE_TOKEN = \"fake-workspace\"\n\
WORKSTATE_REPO_CONTROLLER_TOKEN='fake-repo'\n\
WORKSTATE_EMPTY_TOKEN=\n\
not a line\n",
        )
        .unwrap();
        let read = |k: &str| read_env_value(&env, k).unwrap();
        assert_eq!(read("WORKSTATE_CONTROLLER_TOKEN").as_deref(), Some("fake-controller"));
        assert_eq!(read("WORKSTATE_WORKSPACE_TOKEN").as_deref(), Some("fake-workspace"));
        assert_eq!(read("WORKSTATE_REPO_CONTROLLER_TOKEN").as_deref(), Some("fake-repo"));
        assert_eq!(read("WORKSTATE_EMPTY_TOKEN"), None);
        assert_eq!(read("WORKSTATE_WORKER_TOKEN"), None);
        assert!(read_env_value(&dir.path().join("missing.env"), "X").is_err());
        assert!(has_env_key(&env, "WORKSTATE_WORKSPACE_TOKEN").unwrap());
        assert!(!has_env_key(&env, "WORKSTATE_EMPTY_TOKEN").unwrap());
        assert!(!has_env_key(&env, "WORKSTATE_WORKER_TOKEN").unwrap());
        assert!(has_env_key(&dir.path().join("missing.env"), "X").is_err());
    }
}

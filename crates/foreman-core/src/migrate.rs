//! m2x/2: a Foreman home written while a workspace was called a "platform" is brought to today's names.
//! Idempotent and never deletes data; run at app start and before every `fm` command that reads the
//! registry.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use toml::{Table, Value};

use crate::{CoreError, Result};

const OLD_REGISTRY: &str = "platforms";
const NEW_REGISTRY: &str = "workspaces";
const OLD_ROLE: &str = "platform";
const NEW_ROLE: &str = "workspace";

/// Returns the notices the human has to read; empty when there was nothing to do.
///
/// - `<home>/platforms/` becomes `<home>/workspaces/` (both present: left alone, with a notice).
/// - Every `<home>/supervisor/<w>.toml`: field `platform` → `workspace`, role `platform` → `workspace`,
///   wanted session `fm-<w>-platform` → `fm-<w>-workspace`; written back only when it changed. One that was
///   started is saved stopped: its old-named session is no longer wanted, and the migration stops nothing.
pub fn migrate_home(home: &Path) -> Result<Vec<String>> {
    let mut notices = Vec::new();
    let old = home.join(OLD_REGISTRY);
    let new = home.join(NEW_REGISTRY);
    if old.is_dir() {
        if new.exists() {
            notices.push(format!(
                "both {} and {} exist: Foreman reads only {}; move what you still need out of {} by hand",
                old.display(),
                new.display(),
                new.display(),
                old.display()
            ));
        } else {
            fs::rename(&old, &new).map_err(|e| CoreError::io(&old, e))?;
        }
    }

    let dir = home.join("supervisor");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(notices),
        Err(e) => return Err(CoreError::io(&dir, e)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| CoreError::io(&dir, e))?.path();
        if path.extension().is_some_and(|x| x == "toml") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|e| CoreError::io(&path, e))?;
        let Ok(mut table) = text.parse::<Table>() else {
            notices.push(format!("{} is not valid TOML: not migrated", path.display()));
            continue;
        };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        if let Some(notice) = migrate_desired(&mut table, &stem) {
            let text = toml::to_string(&table).map_err(|source| CoreError::Serialize { name: stem, source })?;
            let tmp = path.with_extension("toml.tmp");
            fs::write(&tmp, text).map_err(|e| CoreError::io(&tmp, e))?;
            fs::rename(&tmp, &path).map_err(|e| CoreError::io(&path, e))?;
            notices.extend(notice);
        }
    }
    Ok(notices)
}

/// `None`: unchanged. `Some(notice)`: changed, with a notice when it had to be stopped.
fn migrate_desired(table: &mut Table, stem: &str) -> Option<Option<String>> {
    let mut changed = false;
    if !table.contains_key(NEW_ROLE)
        && let Some(name) = table.remove(OLD_ROLE)
    {
        table.insert(NEW_ROLE.into(), name);
        changed = true;
    }
    let workspace = table.get(NEW_ROLE).and_then(Value::as_str).unwrap_or(stem).to_string();
    let old_session = format!("fm-{workspace}-{OLD_ROLE}");
    let new_session = format!("fm-{workspace}-{NEW_ROLE}");
    if let Some(Value::Array(sessions)) = table.get_mut("sessions") {
        for session in sessions.iter_mut().filter_map(Value::as_table_mut) {
            if session.get("role").and_then(Value::as_str) == Some(OLD_ROLE) {
                session.insert("role".into(), NEW_ROLE.into());
                changed = true;
            }
            if session.get("name").and_then(Value::as_str) == Some(old_session.as_str()) {
                session.insert("name".into(), new_session.clone().into());
                changed = true;
            }
        }
    }
    if !changed {
        return None;
    }
    if table.get("started").and_then(Value::as_bool) == Some(true) {
        table.insert("started".into(), false.into());
        return Some(Some(format!(
            "workspace {workspace} was started under its old name; its session {old_session} is no longer \
             supervised — stop it in the tree, then press Start"
        )));
    }
    Some(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real home written before the rename (m2/4), with made-up content.
    fn old_home(started: &[(&str, bool)]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let reg = home.path().join("platforms");
        let sup = home.path().join("supervisor");
        fs::create_dir_all(&reg).unwrap();
        fs::create_dir_all(&sup).unwrap();
        fs::create_dir_all(home.path().join("roles")).unwrap();
        fs::write(home.path().join("roles").join("platform.md"), "You are the platform controller for {platform}.\n")
            .unwrap();
        for (w, started) in started {
            fs::write(
                reg.join(format!("{w}.toml")),
                format!("name = \"{w}\"\nroot = 'C:\\{w}'\ncreated_at = \"2026-10-21T10:00:00Z\"\n"),
            )
            .unwrap();
            fs::write(
                sup.join(format!("{w}.toml")),
                format!(
                    "platform = \"{w}\"\nstarted = {started}\n\n[[sessions]]\nname = \"fm-{w}-platform\"\nrole = \"platform\"\n\
                     cwd = 'C:\\{w}'\nprompt = \"You are the platform controller for {w}\"\npermission_mode = \"auto\"\n\n\
                     [[sessions]]\nname = \"fm-{w}-alpha\"\nrole = \"repo\"\ncwd = 'C:\\{w}\\Alpha'\nrepo = \"Alpha\"\n\
                     prompt = \"p\"\npermission_mode = \"default\"\n"
                ),
            )
            .unwrap();
        }
        home
    }

    fn snapshot(home: &Path) -> Vec<(String, String)> {
        let mut files = Vec::new();
        let mut dirs = vec![home.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    let rel = path.strip_prefix(home).unwrap().to_string_lossy().replace('\\', "/");
                    files.push((rel, fs::read_to_string(&path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    fn desired(home: &Path, w: &str) -> Table {
        fs::read_to_string(home.join("supervisor").join(format!("{w}.toml"))).unwrap().parse().unwrap()
    }

    #[test]
    fn an_old_home_is_migrated_once_and_the_second_run_changes_nothing() {
        let home = old_home(&[("nomos", false), ("live", true)]);
        let notices = migrate_home(home.path()).unwrap();
        assert_eq!(
            notices,
            ["workspace live was started under its old name; its session fm-live-platform is no longer supervised \
              — stop it in the tree, then press Start"]
        );
        assert!(!home.path().join("platforms").exists());
        assert!(home.path().join("workspaces").join("nomos.toml").is_file());
        let names: Vec<_> = crate::Registry::at(home.path()).list().unwrap().into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["live", "nomos"]);
        assert!(home.path().join("roles").join("platform.md").is_file(), "nothing is deleted");

        for (w, started) in [("nomos", false), ("live", false)] {
            let d = desired(home.path(), w);
            assert_eq!(d.get("workspace").and_then(Value::as_str), Some(w));
            assert!(!d.contains_key("platform"));
            assert_eq!(d.get("started").and_then(Value::as_bool), Some(started));
            let sessions = d["sessions"].as_array().unwrap();
            assert_eq!(sessions[0]["name"].as_str(), Some(format!("fm-{w}-workspace").as_str()));
            assert_eq!(sessions[0]["role"].as_str(), Some("workspace"));
            assert_eq!(sessions[0]["prompt"].as_str(), Some(format!("You are the platform controller for {w}").as_str()));
            assert_eq!(sessions[1]["name"].as_str(), Some(format!("fm-{w}-alpha").as_str()), "other sessions keep");
            assert_eq!(sessions[1]["role"].as_str(), Some("repo"));
            assert_eq!(sessions[1]["repo"].as_str(), Some("Alpha"));
            assert_eq!(sessions[1]["permission_mode"].as_str(), Some("default"));
        }
        assert!(!home.path().join("supervisor").join("nomos.toml.tmp").exists());

        let before = snapshot(home.path());
        assert_eq!(migrate_home(home.path()).unwrap(), Vec::<String>::new());
        assert_eq!(snapshot(home.path()), before, "the second run changes nothing");
    }

    #[test]
    fn both_registry_dirs_are_left_alone_with_a_notice() {
        let home = old_home(&[("nomos", false)]);
        fs::create_dir_all(home.path().join("workspaces")).unwrap();
        fs::write(home.path().join("workspaces").join("other.toml"), "name = \"other\"\n").unwrap();
        let notices = migrate_home(home.path()).unwrap();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].starts_with("both "), "{}", notices[0]);
        assert!(notices[0].contains("platforms") && notices[0].contains("workspaces"), "{}", notices[0]);
        assert!(home.path().join("platforms").join("nomos.toml").is_file());
        assert!(!home.path().join("workspaces").join("nomos.toml").exists(), "no merge");
        assert!(home.path().join("workspaces").join("other.toml").is_file());
        assert_eq!(migrate_home(home.path()).unwrap(), notices, "repeats until the human resolves it");
    }

    #[test]
    fn an_empty_or_current_home_is_untouched() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(migrate_home(home.path()).unwrap(), Vec::<String>::new());
        assert!(fs::read_dir(home.path()).unwrap().next().is_none(), "nothing created");

        let sup = home.path().join("supervisor");
        fs::create_dir_all(&sup).unwrap();
        let current = "workspace = \"demo\"\nstarted = true\n\n[[sessions]]\nname = \"fm-demo-workspace\"\n\
                       role = \"workspace\"\ncwd = 'C:\\demo'\nprompt = \"p\"\npermission_mode = \"auto\"\n";
        fs::write(sup.join("demo.toml"), current).unwrap();
        fs::write(sup.join("broken.toml"), "started = ").unwrap();
        let notices = migrate_home(home.path()).unwrap();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("broken.toml"), "{}", notices[0]);
        assert_eq!(fs::read_to_string(sup.join("demo.toml")).unwrap(), current, "started stays");
        assert_eq!(fs::read_to_string(sup.join("broken.toml")).unwrap(), "started = ");
    }
}

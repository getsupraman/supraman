use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub const HOME_VAR: &str = "SUPRAMAN_HOME";
/// Read when [`HOME_VAR`] is unset; sessions and scripts started by an older build carry only this one.
pub const LEGACY_HOME_VAR: &str = "FOREMAN_HOME";
pub const HOME_DIR: &str = "Supraman";
pub const LEGACY_HOME_DIR: &str = "Foreman";
/// Written into a home copied from [`LEGACY_HOME_DIR`]: where it came from and when.
pub const MIGRATED_FILE: &str = "migrated-from.toml";
/// The copy's notice until the app has shown it ([`take_migration_notice`]).
const NOTICE_FILE: &str = "migration-notice.txt";

pub const MIGRATION_NOTICE: &str = r"Your Foreman settings were copied to %APPDATA%\Supraman; the old folder is kept.";

fn non_empty(v: Option<OsString>) -> Option<OsString> {
    v.filter(|v| !v.is_empty())
}

/// `SUPRAMAN_HOME`, else `FOREMAN_HOME`, else `%APPDATA%\Supraman`; an empty value counts as unset.
pub fn home_from(supraman_home: Option<OsString>, foreman_home: Option<OsString>, appdata: Option<OsString>) -> Option<PathBuf> {
    non_empty(supraman_home).or_else(|| non_empty(foreman_home)).map(PathBuf::from).or_else(|| default_home(appdata))
}

pub fn default_home(appdata: Option<OsString>) -> Option<PathBuf> {
    non_empty(appdata).map(|a| PathBuf::from(a).join(HOME_DIR))
}

/// `%APPDATA%\Foreman`, the home a first start copies from.
pub fn legacy_default_home(appdata: Option<OsString>) -> Option<PathBuf> {
    non_empty(appdata).map(|a| PathBuf::from(a).join(LEGACY_HOME_DIR))
}

/// The home an env var names, without the `%APPDATA%` default: a hook's or a session's.
pub fn env_home() -> Option<PathBuf> {
    non_empty(std::env::var_os(HOME_VAR)).or_else(|| non_empty(std::env::var_os(LEGACY_HOME_VAR))).map(PathBuf::from)
}

pub fn home_from_env() -> Option<PathBuf> {
    home_from(std::env::var_os(HOME_VAR), std::env::var_os(LEGACY_HOME_VAR), std::env::var_os("APPDATA"))
}

/// Both names, for a session's or a script's environment: an older `fm` reads only the legacy one.
pub fn home_env(home: &Path) -> [(&'static str, String); 2] {
    let home = home.display().to_string();
    [(HOME_VAR, home.clone()), (LEGACY_HOME_VAR, home)]
}

/// The default home and the legacy one it would be copied from, while that copy is still to make.
pub fn pending_migration() -> Option<(PathBuf, PathBuf)> {
    if env_home().is_some() {
        return None;
    }
    let (new, old) = (default_home(std::env::var_os("APPDATA"))?, legacy_default_home(std::env::var_os("APPDATA"))?);
    (!new.exists() && old.is_dir()).then_some((old, new))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    pub from: PathBuf,
    pub to: PathBuf,
    pub files: usize,
}

/// [`migrate_home`] from `%APPDATA%\Foreman` to `%APPDATA%\Supraman`; nothing when an env var names the home.
pub fn migrate_default_home() -> io::Result<Option<Migrated>> {
    match pending_migration() {
        Some((old, new)) => migrate_home(&old, &new),
        None => Ok(None),
    }
}

/// Copies the whole of `old` to `new` when `new` does not exist yet and `old` does: into `<new>.migrating`, which
/// is then renamed to `new`, so `new` appears whole or not at all. `old` is only read. Two starts at once copy
/// once: the second waits for the first's lock and then finds `new`.
pub fn migrate_home(old: &Path, new: &Path) -> io::Result<Option<Migrated>> {
    if new.exists() || !old.is_dir() {
        return Ok(None);
    }
    let _lock = crate::fs::lock_file(&lock_path(new))?;
    if new.exists() {
        return Ok(None);
    }
    let mut staging = new.as_os_str().to_owned();
    staging.push(".migrating");
    let staging = PathBuf::from(staging);
    match fs::remove_dir_all(&staging) {
        Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let copied = copy_tree(old, &staging).and_then(|files| {
        let marker = format!("from = {}\nat_ms = {}\n", toml_string(&old.display().to_string()), crate::time::now_ms());
        crate::fs::write_atomic(&staging.join(MIGRATED_FILE), marker)?;
        crate::fs::write_atomic(&staging.join(NOTICE_FILE), MIGRATION_NOTICE)?;
        fs::rename(&staging, new)?;
        Ok(files)
    });
    match copied {
        Ok(files) => Ok(Some(Migrated { from: old.to_path_buf(), to: new.to_path_buf(), files })),
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// The migration's notice, once: the next call finds nothing.
pub fn take_migration_notice(home: &Path) -> Option<String> {
    let path = home.join(NOTICE_FILE);
    let text = fs::read_to_string(&path).ok()?;
    fs::remove_file(&path).ok()?;
    Some(text)
}

/// In the temp folder, so neither `%APPDATA%` nor the old home gets a file of the copy's.
fn lock_path(new: &Path) -> PathBuf {
    let digest = Sha256::digest(crate::paths::path_key(new).as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    std::env::temp_dir().join(format!("supraman-migrate-{hex}.lock"))
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Every file below `from`, but a `*.tmp` (half of another writer's `write_atomic`); a `*.lock` arrives empty,
/// since its holder's byte-range lock may refuse the read and a lock file holds nothing.
fn copy_tree(from: &Path, to: &Path) -> io::Result<usize> {
    fs::create_dir_all(to)?;
    let mut files = 0;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        let ext = src.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        if kind.is_dir() {
            files += copy_tree(&src, &dst)?;
        } else if ext.as_deref() == Some("tmp") {
        } else if ext.as_deref() == Some("lock") {
            fs::write(&dst, b"")?;
            files += 1;
        } else if kind.is_file() {
            fs::copy(&src, &dst)?;
            files += 1;
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supraman_home_wins_then_foreman_home_then_appdata() {
        let appdata = || Some(OsString::from(r"C:\Users\x\AppData\Roaming"));
        let (s, f) = (|| Some(OsString::from(r"D:\s")), || Some(OsString::from(r"D:\f")));
        assert_eq!(home_from(s(), f(), appdata()), Some(PathBuf::from(r"D:\s")));
        assert_eq!(home_from(None, f(), appdata()), Some(PathBuf::from(r"D:\f")), "the legacy name is still read");
        assert_eq!(home_from(Some(OsString::new()), f(), appdata()), Some(PathBuf::from(r"D:\f")), "empty counts as unset");
        assert_eq!(home_from(None, Some(OsString::new()), appdata()), Some(PathBuf::from(r"C:\Users\x\AppData\Roaming\Supraman")));
        assert_eq!(home_from(None, None, None), None);
        assert_eq!(legacy_default_home(appdata()), Some(PathBuf::from(r"C:\Users\x\AppData\Roaming\Foreman")));
    }

    /// A home laid out like a real `%APPDATA%\Foreman`, each kind of state the app and `fm` keep.
    const LAYOUT: &[(&str, &str)] = &[
        ("settings.toml", "[appearance]\ntheme = \"Foreman Dark\"\n"),
        ("workspaces/autowork.toml", "name = \"autowork\"\n"),
        ("workspaces/gaia.toml", "name = \"gaia\"\n"),
        ("supervisor/autowork.json", "{\"wanted\":[]}"),
        ("supervisor/refresh.lock", ""),
        ("minted/fm-autowork-workspace.json", "{\"origin\":\"https://workstate.example\"}"),
        ("mcp/workstate-fm-autowork-workspace.json", "{}"),
        ("oauth/servers.json", "{\"https://workstate.example\":{}}"),
        ("oauth/secrets/refresh-https-workstate.example.bin", "\u{1}\u{2}dpapi-blob"),
        ("stars.json", "[\"fm-autowork-workspace\"]"),
        ("dismissed.json", "{}"),
        ("tabs.json", "[]"),
        ("schedules/autowork.toml", "[[schedule]]\nid = \"nightly\"\n"),
        ("schedules/runs/autowork/nightly.jsonl", "{\"outcome\":\"ok\"}\n"),
        ("schedules/state/autowork/nightly/note.txt", "x"),
        ("events/fm-autowork-workspace.jsonl", "{}\n"),
        ("relay/autowork/outbox/m-1.json", "{}"),
        ("logs/foreman.log", "started\n"),
        ("logs/foreman.log.1", "older\n"),
    ];

    fn lay_out(home: &Path) {
        for (rel, text) in LAYOUT {
            let path = home.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    fn snapshot(home: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    out.push((p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(home, home, &mut out);
        out.sort();
        out
    }

    #[test]
    fn a_first_start_copies_every_kind_of_state_and_keeps_the_old_home() {
        let appdata = tempfile::tempdir().unwrap();
        let (old, new) = (appdata.path().join("Foreman"), appdata.path().join("Supraman"));
        lay_out(&old);
        fs::write(old.join("settings.toml.123-0.tmp"), "half").unwrap();
        let before = snapshot(&old);

        let migrated = migrate_home(&old, &new).unwrap().expect("the first start copies");
        assert_eq!((migrated.from.as_path(), migrated.to.as_path(), migrated.files), (old.as_path(), new.as_path(), LAYOUT.len()));
        assert_eq!(snapshot(&old), before, "the old home is only read");
        let copied: Vec<_> = snapshot(&new).into_iter().filter(|(rel, _)| rel != MIGRATED_FILE && rel != NOTICE_FILE).collect();
        let mut want: Vec<_> = LAYOUT.iter().map(|(rel, text)| (rel.to_string(), text.as_bytes().to_vec())).collect();
        want.sort();
        assert_eq!(copied, want, "every file arrives byte for byte, the half-written temp file does not");
        let marker = fs::read_to_string(new.join(MIGRATED_FILE)).unwrap();
        assert!(marker.contains(&toml_string(&old.display().to_string())), "{marker}");
        assert!(!appdata.path().join("Supraman.migrating").exists(), "the staging folder became the home");

        assert_eq!(take_migration_notice(&new).as_deref(), Some(MIGRATION_NOTICE));
        assert_eq!(take_migration_notice(&new), None, "shown once");

        fs::write(old.join("stars.json"), "[\"changed later\"]").unwrap();
        assert_eq!(migrate_home(&old, &new).unwrap(), None, "a second start never copies again");
        assert_eq!(fs::read_to_string(new.join("stars.json")).unwrap(), "[\"fm-autowork-workspace\"]");
    }

    #[test]
    fn nothing_to_copy_without_an_old_home() {
        let appdata = tempfile::tempdir().unwrap();
        let new = appdata.path().join("Supraman");
        assert_eq!(migrate_home(&appdata.path().join("Foreman"), &new).unwrap(), None);
        assert!(!new.exists());
    }

    #[test]
    fn a_leftover_staging_folder_from_a_crashed_copy_is_replaced() {
        let appdata = tempfile::tempdir().unwrap();
        let (old, new) = (appdata.path().join("Foreman"), appdata.path().join("Supraman"));
        lay_out(&old);
        fs::create_dir_all(appdata.path().join("Supraman.migrating/workspaces")).unwrap();
        fs::write(appdata.path().join("Supraman.migrating/workspaces/stale.toml"), "x").unwrap();
        migrate_home(&old, &new).unwrap().unwrap();
        assert!(!new.join("workspaces/stale.toml").exists());
        assert!(new.join("workspaces/autowork.toml").is_file());
    }

    #[test]
    fn concurrent_first_starts_copy_once() {
        let appdata = tempfile::tempdir().unwrap();
        let (old, new) = (appdata.path().join("Foreman"), appdata.path().join("Supraman"));
        lay_out(&old);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let starts: Vec<_> = (0..8)
            .map(|_| {
                let (old, new, barrier) = (old.clone(), new.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    migrate_home(&old, &new).unwrap()
                })
            })
            .collect();
        let copies = starts.into_iter().filter_map(|s| s.join().unwrap()).count();
        assert_eq!(copies, 1);
        assert_eq!(snapshot(&new).len(), LAYOUT.len() + 2);
    }
}

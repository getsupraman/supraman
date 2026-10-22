use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::{CoreError, Platform, Result, validate_name};

/// One TOML file per platform at `<home>/platforms/<name>.toml`.
#[derive(Debug, Clone)]
pub struct Registry {
    home: PathBuf,
}

/// `FOREMAN_HOME` when set and non-empty, else `%APPDATA%\Foreman`.
pub fn home_from(foreman_home: Option<OsString>, appdata: Option<OsString>) -> Option<PathBuf> {
    let non_empty = |v: Option<OsString>| v.filter(|v| !v.is_empty());
    non_empty(foreman_home)
        .map(PathBuf::from)
        .or_else(|| non_empty(appdata).map(|a| PathBuf::from(a).join("Foreman")))
}

impl Registry {
    pub fn at(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn from_env() -> Result<Self> {
        home_from(std::env::var_os("FOREMAN_HOME"), std::env::var_os("APPDATA"))
            .map(Self::at)
            .ok_or(CoreError::NoHome)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn platforms_dir(&self) -> PathBuf {
        self.home.join("platforms")
    }

    pub fn path_of(&self, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        Ok(self.platforms_dir().join(format!("{name}.toml")))
    }

    pub fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.path_of(name)?.is_file())
    }

    pub fn load(&self, name: &str) -> Result<Platform> {
        let path = self.path_of(name)?;
        let text = fs::read_to_string(&path).map_err(|e| match e.kind() {
            ErrorKind::NotFound => CoreError::NotFound(name.to_string()),
            _ => CoreError::io(&path, e),
        })?;
        toml::from_str(&text).map_err(|source| CoreError::Parse { path, source })
    }

    /// Sorted by name; an absent registry directory is an empty list, and is not created.
    pub fn list(&self) -> Result<Vec<Platform>> {
        let dir = self.platforms_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(CoreError::io(&dir, e)),
        };
        let mut names = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| CoreError::io(&dir, e))?.path();
            if path.extension().is_some_and(|x| x == "toml")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                && validate_name(stem).is_ok()
            {
                names.push(stem.to_string());
            }
        }
        names.sort();
        names.iter().map(|n| self.load(n)).collect()
    }

    pub fn save(&self, platform: &Platform, force: bool) -> Result<PathBuf> {
        let path = self.path_of(&platform.name)?;
        if !force && path.exists() {
            return Err(CoreError::AlreadyExists(platform.name.clone()));
        }
        let text = toml::to_string(platform).map_err(|source| CoreError::Serialize {
            name: platform.name.clone(),
            source,
        })?;
        let dir = self.platforms_dir();
        fs::create_dir_all(&dir).map_err(|e| CoreError::io(&dir, e))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(|e| CoreError::io(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| CoreError::io(&path, e))?;
        Ok(path)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let path = self.path_of(name)?;
        fs::remove_file(&path).map_err(|e| match e.kind() {
            ErrorKind::NotFound => CoreError::NotFound(name.to_string()),
            _ => CoreError::io(&path, e),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Repo;

    fn sample(name: &str) -> Platform {
        Platform::new(
            name,
            r"P:\Projects\Magic\Nomos",
            vec![
                Repo {
                    name: "Nomos".into(),
                    path: r"P:\Projects\Magic\Nomos\Nomos".into(),
                    has_claude_md: true,
                    has_agents_md: false,
                    branch: Some("main".into()),
                    enabled: true,
                },
                Repo {
                    name: "Nomos.Pdf".into(),
                    path: r"P:\Projects\Magic\Nomos\Nomos.Pdf".into(),
                    has_claude_md: false,
                    has_agents_md: true,
                    branch: None,
                    enabled: false,
                },
            ],
        )
    }

    #[test]
    fn round_trip() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        let p = sample("nomos");
        let path = reg.save(&p, false).unwrap();
        assert_eq!(path, home.path().join("platforms").join("nomos.toml"));
        assert_eq!(reg.load("nomos").unwrap(), p);
        assert_eq!(reg.list().unwrap(), vec![p]);
        assert!(!home.path().join("platforms").join("nomos.toml.tmp").exists());
    }

    #[test]
    fn duplicate_name_needs_force() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        reg.save(&sample("nomos"), false).unwrap();
        let mut changed = sample("nomos");
        changed.repos.truncate(1);
        assert!(matches!(
            reg.save(&changed, false),
            Err(CoreError::AlreadyExists(n)) if n == "nomos"
        ));
        assert_eq!(reg.load("nomos").unwrap().repos.len(), 2);
        reg.save(&changed, true).unwrap();
        assert_eq!(reg.load("nomos").unwrap().repos.len(), 1);
    }

    #[test]
    fn list_is_sorted_and_remove_works() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        assert!(reg.list().unwrap().is_empty());
        assert!(!reg.platforms_dir().exists(), "list must not create the registry");
        reg.save(&sample("zeta"), false).unwrap();
        reg.save(&sample("alpha"), false).unwrap();
        let names: Vec<_> = reg.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["alpha", "zeta"]);
        reg.remove("zeta").unwrap();
        assert!(matches!(reg.remove("zeta"), Err(CoreError::NotFound(_))));
        assert!(matches!(reg.load("zeta"), Err(CoreError::NotFound(_))));
        assert!(reg.exists("alpha").unwrap());
    }

    #[test]
    fn invalid_names_are_rejected_before_touching_disk() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        assert!(matches!(reg.save(&sample("Bad.Name"), false), Err(CoreError::InvalidName(_))));
        assert!(matches!(reg.load("../x"), Err(CoreError::InvalidName(_))));
        assert!(!reg.platforms_dir().exists());
    }

    #[test]
    fn corrupt_file_is_a_parse_error() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        fs::create_dir_all(reg.platforms_dir()).unwrap();
        fs::write(reg.platforms_dir().join("broken.toml"), "name = ").unwrap();
        let err = reg.load("broken").unwrap_err();
        assert!(matches!(err, CoreError::Parse { .. }));
        assert!(!err.is_user_error());
    }

    #[test]
    fn foreman_home_overrides_appdata() {
        let appdata = Some(OsString::from(r"C:\Users\x\AppData\Roaming"));
        assert_eq!(
            home_from(Some(OsString::from(r"D:\fm")), appdata.clone()),
            Some(PathBuf::from(r"D:\fm"))
        );
        assert_eq!(
            home_from(Some(OsString::new()), appdata.clone()),
            Some(PathBuf::from(r"C:\Users\x\AppData\Roaming").join("Foreman"))
        );
        assert_eq!(
            home_from(None, appdata),
            Some(PathBuf::from(r"C:\Users\x\AppData\Roaming").join("Foreman"))
        );
        assert_eq!(home_from(None, None), None);
    }
}

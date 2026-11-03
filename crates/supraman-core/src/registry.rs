use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::{CoreError, Workspace, Result, validate_name};

/// One TOML file per workspace at `<home>/workspaces/<name>.toml`.
#[derive(Debug, Clone)]
pub struct Registry {
    home: PathBuf,
}

impl Registry {
    pub fn at(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn from_env() -> Result<Self> {
        crate::home::home_from_env().map(Self::at).ok_or(CoreError::NoHome)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn workspaces_dir(&self) -> PathBuf {
        self.home.join("workspaces")
    }

    pub fn path_of(&self, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        Ok(self.workspaces_dir().join(format!("{name}.toml")))
    }

    pub fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.path_of(name)?.is_file())
    }

    pub fn load(&self, name: &str) -> Result<Workspace> {
        let path = self.path_of(name)?;
        let text = fs::read_to_string(&path).map_err(|e| match e.kind() {
            ErrorKind::NotFound => CoreError::NotFound(name.to_string()),
            _ => CoreError::io(&path, e),
        })?;
        toml::from_str(&text).map_err(|source| CoreError::Parse { path, source })
    }

    /// Sorted by name; an absent registry directory is an empty list, and is not created. The first
    /// file that cannot be loaded fails the whole list; [`Self::load_all`] keeps the others.
    pub fn list(&self) -> Result<Vec<Workspace>> {
        self.names()?.iter().map(|n| self.load(n)).collect()
    }

    /// Every workspace that loads, sorted by name, and an error for each file that does not.
    pub fn load_all(&self) -> Result<(Vec<Workspace>, Vec<CoreError>)> {
        let (mut loaded, mut failed) = (Vec::new(), Vec::new());
        for name in self.names()? {
            match self.load(&name) {
                Ok(w) => loaded.push(w),
                Err(e) => failed.push(e),
            }
        }
        Ok((loaded, failed))
    }

    fn names(&self) -> Result<Vec<String>> {
        let dir = self.workspaces_dir();
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
        Ok(names)
    }

    pub fn save(&self, workspace: &Workspace, force: bool) -> Result<PathBuf> {
        let path = self.path_of(&workspace.name)?;
        if !force && path.exists() {
            return Err(CoreError::AlreadyExists(workspace.name.clone()));
        }
        let text = toml::to_string(workspace).map_err(|source| CoreError::Serialize {
            name: workspace.name.clone(),
            source,
        })?;
        crate::fs::write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))?;
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
    use crate::Folder;

    fn sample(name: &str) -> Workspace {
        Workspace::new(
            name,
            r"C:\work\demo",
            vec![
                Folder { has_claude_md: true, branch: Some("main".into()), ..Folder::new("Nomos", r"C:\work\demo\Nomos") },
                Folder { has_agents_md: true, enabled: false, ..Folder::new("Nomos.Pdf", r"C:\work\demo\Nomos.Pdf") },
            ],
        )
    }

    #[test]
    fn round_trip() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        let p = sample("nomos");
        let path = reg.save(&p, false).unwrap();
        assert_eq!(path, home.path().join("workspaces").join("nomos.toml"));
        assert_eq!(reg.load("nomos").unwrap(), p);
        assert_eq!(reg.list().unwrap(), vec![p]);
        assert!(!home.path().join("workspaces").join("nomos.toml.tmp").exists());
    }

    #[test]
    fn duplicate_name_needs_force() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        reg.save(&sample("nomos"), false).unwrap();
        let mut changed = sample("nomos");
        changed.folders.truncate(1);
        assert!(matches!(
            reg.save(&changed, false),
            Err(CoreError::AlreadyExists(n)) if n == "nomos"
        ));
        assert_eq!(reg.load("nomos").unwrap().folders.len(), 2);
        reg.save(&changed, true).unwrap();
        assert_eq!(reg.load("nomos").unwrap().folders.len(), 1);
    }

    #[test]
    fn list_is_sorted_and_remove_works() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        assert!(reg.list().unwrap().is_empty());
        assert!(!reg.workspaces_dir().exists(), "list must not create the registry");
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
        assert!(!reg.workspaces_dir().exists());
    }

    #[test]
    fn corrupt_file_is_a_parse_error() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        fs::create_dir_all(reg.workspaces_dir()).unwrap();
        fs::write(reg.workspaces_dir().join("broken.toml"), "name = ").unwrap();
        let err = reg.load("broken").unwrap_err();
        assert!(matches!(err, CoreError::Parse { .. }));
        assert!(!err.is_user_error());
    }

    #[test]
    fn one_corrupt_file_fails_list_but_load_all_keeps_the_others() {
        let home = tempfile::tempdir().unwrap();
        let reg = Registry::at(home.path());
        assert_eq!(reg.load_all().unwrap().0, vec![]);
        reg.save(&sample("alpha"), false).unwrap();
        reg.save(&sample("zeta"), false).unwrap();
        fs::write(reg.workspaces_dir().join("broken.toml"), "name = ").unwrap();
        assert!(reg.list().is_err());
        let (loaded, failed) = reg.load_all().unwrap();
        assert_eq!(loaded.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["alpha", "zeta"]);
        assert_eq!(failed.len(), 1);
        assert!(matches!(&failed[0], CoreError::Parse { path, .. } if path.ends_with("broken.toml")), "{failed:?}");
    }
}

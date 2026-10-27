//! Starred sessions (m5/27): always shown in the tree, stopped or closed, however old, until unstarred.
//! Persisted as `<FOREMAN_HOME>/starred.json`, `{"sessions": ["<sessionId>", …]}`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    sessions: BTreeSet<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Stars {
    path: Option<PathBuf>,
    sessions: BTreeSet<String>,
}

impl Stars {
    pub fn path(home: &Path) -> PathBuf {
        home.join("starred.json")
    }

    /// An unreadable file starts empty, with the reason.
    pub fn load(home: Option<&Path>) -> (Self, Option<String>) {
        let Some(path) = home.map(Self::path) else {
            return (Self::default(), None);
        };
        let (sessions, notice) = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(f) => (f.sessions, None),
                Err(e) => (BTreeSet::new(), Some(format!("Cannot read {} ({e}): no session is starred", path.display()))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeSet::new(), None),
            Err(e) => (BTreeSet::new(), Some(format!("Cannot read {} ({e}): no session is starred", path.display()))),
        };
        (Self { path: Some(path), sessions }, notice)
    }

    pub fn contains(&self, session_id: &str) -> bool {
        self.sessions.contains(session_id)
    }

    pub fn ids(&self) -> &BTreeSet<String> {
        &self.sessions
    }

    /// Stars or unstars `session_id` and writes the file; `Err` says why it could not be written.
    pub fn toggle(&mut self, session_id: &str) -> Result<bool, String> {
        let on = if self.sessions.remove(session_id) {
            false
        } else {
            self.sessions.insert(session_id.to_string());
            true
        };
        self.save().map(|_| on)
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(&File { sessions: self.sessions.clone() }).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_survives_a_reload_and_unstar_removes_it() {
        let home = tempfile::tempdir().unwrap();
        let (mut stars, notice) = Stars::load(Some(home.path()));
        assert!(notice.is_none() && stars.ids().is_empty());
        assert_eq!(stars.toggle("aaaa-1"), Ok(true));
        assert_eq!(stars.toggle("bbbb-2"), Ok(true));
        let (again, _) = Stars::load(Some(home.path()));
        assert!(again.contains("aaaa-1") && again.contains("bbbb-2"), "read back from starred.json");
        let mut again = again;
        assert_eq!(again.toggle("aaaa-1"), Ok(false));
        let (third, _) = Stars::load(Some(home.path()));
        assert!(!third.contains("aaaa-1") && third.contains("bbbb-2"));
    }

    #[test]
    fn an_unreadable_file_starts_empty_and_says_so() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(Stars::path(home.path()), "not json").unwrap();
        let (stars, notice) = Stars::load(Some(home.path()));
        assert!(stars.ids().is_empty());
        assert!(notice.unwrap().contains("starred.json"));
    }
}

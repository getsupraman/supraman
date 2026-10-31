use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use supraman_core::fs::{load_json_or_notice, write_atomic};
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

    pub fn load(home: Option<&Path>) -> (Self, Option<String>) {
        let Some(path) = home.map(Self::path) else {
            return (Self::default(), None);
        };
        let (file, notice) = load_json_or_notice::<File>(&path, "no session is starred");
        (Self { path: Some(path), sessions: file.sessions }, notice)
    }

    pub fn ids(&self) -> &BTreeSet<String> {
        &self.sessions
    }

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
        write_atomic(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
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
        assert!(again.ids().contains("aaaa-1") && again.ids().contains("bbbb-2"), "read back from starred.json");
        let mut again = again;
        assert_eq!(again.toggle("aaaa-1"), Ok(false));
        let (third, _) = Stars::load(Some(home.path()));
        assert!(!third.ids().contains("aaaa-1") && third.ids().contains("bbbb-2"));
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

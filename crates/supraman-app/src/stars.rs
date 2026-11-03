use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use supraman_core::store::{JsonStore, Save};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    sessions: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct Stars(JsonStore<File>);

impl Stars {
    pub fn path(home: &Path) -> PathBuf {
        home.join("starred.json")
    }

    pub fn load(home: Option<&Path>) -> (Self, Option<String>) {
        let (store, notice) = JsonStore::load(home.map(Self::path), "no session is starred");
        (Self(store), notice)
    }

    pub fn ids(&self) -> &BTreeSet<String> {
        &self.0.get().sessions
    }

    /// Whether `session_id` is starred now, and the save of the change.
    pub fn toggle(&mut self, session_id: &str) -> (bool, Option<Save>) {
        let mut on = false;
        let save = self.0.edit(|f| {
            on = !f.sessions.remove(session_id);
            if on {
                f.sessions.insert(session_id.to_string());
            }
            true
        });
        (on, save)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggled(stars: &mut Stars, session_id: &str) -> bool {
        let (on, save) = stars.toggle(session_id);
        save.expect("a home").write().unwrap();
        on
    }

    #[test]
    fn a_star_survives_a_reload_and_unstar_removes_it() {
        let home = tempfile::tempdir().unwrap();
        let (mut stars, notice) = Stars::load(Some(home.path()));
        assert!(notice.is_none() && stars.ids().is_empty());
        assert!(toggled(&mut stars, "aaaa-1"));
        assert!(toggled(&mut stars, "bbbb-2"));
        let (mut again, _) = Stars::load(Some(home.path()));
        assert!(again.ids().contains("aaaa-1") && again.ids().contains("bbbb-2"), "read back from starred.json");
        assert!(!toggled(&mut again, "aaaa-1"));
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

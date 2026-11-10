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

    /// The stars made before this store was read join the file's.
    pub fn absorb(&mut self, early: &Stars) -> Option<Save> {
        self.0.edit(|f| {
            let before = f.sessions.len();
            f.sessions.extend(early.ids().iter().cloned());
            f.sessions.len() != before
        })
    }

    /// Each starred session that continued (`continued`: to the newest of its chain) is starred under the newest instead.
    pub fn follow(&mut self, continued: &std::collections::HashMap<String, String>) -> Option<Save> {
        self.0.edit(|f| {
            let moved: Vec<(String, String)> =
                f.sessions.iter().filter_map(|sid| continued.get(sid).filter(|newest| *newest != sid).map(|newest| (sid.clone(), newest.clone()))).collect();
            for (sid, newest) in &moved {
                f.sessions.remove(sid);
                f.sessions.insert(newest.clone());
            }
            !moved.is_empty()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_moves_to_the_newest_session_of_its_chain_and_survives_a_reload() {
        let home = tempfile::tempdir().unwrap();
        let (mut stars, _) = Stars::load(Some(home.path()));
        assert!(toggled(&mut stars, "old-1"));
        assert!(toggled(&mut stars, "kept-2"));
        let continued = std::collections::HashMap::from([("old-1".to_string(), "new-3".to_string()), ("gone-4".to_string(), "new-5".to_string())]);
        stars.follow(&continued).expect("a change").write().unwrap();
        assert_eq!(stars.ids().iter().map(String::as_str).collect::<Vec<_>>(), ["kept-2", "new-3"]);
        assert!(stars.follow(&continued).is_none(), "nothing left to move");
        let (again, _) = Stars::load(Some(home.path()));
        assert_eq!(again.ids(), stars.ids());
    }

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
    fn a_star_made_before_the_file_is_read_is_kept_with_the_files() {
        let home = tempfile::tempdir().unwrap();
        let (mut stars, _) = Stars::load(Some(home.path()));
        assert!(toggled(&mut stars, "aaaa-1"));
        let mut early = Stars::default();
        let (on, save) = early.toggle("bbbb-2");
        assert!(on && save.is_none(), "no file yet, nothing written");
        let (mut read, _) = Stars::load(Some(home.path()));
        read.absorb(&early).expect("a change").write().unwrap();
        assert!(read.absorb(&early).is_none(), "nothing new the second time");
        let (again, _) = Stars::load(Some(home.path()));
        assert_eq!(again.ids().iter().map(String::as_str).collect::<Vec<_>>(), ["aaaa-1", "bbbb-2"]);
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

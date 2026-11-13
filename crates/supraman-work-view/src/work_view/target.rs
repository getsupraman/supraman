//! What a Work tab shows: a folder's bound projects or a group's folders'; one tab per target.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use supraman_core::paths::{folder_name, path_key};

/// A folder is the same target by its `path_key`; a group by its name; the two never collide.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Folder(PathBuf),
    Group(String),
}

impl Target {
    fn id(&self) -> (bool, String) {
        match self {
            Target::Folder(path) => (false, path_key(path)),
            Target::Group(name) => (true, name.clone()),
        }
    }

    /// The folder's or group's name as the user reads it.
    pub fn name(&self) -> String {
        match self {
            Target::Folder(path) => folder_name(path),
            Target::Group(name) => name.clone(),
        }
    }

    pub fn folder(&self) -> Option<&Path> {
        match self {
            Target::Folder(path) => Some(path),
            Target::Group(_) => None,
        }
    }

    /// A hit key part: unique per target.
    pub fn key(&self) -> String {
        match self.id() {
            (false, key) => format!("folder-{key}"),
            (true, name) => format!("group-{name}"),
        }
    }
}

impl PartialEq for Target {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Target {}

impl Hash for Target {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

/// `Work · <name>`: the project's when the target binds exactly one, else the folder's or group's.
pub fn title(target: &Target, projects: &[&str]) -> String {
    match projects {
        [one] if !one.is_empty() => format!("Work · {one}"),
        _ => format!("Work · {}", target.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_and_a_group_of_one_name_are_two_targets_and_a_folder_is_one_by_its_key() {
        let folder = Target::Folder(PathBuf::from(r"C:\Code\shop"));
        assert_ne!(folder, Target::Group("shop".into()));
        assert_eq!(folder, Target::Folder(PathBuf::from("c:/code/shop/")));
        assert_eq!(folder.name(), "shop");
        assert_ne!(folder.key(), Target::Group("shop".into()).key());
    }

    #[test]
    fn the_title_is_the_one_bound_projects_name_else_the_targets() {
        let group = Target::Group("shop".into());
        assert_eq!(title(&group, &["Supraman"]), "Work · Supraman");
        assert_eq!(title(&group, &["a", "b"]), "Work · shop");
        assert_eq!(title(&group, &[]), "Work · shop");
    }

    #[test]
    fn a_target_round_trips_through_json() {
        for target in [Target::Folder(r"C:\Code\shop".into()), Target::Group("shop".into())] {
            let text = serde_json::to_string(&target).unwrap();
            assert_eq!(serde_json::from_str::<Target>(&text).unwrap(), target);
        }
        assert_eq!(serde_json::to_string(&Target::Group("g".into())).unwrap(), r#"{"group":"g"}"#);
    }
}

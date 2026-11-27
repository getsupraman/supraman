use std::collections::HashSet;

use super::unlisted;
use crate::fleet::Tree;

/// A group row's key in the sidebar's rows and its `collapsed` set.
pub(in crate::view) fn group(name: &str) -> String {
    format!("group:{name}")
}

/// A folder row's key (by [`crate::fleet::FolderNode::key`]) in the sidebar's rows and its `collapsed` set.
pub(in crate::view) fn folder(key: &str) -> String {
    format!("repo:{key}")
}

/// What Collapse all folds: every group, every folder row, Other sessions.
pub(super) fn all(tree: &Tree) -> HashSet<String> {
    let groups = tree.entries.iter().filter(|e| e.group).map(|e| group(&e.name));
    let folders = tree.folders().map(|(_, f)| folder(&f.key));
    groups.chain(folders).chain([unlisted::KEY.to_string()]).collect()
}

/// What Expand all leaves folded: Other sessions, which can be long.
pub(super) fn expanded() -> HashSet<String> {
    unlisted::first_collapsed()
}

#[cfg(feature = "selftest")]
impl crate::view::FleetView {
    /// How many session rows of the entry `name` the sidebar holds, in view or not.
    pub fn session_rows(&self, name: &str) -> usize {
        let (tree, rows, _) = &*self.sidebar.left_rows;
        rows.iter().filter(|r| matches!(r, super::LeftRow::Session(e, ..) if tree.entries.get(*e).is_some_and(|e| e.name == name))).count()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::fleet::{EntryNode, FolderNode};
    use supraman_core::FolderKind;

    fn node(key: &str, parent: Option<&str>) -> FolderNode {
        FolderNode { parent: parent.map(Into::into), ..FolderNode::bare(key.into(), key.into(), PathBuf::from(key), FolderKind::Plain, None) }
    }

    #[test]
    fn collapse_all_folds_groups_folders_shown_children_and_other_sessions() {
        let tree = Tree {
            entries: vec![
                EntryNode { name: "g".into(), group: true, folders: vec![node("g/a", None), node("g/a/c", Some("g/a"))] },
                EntryNode { name: "top".into(), group: false, folders: vec![node("top", None)] },
            ],
            ..Tree::default()
        };
        let all = all(&tree);
        for key in [group("g"), folder("g/a"), folder("g/a/c"), folder("top"), unlisted::KEY.to_string()] {
            assert!(all.contains(&key), "{key} folded");
        }
        assert!(!all.contains(&group("top")), "a top-level folder is no group");
    }

    #[test]
    fn expand_all_keeps_other_sessions_folded() {
        assert_eq!(expanded(), HashSet::from([unlisted::KEY.to_string()]));
        assert_eq!(unlisted::KEY, folder(crate::fleet::UNLISTED));
    }
}

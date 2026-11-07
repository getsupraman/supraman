//! GitMan's background `git maintenance run` (docs/research/gitkraken-12.6.1.md §7): the commit-graph, the loose
//! objects packed and the packs repacked, never while a git holds an index.

use std::path::Path;

use super::{Call, Session, WriteError};
use crate::{Dirs, Version};

/// The first git whose `geometric-repack` task replaces `incremental-repack`.
pub const GEOMETRIC: Version = Version { major: 2, minor: 54, patch: 0 };

/// The tasks run on `version`.
pub fn tasks(version: Version) -> [&'static str; 3] {
    let repack = if version >= GEOMETRIC { "--task=geometric-repack" } else { "--task=incremental-repack" };
    ["--task=commit-graph", "--task=loose-objects", repack]
}

/// The repack is left out while the repository has no pack: git refuses it then, before the loose objects make one.
pub fn run(s: &Session) -> Result<(), WriteError> {
    let mut args = vec!["maintenance", "run"];
    args.extend(tasks(s.writer().repo().git().version));
    if !has_pack(&s.writer().dirs().common_dir) {
        args.pop();
    }
    s.output(&Call::new(&args)).map(drop)
}

fn has_pack(common_dir: &Path) -> bool {
    std::fs::read_dir(common_dir.join("objects").join("pack")).is_ok_and(|entries| entries.flatten().any(|e| e.path().extension().is_some_and(|x| x == "pack")))
}

/// Whether a git holds the index of the repository's main checkout or of one of its worktrees.
pub fn index_locked(dirs: &Dirs) -> bool {
    let held = |dir: &Path| dir.join("index.lock").exists();
    if held(&dirs.git_dir) || held(&dirs.common_dir) {
        return true;
    }
    std::fs::read_dir(dirs.common_dir.join("worktrees")).is_ok_and(|entries| entries.flatten().any(|e| held(&e.path())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;
    use crate::write::{LockSet, Writer};

    #[test]
    fn geometric_repack_from_2_54_on() {
        let v = |minor| Version { major: 2, minor, patch: 0 };
        assert_eq!(tasks(v(53))[2], "--task=incremental-repack");
        assert_eq!(tasks(v(54))[2], "--task=geometric-repack");
        assert_eq!(tasks(Version { major: 3, minor: 0, patch: 0 })[2], "--task=geometric-repack");
    }

    #[test]
    fn a_run_writes_the_commit_graph_and_an_index_lock_anywhere_in_the_repo_is_seen() {
        let f = Fixture::copy();
        let writer = Writer::open(&f.repo()).unwrap();
        let graph = writer.dirs().common_dir.join("objects").join("info").join("commit-graph");
        let _ = std::fs::remove_file(&graph);
        writer.run(LockSet::maintenance(), run).unwrap();
        assert!(graph.is_file() || writer.dirs().common_dir.join("objects/info/commit-graphs").is_dir(), "the commit-graph task ran");
        assert!(has_pack(&writer.dirs().common_dir), "the loose objects packed");
        writer.run(LockSet::maintenance(), run).expect("a second run repacks");

        assert!(!index_locked(writer.dirs()));
        let side = Writer::open(&crate::Repo::new(f.git.clone(), &f.worktree)).unwrap();
        let lock = side.dirs().git_dir.join("index.lock");
        std::fs::write(&lock, b"").unwrap();
        assert!(index_locked(writer.dirs()), "the worktree's index is held");
        std::fs::remove_file(&lock).unwrap();
        std::fs::write(writer.dirs().git_dir.join("index.lock"), b"").unwrap();
        assert!(index_locked(writer.dirs()));
    }
}

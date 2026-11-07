use std::collections::HashSet;
use std::path::PathBuf;

use super::{Call, Resource, Session, WriteError};
use crate::parse::{nul_records, text};

/// Who a snapshot commit says made it.
const SNAPSHOT_AUTHOR: (&str, &str) = ("GitMan", "gitman@supraman.invalid");

/// A copy of the index in the git dir, removed when it drops.
pub(super) struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

impl Session<'_> {
    /// A dangling commit (parents HEAD and a commit of the index, message `what`) of the index with the working
    /// files of `paths` (tracked, untracked or deleted) added, built in a copy of the index: the real index is not
    /// touched. Its sha. The session holds at least the index's read lock.
    pub fn snapshot(&self, paths: &[&str], what: &str) -> Result<String, WriteError> {
        assert!(self.held().locks().access(Resource::Index).is_some(), "a snapshot reads the index under its lock");
        let temp = self.temp_index("snapshot");
        let index = self.path_in_git_dir("index");
        if index.exists() {
            std::fs::copy(&index, &temp.0).map_err(|source| WriteError::Io { what: format!("copy {}", index.display()), source })?;
        }
        let in_temp = |call: Call| temp.on(call);
        let head = text(&self.run(&Call::new(&["rev-parse", "--verify", "-q", "HEAD"]))?.stdout).trim().to_string();
        let head: Vec<&str> = [head.as_str()].into_iter().filter(|h| !h.is_empty()).collect();
        let index_tree = text(&self.output(&in_temp(Call::new(&["write-tree"])))?).trim().to_string();
        let index = self.commit_tree(&index_tree, &head, &format!("Index {what}"))?;
        let indexed: HashSet<String> = nul_records(&self.output(&in_temp(Call::new(&["ls-files", "-z"])))?).map(text).collect();
        let top = &self.writer().dirs().top;
        let present: Vec<&str> = paths.iter().copied().filter(|p| indexed.contains(*p) || top.join(p).symlink_metadata().is_ok()).collect();
        if !present.is_empty() {
            self.output(&in_temp(Call::new(&["add", "-A"]).paths(present)))?;
        }
        let tree = text(&self.output(&in_temp(Call::new(&["write-tree"])))?).trim().to_string();
        let parents: Vec<&str> = head.iter().copied().chain([index.as_str()]).collect();
        self.commit_tree(&tree, &parents, what)
    }

    /// A commit of `tree` with `parents` and `message` by [`SNAPSHOT_AUTHOR`]; its sha.
    pub(super) fn commit_tree(&self, tree: &str, parents: &[&str], message: &str) -> Result<String, WriteError> {
        let (name, email) = SNAPSHOT_AUTHOR;
        let call = commit_tree_call(tree, parents, message).env("GIT_AUTHOR_NAME", name).env("GIT_AUTHOR_EMAIL", email).env("GIT_COMMITTER_NAME", name).env("GIT_COMMITTER_EMAIL", email);
        Ok(text(&self.output(&call)?).trim().to_string())
    }

    /// A copy of the index in the git dir under `name`, not made yet; removed when it drops.
    pub(super) fn temp_index(&self, name: &str) -> TempIndex {
        TempIndex(self.path_in_git_dir(&format!("gitman-{name}-{}.index", supraman_core::time::hex6())))
    }
}

/// `commit-tree` of `tree` with `parents` and `message`, as the user.
pub(super) fn commit_tree_call(tree: &str, parents: &[&str], message: &str) -> Call {
    let mut args = vec!["commit-tree", tree, "-m", message];
    parents.iter().for_each(|p| args.extend(["-p", p]));
    Call::new(&args)
}

impl TempIndex {
    /// `call` on this index in place of the real one.
    pub(super) fn on(&self, call: Call) -> Call {
        call.env("GIT_INDEX_FILE", self.0.as_os_str())
    }
}

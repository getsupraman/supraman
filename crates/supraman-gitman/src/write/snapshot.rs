use std::collections::HashSet;
use std::path::PathBuf;

use super::{Call, Resource, Session, WriteError};
use crate::parse::{nul_records, text};

/// Who a snapshot commit says made it.
const SNAPSHOT_AUTHOR: (&str, &str) = ("GitMan", "gitman@supraman.invalid");

/// A copy of the index in the git dir, removed when it drops.
struct TempIndex(PathBuf);

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
        let temp = TempIndex(self.path_in_git_dir(&format!("gitman-snapshot-{}.index", supraman_core::time::hex6())));
        let index = self.path_in_git_dir("index");
        if index.exists() {
            std::fs::copy(&index, &temp.0).map_err(|source| WriteError::Io { what: format!("copy {}", index.display()), source })?;
        }
        let in_temp = |call: Call| call.env("GIT_INDEX_FILE", temp.0.as_os_str());
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
        let mut args = vec!["commit-tree", tree, "-m", message];
        parents.iter().for_each(|p| args.extend(["-p", p]));
        let (name, email) = SNAPSHOT_AUTHOR;
        let call = Call::new(&args).env("GIT_AUTHOR_NAME", name).env("GIT_AUTHOR_EMAIL", email).env("GIT_COMMITTER_NAME", name).env("GIT_COMMITTER_EMAIL", email);
        Ok(text(&self.output(&call)?).trim().to_string())
    }
}

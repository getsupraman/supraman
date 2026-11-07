use std::path::PathBuf;

use super::{Call, Resource, Session, WriteError};
use crate::parse::text;

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
    /// A dangling commit (parent HEAD, message `what`) of the index with the working files of `paths` (tracked,
    /// untracked or deleted) added, built in a copy of the index: the real index is not touched. Its sha.
    /// The session holds at least the index's read lock.
    pub fn snapshot(&self, paths: &[&str], what: &str) -> Result<String, WriteError> {
        assert!(self.held().locks().access(Resource::Index).is_some(), "a snapshot reads the index under its lock");
        let temp = TempIndex(self.path_in_git_dir(&format!("gitman-snapshot-{}.index", supraman_core::time::hex6())));
        let index = self.path_in_git_dir("index");
        if index.exists() {
            std::fs::copy(&index, &temp.0).map_err(|source| WriteError::Io { what: format!("copy {}", index.display()), source })?;
        }
        let in_temp = |call: Call| call.env("GIT_INDEX_FILE", temp.0.as_os_str());
        if !paths.is_empty() {
            self.output(&in_temp(Call::new(&["add", "-A"]).paths(paths.iter().copied())))?;
        }
        let tree = text(&self.output(&in_temp(Call::new(&["write-tree"])))?).trim().to_string();
        let head = self.run(&Call::new(&["rev-parse", "--verify", "-q", "HEAD"]))?;
        let mut args = vec!["commit-tree", tree.as_str(), "-m", what];
        let head = text(&head.stdout).trim().to_string();
        if !head.is_empty() {
            args.extend(["-p", head.as_str()]);
        }
        let (name, email) = SNAPSHOT_AUTHOR;
        let call = Call::new(&args).env("GIT_AUTHOR_NAME", name).env("GIT_AUTHOR_EMAIL", email).env("GIT_COMMITTER_NAME", name).env("GIT_COMMITTER_EMAIL", email);
        Ok(text(&self.output(&call)?).trim().to_string())
    }
}

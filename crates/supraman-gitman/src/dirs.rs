use std::collections::HashSet;
use std::path::PathBuf;

use crate::parse::{nul_records, text};
use crate::{GitError, Repo};

/// Where a checkout's files are: its top, its own git dir (HEAD, index) and the common one (refs, objects);
/// the two git dirs are one outside a linked worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub top: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

pub fn dirs(repo: &Repo) -> Result<Dirs, GitError> {
    let args = ["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-dir", "--git-common-dir"];
    repo.read(&args, None, |out| match text(out).lines().collect::<Vec<_>>().as_slice() {
        [top, git_dir, common_dir] => Ok(Dirs { top: top.into(), git_dir: git_dir.into(), common_dir: common_dir.into() }),
        lines => Err(format!("{} lines, not 3", lines.len())),
    })
}

/// Which of `paths` (from the repo's folder, `/`-separated) git ignores, in one call; a tracked file never is.
pub fn ignored(repo: &Repo, paths: &[String]) -> Result<HashSet<String>, GitError> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let args = ["check-ignore", "--stdin", "-z"];
    let input = paths.iter().flat_map(|p| p.bytes().chain([0])).collect();
    let out = repo.run_paths(&args, Some(input))?;
    match out.code {
        Some(0) | Some(1) => Ok(nul_records(&out.stdout).map(text).collect()),
        code => Err(GitError::Failed { args: args.join(" "), code, stderr: text(&out.stderr).trim().to_string() }),
    }
}

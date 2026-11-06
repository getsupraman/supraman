use std::path::PathBuf;

use crate::parse::text;
use crate::{GitError, Repo};

/// How far git looks for a NUL to call a file binary (`buffer_is_binary`).
const BINARY_PROBE: usize = 8000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub bytes: Vec<u8>,
    pub binary: bool,
}

/// The bytes of `path` at `rev` as stored, without filters or textconv.
pub fn blob(repo: &Repo, rev: &str, path: &str) -> Result<Blob, GitError> {
    let object = format!("{rev}:{path}");
    let bytes = repo.output(&["cat-file", "blob", "--end-of-options", &object], None)?;
    let binary = is_binary(&bytes);
    Ok(Blob { bytes, binary })
}

/// The bytes of `path` as staged in the index (stage 0), without filters.
pub fn staged_blob(repo: &Repo, path: &str) -> Result<Blob, GitError> {
    let object = format!(":0:{path}");
    let bytes = repo.output(&["cat-file", "blob", "--end-of-options", &object], None)?;
    let binary = is_binary(&bytes);
    Ok(Blob { bytes, binary })
}

/// The bytes of `path` (relative to the top of the worktree, as `status` names it) read from the disk as they are.
pub fn worktree_file(repo: &Repo, path: &str) -> Result<Blob, GitError> {
    let top = match repo.known_dirs() {
        Some(dirs) => dirs.top.clone(),
        None => repo.read(&["rev-parse", "--show-toplevel"], None, |out| Ok(PathBuf::from(text(out).trim())))?,
    };
    let bytes = std::fs::read(top.join(path)).map_err(|source| GitError::Read { path: path.to_string(), source })?;
    let binary = is_binary(&bytes);
    Ok(Blob { bytes, binary })
}

/// Binary as git decides it: a NUL among the first 8000 bytes.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_PROBE)].contains(&0)
}

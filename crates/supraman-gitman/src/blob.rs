use std::io::Read;
use std::path::PathBuf;

use supraman_core::text::{BINARY_PROBE, Kind, classify};

use crate::parse::text;
use crate::{GitError, Repo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub bytes: Vec<u8>,
    pub binary: bool,
}

/// The bytes of `path` at `rev` as stored, without filters or textconv.
pub fn blob(repo: &Repo, rev: &str, path: &str) -> Result<Blob, GitError> {
    object(repo, &format!("{rev}:{path}"), None)
}

/// The bytes of `path` as staged in the index (stage 0), without filters.
pub fn staged_blob(repo: &Repo, path: &str) -> Result<Blob, GitError> {
    object(repo, &format!(":0:{path}"), None)
}

/// [`blob`]'s first [`BINARY_PROBE`] bytes, all [`classify`] reads; git stops once they came.
pub fn blob_head(repo: &Repo, rev: &str, path: &str) -> Result<Blob, GitError> {
    object(repo, &format!("{rev}:{path}"), Some(BINARY_PROBE))
}

/// [`staged_blob`]'s first [`BINARY_PROBE`] bytes.
pub fn staged_blob_head(repo: &Repo, path: &str) -> Result<Blob, GitError> {
    object(repo, &format!(":0:{path}"), Some(BINARY_PROBE))
}

fn object(repo: &Repo, object: &str, limit: Option<usize>) -> Result<Blob, GitError> {
    let args = ["cat-file", "blob", "--end-of-options", object];
    let bytes = match limit {
        Some(limit) => repo.output_head(&args, limit)?,
        None => repo.output(&args, None)?,
    };
    let binary = is_binary(&bytes);
    Ok(Blob { bytes, binary })
}

fn top(repo: &Repo) -> Result<PathBuf, GitError> {
    match repo.known_dirs() {
        Some(dirs) => Ok(dirs.top.clone()),
        None => repo.read(&["rev-parse", "--show-toplevel"], None, |out| Ok(PathBuf::from(text(out).trim()))),
    }
}

/// The bytes of `path` (relative to the top of the worktree, as `status` names it) read from the disk as they are.
pub fn worktree_file(repo: &Repo, path: &str) -> Result<Blob, GitError> {
    let bytes = std::fs::read(top(repo)?.join(path)).map_err(|source| GitError::Read { path: path.to_string(), source })?;
    let binary = is_binary(&bytes);
    Ok(Blob { bytes, binary })
}

/// [`worktree_file`]'s first [`BINARY_PROBE`] bytes.
pub fn worktree_head(repo: &Repo, path: &str) -> Result<Blob, GitError> {
    let read = |source| GitError::Read { path: path.to_string(), source };
    let mut bytes = Vec::new();
    std::fs::File::open(top(repo)?.join(path)).map_err(read)?.take(BINARY_PROBE as u64).read_to_end(&mut bytes).map_err(read)?;
    let binary = is_binary(&bytes);
    Ok(Blob { bytes, binary })
}

/// The size in bytes of the worktree file [`worktree_file`] reads, `None` when there is none.
pub fn worktree_size(repo: &Repo, path: &str) -> Result<Option<u64>, GitError> {
    Ok(std::fs::metadata(top(repo)?.join(path)).ok().map(|m| m.len()))
}

/// Each object's size in bytes (`rev:path`, `:0:path`) in one `cat-file --batch-check`, `None` for one git cannot find.
pub fn object_sizes(repo: &Repo, objects: &[&str]) -> Result<Vec<Option<u64>>, GitError> {
    if objects.iter().any(|o| o.contains(['\n', '\r'])) {
        return objects.iter().map(|o| Ok(repo.output(&["cat-file", "-s", "--end-of-options", o], None).ok().and_then(|out| text(&out).trim().parse().ok()))).collect();
    }
    if objects.is_empty() {
        return Ok(Vec::new());
    }
    let input = objects.iter().flat_map(|o| [o.as_bytes(), b"\n"]).flatten().copied().collect();
    repo.read(&["cat-file", "--batch-check=%(objectsize)"], Some(input), |out| {
        let sizes: Vec<Option<u64>> = text(out).lines().map(|line| line.trim().parse().ok()).collect();
        if sizes.len() == objects.len() { Ok(sizes) } else { Err(format!("{} answers for {} objects", sizes.len(), objects.len())) }
    })
}

fn is_binary(bytes: &[u8]) -> bool {
    classify(bytes) == Kind::Binary
}

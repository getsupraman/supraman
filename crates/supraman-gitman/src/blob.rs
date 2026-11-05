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

/// Binary as git decides it: a NUL among the first 8000 bytes.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_PROBE)].contains(&0)
}

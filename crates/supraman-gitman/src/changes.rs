use crate::parse::{nul_records, text};
use crate::{GitError, Repo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    /// File, symlink or submodule became another of these.
    TypeChanged,
    Unmerged,
    /// In the worktree only, unknown to the index.
    Untracked,
}

impl ChangeKind {
    /// A `--name-status` or porcelain letter; `None` for `.` (unchanged) and letters git does not print here.
    pub(crate) fn from_letter(letter: u8) -> Option<ChangeKind> {
        Some(match letter {
            b'M' => ChangeKind::Modified,
            b'A' => ChangeKind::Added,
            b'D' => ChangeKind::Deleted,
            b'R' => ChangeKind::Renamed,
            b'C' => ChangeKind::Copied,
            b'T' => ChangeKind::TypeChanged,
            b'U' => ChangeKind::Unmerged,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub kind: ChangeKind,
    pub path: String,
    /// The source of a rename or a copy.
    pub old_path: Option<String>,
    /// A rename's or copy's similarity in percent.
    pub similarity: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    pub path: String,
    /// `0o100644`, or `0o100755` for an executable.
    pub mode: u32,
    pub blob: String,
}

/// What `sha` changed: a merge against its first parent, a root commit against the empty tree.
pub fn changed_files(repo: &Repo, sha: &str) -> Result<Vec<FileChange>, GitError> {
    let args = ["diff-tree", "-r", "-z", "--name-status", "-M", "--root", "--diff-merges=first-parent", "--no-commit-id", "--end-of-options", sha];
    repo.read(&args, None, parse_name_status)
}

/// What changed from `from` to `to`.
pub fn changed_between(repo: &Repo, from: &str, to: &str) -> Result<Vec<FileChange>, GitError> {
    repo.read(&["diff-tree", "-r", "-z", "--name-status", "-M", "--end-of-options", from, to], None, parse_name_status)
}

/// Every regular file in the tree of `rev`, from the top whatever folder the repo was opened in.
pub fn tree(repo: &Repo, rev: &str) -> Result<Vec<TreeFile>, GitError> {
    repo.read(&["ls-tree", "-z", "-r", "--full-tree", "--end-of-options", rev], None, parse_tree)
}

/// `--name-status -z`: a status (`R087` with its score), then one path, or two for a rename or a copy.
fn parse_name_status(out: &[u8]) -> Result<Vec<FileChange>, String> {
    let mut records = nul_records(out);
    let mut changes = Vec::new();
    while let Some(status) = records.next() {
        let &[letter, ref score @ ..] = status else { return Err("an empty status".into()) };
        let kind = ChangeKind::from_letter(letter).ok_or_else(|| format!("status {:?}", text(status)))?;
        let mut path = || records.next().map(text).ok_or_else(|| format!("no path after {:?}", text(status)));
        let change = if matches!(kind, ChangeKind::Renamed | ChangeKind::Copied) {
            let old_path = path()?;
            FileChange { kind, path: path()?, old_path: Some(old_path), similarity: text(score).parse().ok() }
        } else {
            FileChange { kind, path: path()?, old_path: None, similarity: None }
        };
        changes.push(change);
    }
    Ok(changes)
}

/// `ls-tree -z`: `<mode> SP <type> SP <id> TAB <path>`; only blobs of mode 100644 and 100755.
fn parse_tree(out: &[u8]) -> Result<Vec<TreeFile>, String> {
    let mut files = Vec::new();
    for record in nul_records(out) {
        let line = text(record);
        let (meta, path) = line.split_once('\t').ok_or_else(|| format!("ls-tree {line:?}"))?;
        let mut meta = meta.split(' ');
        let (Some(mode), Some(kind), Some(blob)) = (meta.next(), meta.next(), meta.next()) else {
            return Err(format!("ls-tree {line:?}"));
        };
        let mode = u32::from_str_radix(mode, 8).map_err(|_| format!("mode {mode:?}"))?;
        if kind == "blob" && matches!(mode, 0o100644 | 0o100755) {
            files.push(TreeFile { path: path.to_string(), mode, blob: blob.to_string() });
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status_and_tree_parse() {
        let changes = parse_name_status(b"M\0a.txt\0R087\0old\0new\0A\0b\0").unwrap();
        assert_eq!(changes[1], FileChange { kind: ChangeKind::Renamed, path: "new".into(), old_path: Some("old".into()), similarity: Some(87) });
        assert_eq!(changes.len(), 3);
        let tree = parse_tree(b"100644 blob aa\ta b.txt\0120000 blob bb\tlink\0160000 commit cc\tsub\0100755 blob dd\trun.sh\0").unwrap();
        assert_eq!(tree.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a b.txt", "run.sh"]);
    }
}

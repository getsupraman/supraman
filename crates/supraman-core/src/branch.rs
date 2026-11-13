use std::fs;
use std::path::Path;

/// The branch checked out at `dir` by the nearest `.git`; `None` when detached or no repo.
pub fn branch_at(dir: &Path) -> Option<String> {
    let git = crate::paths::git_at(dir)?;
    let git_dir = if git.is_dir() { git } else { crate::paths::linked_git_dir(&git)? };
    read_branch(&git_dir)
}

fn read_branch(git_dir: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let reference = head.trim().strip_prefix("ref:")?.trim();
    Some(reference.strip_prefix("refs/heads/").unwrap_or(reference).to_string())
}


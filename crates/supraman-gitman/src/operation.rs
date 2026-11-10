use std::path::Path;

use crate::history::short_sha;
use crate::parse::text;
use crate::{GitError, Repo};

/// The file GitMan writes into `rebase-merge/` or `rebase-apply/` when it starts a rebase itself.
pub const REBASE_MARKER: &str = "gitman-rebase";

/// What a checkout's git dir says is under way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OperationState {
    #[default]
    None,
    Merge,
    /// `external`: started by something other than GitMan (no [`REBASE_MARKER`]).
    Rebase { external: bool },
    Am,
    /// `sequence`: more commits wait in `sequencer/todo`.
    Revert { sequence: bool },
    CherryPick { sequence: bool },
}

/// The operation the files in `git_dir` (a checkout's own, as `Dirs::git_dir`) say is under way; blocking.
pub fn operation_state(git_dir: &Path) -> OperationState {
    let has = |name: &str| git_dir.join(name).exists();
    let rebase = |dir: &str| OperationState::Rebase { external: !git_dir.join(dir).join(REBASE_MARKER).exists() };
    if has("rebase-merge") {
        return rebase("rebase-merge");
    }
    if has("rebase-apply/rebasing") {
        return rebase("rebase-apply");
    }
    if has("rebase-apply") {
        return OperationState::Am;
    }
    if has("MERGE_HEAD") {
        return OperationState::Merge;
    }
    let todo = std::fs::read_to_string(git_dir.join("sequencer").join("todo")).ok();
    let sequence = todo.as_deref().is_some_and(|t| t.lines().any(|l| !l.trim().is_empty() && !l.starts_with('#')));
    if has("REVERT_HEAD") {
        return OperationState::Revert { sequence };
    }
    if has("CHERRY_PICK_HEAD") {
        return OperationState::CherryPick { sequence };
    }
    match todo.as_deref().and_then(|t| t.split_whitespace().next()) {
        Some("revert" | "r") => OperationState::Revert { sequence: true },
        Some("pick" | "p") => OperationState::CherryPick { sequence: true },
        _ => OperationState::None,
    }
}

/// The two sides an operation under way joins: `current` is git's "ours" (stage 2), `incoming` its "theirs" (stage 3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnderWay {
    pub current: String,
    pub incoming: String,
    /// A rebase's or am's commit under way and how many it has.
    pub step: Option<(u32, u32)>,
    /// A merge's `MERGE_MSG` without its comment lines.
    pub message: Option<String>,
}

/// The sides of `state` in the checkout whose git dir is `git_dir` and whose branch is `head`.
pub fn under_way(repo: &Repo, git_dir: &Path, state: OperationState, head: Option<&str>) -> Result<Option<UnderWay>, GitError> {
    let read = |name: &str| std::fs::read_to_string(git_dir.join(name)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let current = || match head {
        Some(branch) => Ok(branch.to_string()),
        None => named(repo, "HEAD"),
    };
    let number = |name: &str| read(name).and_then(|n| n.parse::<u32>().ok());
    let under_way = match state {
        OperationState::None => return Ok(None),
        OperationState::Merge => {
            let incoming = read("MERGE_HEAD").map_or(Ok(String::new()), |sha| named(repo, sha.lines().next().unwrap_or_default()))?;
            let message = read("MERGE_MSG").map(|m| without_comments(&m));
            UnderWay { current: current()?, incoming, step: None, message }
        }
        OperationState::Rebase { .. } => {
            let dir = if git_dir.join("rebase-merge").exists() { "rebase-merge" } else { "rebase-apply" };
            let branch = read(&format!("{dir}/head-name")).map(|h| h.trim_start_matches("refs/heads/").to_string());
            let onto = read(&format!("{dir}/onto")).map_or(Ok(String::new()), |sha| named(repo, &sha))?;
            let (at, of) = if dir == "rebase-merge" { ("msgnum", "end") } else { ("next", "last") };
            let step = number(&format!("{dir}/{at}")).zip(number(&format!("{dir}/{of}")));
            UnderWay { current: onto, incoming: branch.unwrap_or_else(|| "detached HEAD".into()), step, message: None }
        }
        OperationState::Am => {
            let step = number("rebase-apply/next").zip(number("rebase-apply/last"));
            UnderWay { current: current()?, incoming: "the patch".into(), step, message: None }
        }
        OperationState::CherryPick { .. } | OperationState::Revert { .. } => {
            let file = if matches!(state, OperationState::Revert { .. }) { "REVERT_HEAD" } else { "CHERRY_PICK_HEAD" };
            let incoming = read(file).map(|sha| short_sha(&sha).to_string()).unwrap_or_default();
            UnderWay { current: current()?, incoming, step: None, message: None }
        }
    };
    Ok(Some(under_way))
}

/// A local or remote branch at `rev`, else its short sha.
fn named(repo: &Repo, rev: &str) -> Result<String, GitError> {
    let sha = repo.read(&["rev-parse", "-q", "--verify", rev], None, |out| Ok(text(out).trim().to_string())).unwrap_or_default();
    if sha.is_empty() {
        return Ok(rev.to_string());
    }
    let names = repo.read(&["for-each-ref", "--points-at", &sha, "--format=%(refname:short)", "refs/heads", "refs/remotes"], None, |out| Ok(text(out)))?;
    Ok(names.lines().next().map_or_else(|| short_sha(&sha).to_string(), str::to_string))
}

/// `message` without the lines git comments out, the blank ones at its end gone.
fn without_comments(message: &str) -> String {
    let kept: Vec<&str> = message.lines().filter(|l| !l.starts_with('#')).collect();
    kept.join("
").trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sequencer_between_picks_is_still_its_sequence() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(operation_state(dir.path()), OperationState::None);
        std::fs::create_dir(dir.path().join("sequencer")).unwrap();
        std::fs::write(dir.path().join("sequencer/todo"), "pick 0123456 Two\n").unwrap();
        assert_eq!(operation_state(dir.path()), OperationState::CherryPick { sequence: true });
        std::fs::write(dir.path().join("sequencer/todo"), "revert 0123456 Two\n").unwrap();
        assert_eq!(operation_state(dir.path()), OperationState::Revert { sequence: true });
    }

    #[test]
    fn a_merge_message_loses_its_comment_lines() {
        assert_eq!(without_comments("Merge branch 'main' into side

# Conflicts:
#	a.txt
"), "Merge branch 'main' into side");
    }
}

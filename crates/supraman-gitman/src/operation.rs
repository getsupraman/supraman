use std::path::Path;

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
}

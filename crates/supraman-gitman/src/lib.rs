//! GitMan over the user's system git. Every function blocks; callers run it off the UI thread.
//! The readers change no ref, index, worktree, config or reflog (C146); every change goes through [`write`].

mod blob;
mod changes;
mod compose;
mod dirs;
mod history;
mod layout;
mod locate;
mod operation;
mod parse;
mod patch;
mod refs;
mod repo;
mod stash;
mod status;
pub mod write;

#[cfg(feature = "test-support")]
pub mod fixture;

pub use blob::{Blob, blob, is_binary, object_sizes, staged_blob, worktree_file, worktree_size};
pub use changes::{ChangeKind, FileChange, TreeFile, changed_between, changed_files, tree};
pub use compose::{COMMIT_HOOKS, commit_hooks, head_message};
pub use dirs::{Dirs, dirs, ignored};
pub use history::{Commit, Details, FIRST_PAGE, History, NEXT_PAGE, Page, Person, Signature, details, history, short_sha};
pub use layout::{Edge, EdgeKind, LANE_COLOURS, Layout, RefGroup, RefKind, RefLabel, Row};
pub use locate::{FLOOR, Git, GitmanSettings, Located, SECTION, Version, check, judge, locate, located};
pub use operation::{OperationState, REBASE_MARKER, operation_state};
pub use patch::{DiffOf, FilePatch, Hunk, Line, LineKind, patch};
pub use refs::{Branch, Head, Refs, RemoteBranch, Tag, Upstream, Worktree, refs};
pub use repo::{GitError, Repo};
pub use stash::{Stash, stashes};
pub use status::{BranchStatus, Entry, EntryKind, Operation, Status, status, status_first};

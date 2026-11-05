//! GitMan's read layer over the user's system git. Every function blocks; callers run it off the UI thread.
//! Nothing here changes refs, the index, the worktree, the config or the reflog (C146).

mod blob;
mod changes;
mod history;
mod layout;
mod locate;
mod parse;
mod patch;
mod refs;
mod repo;
mod status;

#[cfg(feature = "test-support")]
pub mod fixture;

pub use blob::{Blob, blob, is_binary, staged_blob, worktree_file};
pub use changes::{ChangeKind, FileChange, TreeFile, changed_between, changed_files, tree};
pub use history::{Commit, Details, FIRST_PAGE, NEXT_PAGE, Page, Person, Signature, details, history};
pub use layout::{Edge, EdgeKind, LANE_COLOURS, Layout, RefGroup, RefKind, RefLabel, Row};
pub use locate::{FLOOR, Git, GitmanSettings, Located, SECTION, Version, check, judge, locate};
pub use patch::{DiffOf, FilePatch, Hunk, Line, LineKind, patch};
pub use refs::{Branch, Head, Refs, RemoteBranch, Tag, Upstream, Worktree, refs};
pub use repo::{GitError, Repo};
pub use status::{BranchStatus, Entry, EntryKind, Operation, Status, status};

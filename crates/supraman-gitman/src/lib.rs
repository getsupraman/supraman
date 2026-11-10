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
mod refs;
mod remotes;
mod repo;
mod stash;
mod status;
mod wips;
pub mod write;

#[cfg(feature = "test-support")]
pub mod fixture;

pub use blob::{Blob, blob, object_sizes, staged_blob, worktree_file, worktree_size};
pub use changes::{ChangeKind, FileChange, TreeFile, changed_between, changed_files, tree};
pub use compose::{Amendable, amendable, commit_hooks, head_message, pushed, rewordable};
pub use dirs::{Dirs, dirs, ignored};
pub use history::{Commit, Details, FIRST_PAGE, History, NEXT_PAGE, Page, Person, Signature, details, short_sha};
#[cfg(any(test, feature = "test-support"))]
pub use history::history;
pub use layout::{Edge, EdgeKind, LANE_COLOURS, Layout, RefGroup, RefKind, RefLabel, Row, WorktreeRow};
pub use locate::{FLOOR, Git, GitmanSettings, Located, SECTION, Version, check, judge, locate, located, RemoteSettings, remote_settings, set_setting, BackgroundSettings, background_settings, FETCH_EVERY_MINUTES};
pub use operation::{OperationState, REBASE_MARKER, UnderWay, operation_state, under_way};
pub use refs::{Branch, Head, Refs, RemoteBranch, Tag, Upstream, Worktree, refs, worktrees};
pub use remotes::{Link, Remote, mask_userinfo, remotes, web_link};
pub use repo::{GitError, Repo};
pub use stash::{Stash, stashes};
pub use status::{BranchStatus, Conflict, Entry, EntryKind, Operation, Status, status, status_first};
pub use wips::{WipCounts, WipRead, WorktreeWip, wips};

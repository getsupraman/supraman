use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use supraman_claude::Session;

/// What the tree lists beside the open sessions: the read's errors, the closed sessions and the deletes under way.
#[derive(Default)]
pub struct Listing {
    /// [`crate::fleet::main_repos`] of the sessions, so rendering never walks the filesystem.
    pub(super) main_repos: HashMap<PathBuf, Option<PathBuf>>,
    pub sessions_error: Option<String>,
    pub closed: Vec<Session>,
    /// A selftest's closed sessions, kept across transcript scans.
    pub injected_closed: Vec<Session>,
    pub(super) closed_repos: HashMap<PathBuf, Option<PathBuf>>,
    pub deleted: HashMap<String, i64>,
    pub deleting: HashSet<String>,
}

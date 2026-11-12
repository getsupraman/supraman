use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{Head, Refs};

/// In group order: a group's kind is its best ref's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefKind {
    /// A local branch checked out in another, linked worktree.
    Worktree,
    /// A local branch checked out in the main checkout, seen from a linked worktree.
    Main,
    Local,
    Remote,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefLabel {
    pub kind: RefKind,
    /// `main`, `origin/main`, `v1`.
    pub name: String,
    /// The branch HEAD is on.
    pub head: bool,
}

/// The refs of one commit sharing a short name: one pill, one icon per ref.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefGroup {
    pub name: String,
    pub refs: Vec<RefLabel>,
}

impl RefGroup {
    pub fn head(&self) -> bool {
        self.refs.iter().any(|r| r.head)
    }

    pub fn kind(&self) -> RefKind {
        self.refs.iter().map(|r| r.kind).min().unwrap_or(RefKind::Tag)
    }

    fn branches(&self) -> Option<RefGroup> {
        let refs: Vec<RefLabel> = self.refs.iter().filter(|r| r.kind != RefKind::Tag).cloned().collect();
        (!refs.is_empty()).then(|| RefGroup { name: self.name.clone(), refs })
    }
}

/// Every commit's ref groups in order, and the branch groups it hands to its ancestors as ghost refs.
#[derive(Debug, Default, Clone)]
pub(super) struct RefIndex {
    pub groups: HashMap<String, Arc<[RefGroup]>>,
    pub branches: HashMap<String, Arc<[RefGroup]>>,
    pub head: Option<String>,
    pub detached: Option<String>,
}

impl RefIndex {
    pub fn new(refs: &Refs) -> RefIndex {
        let (head, detached) = match &refs.head {
            Head::Branch { sha, .. } => (sha.clone(), None),
            Head::Detached { sha } => (Some(sha.clone()), Some(sha.clone())),
        };
        let mut by_sha: HashMap<String, HashMap<String, Vec<RefLabel>>> = HashMap::new();
        for (sha, short, label) in labels(refs) {
            by_sha.entry(sha).or_default().entry(short).or_default().push(label);
        }
        let mut index = RefIndex { head, detached, ..RefIndex::default() };
        for (sha, named) in by_sha {
            let mut groups: Vec<RefGroup> = named
                .into_iter()
                .map(|(name, mut refs)| {
                    refs.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
                    RefGroup { name, refs }
                })
                .collect();
            groups.sort_by_cached_key(order_key);
            let branches: Vec<RefGroup> = groups.iter().filter_map(RefGroup::branches).collect();
            if !branches.is_empty() {
                index.branches.insert(sha.clone(), branches.into());
            }
            index.groups.insert(sha, groups.into());
        }
        index
    }

    /// Takes the group of local `branch` out of `sha`'s groups; the ghost refs it hands to ancestors stay.
    pub fn take_branch(&mut self, sha: &str, branch: &str) -> Option<RefGroup> {
        let groups = self.groups.get(sha)?;
        let at = groups.iter().position(|g| g.refs.iter().any(|r| r.name == branch && !matches!(r.kind, RefKind::Remote | RefKind::Tag)))?;
        let mut kept = groups.to_vec();
        let taken = kept.remove(at);
        self.groups.insert(sha.to_string(), kept.into());
        Some(taken)
    }
}

/// HEAD's group first, then by kind, more refs first, then the name ignoring case.
fn order_key(g: &RefGroup) -> (bool, RefKind, Reverse<usize>, String, String) {
    (!g.head(), g.kind(), Reverse(g.refs.len()), g.name.to_lowercase(), g.name.clone())
}

/// Every branch, remote branch and commit tag as `(commit, short name, label)`.
fn labels(refs: &Refs) -> Vec<(String, String, RefLabel)> {
    let head_branch = match &refs.head {
        Head::Branch { name, .. } => Some(name.as_str()),
        Head::Detached { .. } => None,
    };
    let held_by = |name: &str| refs.worktrees.iter().find(|w| w.branch.as_deref() == Some(name)).map(|w| w.main);
    let locals = refs.branches.iter().map(|b| {
        let head = head_branch == Some(b.name.as_str());
        let kind = match held_by(&b.name) {
            Some(false) if !head => RefKind::Worktree,
            Some(true) if !head => RefKind::Main,
            _ => RefKind::Local,
        };
        (b.sha.clone(), b.name.clone(), RefLabel { kind, name: b.name.clone(), head })
    });
    let remotes = refs.remotes.iter().map(|r| {
        (r.sha.clone(), r.branch().to_string(), RefLabel { kind: RefKind::Remote, name: r.name.clone(), head: false })
    });
    let tags = refs.tags.iter().filter_map(|t| {
        let sha = t.commit.clone()?;
        Some((sha, t.name.clone(), RefLabel { kind: RefKind::Tag, name: t.name.clone(), head: false }))
    });
    locals.chain(remotes).chain(tags).collect()
}

//! The ref panel's rows as plain values: each ref of [`Refs`] as an [`Entry`] at a tree path in its
//! [`Section`], the order, the filter's match, the section counts, and the refs the graph walks with a set hidden.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ops::Range;

use supraman_gitman::{Head, Refs};
use supraman_ui::icon::Icon;
use supraman_ui::matcher::{find_all, subsequence};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Local,
    Remote,
    Worktrees,
    Stashes,
    Tags,
}

impl Section {
    pub const ALL: [Section; 5] = [Section::Local, Section::Remote, Section::Worktrees, Section::Stashes, Section::Tags];

    /// The section's folder at the tree's root.
    pub fn key(self) -> &'static str {
        match self {
            Section::Local => "local",
            Section::Remote => "remote",
            Section::Worktrees => "worktrees",
            Section::Stashes => "stashes",
            Section::Tags => "tags",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Section::Local => "LOCAL",
            Section::Remote => "REMOTE",
            Section::Worktrees => "WORKTREES",
            Section::Stashes => "STASHES",
            Section::Tags => "TAGS",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Section::Local => Icon::Local,
            Section::Remote => Icon::Remote,
            Section::Worktrees => Icon::Worktree,
            Section::Stashes => Icon::Stash,
            Section::Tags => Icon::Tag,
        }
    }

    pub fn of(path: &str) -> Option<Section> {
        let root = path.split('/').next()?;
        Section::ALL.into_iter().find(|s| s.key() == root)
    }

    /// Whether its header counts `shown/total`: only sections whose refs hide.
    pub fn hides(self) -> bool {
        matches!(self, Section::Local | Section::Remote | Section::Stashes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Branch { name: String, sha: String, current: bool, ahead: u32, behind: u32 },
    Detached { sha: String },
    /// `origin/main`.
    Remote { name: String, sha: String },
    /// `folder` only where it differs from `label`.
    Worktree { label: String, folder: Option<String>, main: bool, sha: Option<String> },
    Stash { sha: String, index: String, label: String, branch: String },
    Tag { name: String, commit: Option<String> },
}

pub fn branch_key(name: &str) -> String {
    format!("refs/heads/{name}")
}

pub fn remote_key(name: &str) -> String {
    format!("refs/remotes/{name}")
}

pub fn stash_key(sha: &str) -> String {
    format!("stash/{sha}")
}

impl Entry {
    /// What the hidden set holds for it; `None` for a ref that never hides (the current branch, a worktree, a tag).
    pub fn hide_key(&self) -> Option<String> {
        match self {
            Entry::Branch { name, current: false, .. } => Some(branch_key(name)),
            Entry::Remote { name, .. } => Some(remote_key(name)),
            Entry::Stash { sha, .. } => Some(stash_key(sha)),
            _ => None,
        }
    }

    /// The commit a click selects in the graph.
    pub fn commit(&self) -> Option<&str> {
        match self {
            Entry::Branch { sha, .. } | Entry::Detached { sha } | Entry::Remote { sha, .. } => Some(sha),
            Entry::Worktree { sha, .. } => sha.as_deref(),
            Entry::Stash { index, .. } => Some(index),
            Entry::Tag { commit, .. } => commit.as_deref(),
        }
    }

    /// What the filter reads, and where the row's label lies in it.
    fn text(&self) -> (String, Range<usize>) {
        let tail = |name: &str| name.len() - name.rsplit('/').next().unwrap_or(name).len()..name.len();
        match self {
            Entry::Branch { name, .. } | Entry::Remote { name, .. } => (name.clone(), tail(name)),
            Entry::Tag { name, .. } => (name.clone(), tail(name)),
            Entry::Detached { .. } => ("HEAD".into(), 0..4),
            Entry::Worktree { label, folder, .. } => (format!("{label} {}", folder.as_deref().unwrap_or("")), 0..label.len()),
            Entry::Stash { label, branch, .. } => (format!("{label} on: {branch}"), 0..label.len()),
        }
    }

    /// The row's label: a branch's, remote branch's or tag's last name part (its folders are rows above it).
    pub fn label(&self) -> String {
        let (text, span) = self.text();
        text[span].to_string()
    }

    /// The filter's match in `hay`: a worktree's a substring; else the first whole occurrence, or failing one a subsequence.
    fn matched(&self, hay: &str, filter: &str) -> Option<Vec<Range<usize>>> {
        let whole = find_all(hay, filter).into_iter().next().map(|r| vec![r]);
        match self {
            Entry::Worktree { .. } => whole,
            _ => whole.or_else(|| subsequence(hay, filter)),
        }
    }

    pub fn matches(&self, filter: &str) -> bool {
        filter.is_empty() || self.matched(&self.text().0, filter).is_some()
    }

    /// The filter's matches inside the label, as byte ranges of [`Entry::label`]: the label's own match when it
    /// holds the whole filter, else the part of the whole text's match that falls on it.
    pub fn marks(&self, filter: &str) -> Vec<Range<usize>> {
        if filter.is_empty() {
            return Vec::new();
        }
        let (text, span) = self.text();
        if let Some(own) = self.matched(&text[span.clone()], filter) {
            return own;
        }
        let Some(found) = self.matched(&text, filter) else { return Vec::new() };
        found.into_iter().filter_map(|r| (r.end > span.start && r.start < span.end).then(|| r.start.max(span.start) - span.start..r.end.min(span.end) - span.start)).collect()
    }
}

/// A–Z ignoring case one name part at a time, so a folder sorts by its own name among the refs beside it.
fn by_parts(a: &str, b: &str) -> Ordering {
    a.to_lowercase().split('/').cmp(b.to_lowercase().split('/')).then_with(|| a.cmp(b))
}

/// Every ref of `refs` at its tree path, in the panel's order: the sections in turn; a detached HEAD first,
/// then the branches; remote branches under their remote; the main worktree first; stashes newest first.
pub fn entries(refs: &Refs) -> Vec<(String, Entry)> {
    let mut out = Vec::new();
    let current = match &refs.head {
        Head::Branch { name, .. } => Some(name.as_str()),
        Head::Detached { sha } => {
            out.push(("local/HEAD".to_string(), Entry::Detached { sha: sha.clone() }));
            None
        }
    };
    let mut branches: Vec<_> = refs.branches.iter().collect();
    branches.sort_by(|a, b| by_parts(&a.name, &b.name));
    out.extend(branches.into_iter().map(|b| {
        let (ahead, behind) = b.upstream.as_ref().filter(|u| !u.gone).map_or((0, 0), |u| (u.ahead, u.behind));
        (format!("local/{}", b.name), Entry::Branch { name: b.name.clone(), sha: b.sha.clone(), current: current == Some(b.name.as_str()), ahead, behind })
    }));
    let mut remotes: Vec<_> = refs.remotes.iter().collect();
    remotes.sort_by(|a, b| by_parts(&a.name, &b.name));
    out.extend(remotes.into_iter().map(|r| (format!("remote/{}", r.name), Entry::Remote { name: r.name.clone(), sha: r.sha.clone() })));
    out.extend(refs.worktrees.iter().enumerate().map(|(i, w)| {
        let folder = w.path.file_name().map_or_else(|| w.path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let label = w.branch.clone().unwrap_or_else(|| folder.clone());
        let folder = (folder != label).then_some(folder);
        (format!("worktrees/{i}"), Entry::Worktree { label, folder, main: i == 0, sha: w.head.clone() })
    }));
    out.extend(refs.stashes.iter().enumerate().map(|(i, s)| {
        (format!("stashes/{i}"), Entry::Stash { sha: s.sha.clone(), index: s.index.clone(), label: s.label(), branch: s.branch.clone() })
    }));
    let mut tags: Vec<_> = refs.tags.iter().collect();
    tags.sort_by(|a, b| by_parts(&a.name, &b.name));
    out.extend(tags.into_iter().map(|t| (format!("tags/{}", t.name), Entry::Tag { name: t.name.clone(), commit: t.commit.clone() })));
    out
}

/// A section's refs the filter keeps, and how many of those are not hidden.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count {
    pub shown: usize,
    pub total: usize,
}

pub fn counts(entries: &[(String, Entry)], hidden: &BTreeSet<String>, filter: &str) -> [Count; 5] {
    let mut counts = [Count::default(); 5];
    for (path, entry) in entries.iter().filter(|(_, e)| e.matches(filter)) {
        let Some(at) = Section::of(path).and_then(|s| Section::ALL.iter().position(|a| *a == s)) else { continue };
        counts[at].total += 1;
        if entry.hide_key().is_none_or(|k| !hidden.contains(&k)) {
            counts[at].shown += 1;
        }
    }
    counts
}

/// `refs` without the hidden ones, the current branch always kept: the graph walks from these tips and labels these refs.
pub fn without(refs: &Refs, hidden: &BTreeSet<String>) -> Refs {
    if hidden.is_empty() {
        return refs.clone();
    }
    let current = match &refs.head {
        Head::Branch { name, .. } => Some(name.clone()),
        Head::Detached { .. } => None,
    };
    let shown = |name: &str| current.as_deref() == Some(name) || !hidden.contains(&branch_key(name));
    let mut out = refs.clone();
    out.branches.retain(|b| shown(&b.name));
    out.remotes.retain(|r| !hidden.contains(&remote_key(&r.name)));
    out.stashes.retain(|s| !hidden.contains(&stash_key(&s.sha)));
    out.worktrees.retain(|w| w.branch.as_deref().is_none_or(shown));
    out
}

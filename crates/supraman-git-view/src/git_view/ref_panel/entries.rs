//! The ref panel's rows as plain values: each ref of [`Refs`] as an [`Entry`] at a tree path in its
//! [`Section`], the order, the filter's match, the section counts, and the refs the graph walks ([`Walk`]).

use std::collections::BTreeSet;
use std::ops::Range;

use supraman_gitman::{Head, Refs};
use supraman_ui::icon::Icon;
use supraman_ui::matcher::{find_all, subsequence};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Local,
    Remote,
    Worktrees,
    Stashes,
    Tags,
}

impl Section {
    pub(crate) const ALL: [Section; 5] = [Section::Local, Section::Remote, Section::Worktrees, Section::Stashes, Section::Tags];

    /// The section's folder at the tree's root.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Section::Local => "local",
            Section::Remote => "remote",
            Section::Worktrees => "worktrees",
            Section::Stashes => "stashes",
            Section::Tags => "tags",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Section::Local => "LOCAL",
            Section::Remote => "REMOTE",
            Section::Worktrees => "WORKTREES",
            Section::Stashes => "STASHES",
            Section::Tags => "TAGS",
        }
    }

    pub(crate) fn icon(self) -> Icon {
        match self {
            Section::Local => Icon::Local,
            Section::Remote => Icon::Remote,
            Section::Worktrees => Icon::Worktree,
            Section::Stashes => Icon::Stash,
            Section::Tags => Icon::Tag,
        }
    }

    pub(crate) fn of(path: &str) -> Option<Section> {
        let root = path.split('/').next()?;
        Section::ALL.into_iter().find(|s| s.key() == root)
    }

    /// Whether its header counts `shown/total`: only sections whose refs hide.
    pub(crate) fn hides(self) -> bool {
        matches!(self, Section::Local | Section::Remote | Section::Stashes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Branch { name: String, sha: String, current: bool, ahead: u32, behind: u32 },
    Detached { sha: String },
    /// `origin/main`.
    Remote { name: String, sha: String },
    /// `folder` only where it differs from `label`.
    Worktree { label: String, folder: Option<String>, main: bool, sha: Option<String> },
    Stash { sha: String, index: String, label: String, branch: String },
    Tag { name: String, commit: Option<String> },
}

pub(crate) fn branch_key(name: &str) -> String {
    format!("refs/heads/{name}")
}

pub(crate) fn remote_key(name: &str) -> String {
    format!("refs/remotes/{name}")
}

pub(crate) fn stash_key(sha: &str) -> String {
    format!("stash/{sha}")
}

impl Entry {
    /// What the hidden set holds for it; `None` for a ref that never hides (the current branch, a worktree, a tag).
    pub(crate) fn hide_key(&self) -> Option<String> {
        match self {
            Entry::Branch { name, current: false, .. } => Some(branch_key(name)),
            Entry::Remote { name, .. } => Some(remote_key(name)),
            Entry::Stash { sha, .. } => Some(stash_key(sha)),
            _ => None,
        }
    }

    /// What the solo set holds for it: any branch, the current one too, and a remote branch.
    pub(crate) fn solo_key(&self) -> Option<String> {
        match self {
            Entry::Branch { name, .. } => Some(branch_key(name)),
            Entry::Remote { name, .. } => Some(remote_key(name)),
            _ => None,
        }
    }

    /// Whether the graph walks from it: a ref that never hides always.
    pub(crate) fn walked(&self, walk: &Walk) -> bool {
        self.hide_key().is_none_or(|k| walk.walks(&k))
    }

    /// Whether the graph walks from it as a tip: [`Entry::walked`], and a tag only while nothing is soloed ([`tips`]).
    pub(crate) fn tip_of(&self, walk: &Walk) -> bool {
        self.walked(walk) && !(matches!(self, Entry::Tag { .. }) && walk.soloing())
    }

    /// The commit a click selects in the graph.
    pub(crate) fn commit(&self) -> Option<&str> {
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
    pub(crate) fn label(&self) -> String {
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

    pub(crate) fn matches(&self, filter: &str) -> bool {
        filter.is_empty() || self.matched(&self.text().0, filter).is_some()
    }

    /// The filter's matches inside the label, as byte ranges of [`Entry::label`]: the label's own match when it
    /// holds the whole filter, else the part of the whole text's match that falls on it.
    pub(crate) fn marks(&self, filter: &str) -> Vec<Range<usize>> {
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

/// A–Z ignoring case one name part at a time, so a folder sorts by its own name among the refs beside it: `/` as
/// the lowest character orders the parts as a list of them would.
fn by_parts(name: &str) -> (String, String) {
    (name.to_lowercase().replace('/', "\0"), name.to_string())
}

/// Every ref of `refs` at its tree path, in the panel's order: the sections in turn; a detached HEAD first,
/// then the branches; remote branches under their remote; the main worktree first; stashes newest first.
pub(crate) fn entries(refs: &Refs) -> Vec<(String, Entry)> {
    let mut out = Vec::new();
    let current = match &refs.head {
        Head::Branch { name, .. } => Some(name.as_str()),
        Head::Detached { sha } => {
            out.push(("local/HEAD".to_string(), Entry::Detached { sha: sha.clone() }));
            None
        }
    };
    let mut branches: Vec<_> = refs.branches.iter().collect();
    branches.sort_by_cached_key(|b| by_parts(&b.name));
    out.extend(branches.into_iter().map(|b| {
        let (ahead, behind) = b.upstream.as_ref().filter(|u| !u.gone).map_or((0, 0), |u| (u.ahead, u.behind));
        (format!("local/{}", b.name), Entry::Branch { name: b.name.clone(), sha: b.sha.clone(), current: current == Some(b.name.as_str()), ahead, behind })
    }));
    let mut remotes: Vec<_> = refs.remotes.iter().collect();
    remotes.sort_by_cached_key(|r| by_parts(&r.name));
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
    tags.sort_by_cached_key(|t| by_parts(&t.name));
    out.extend(tags.into_iter().map(|t| (format!("tags/{}", t.name), Entry::Tag { name: t.name.clone(), commit: t.commit.clone() })));
    out
}

/// The refs hidden from the graph and the refs soloed in it: while any is soloed, the walk is the soloed refs
/// and HEAD alone, the hidden set kept for when the last solo goes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Walk {
    pub(crate) hidden: BTreeSet<String>,
    pub(crate) solo: BTreeSet<String>,
}

impl Walk {
    pub(crate) fn soloing(&self) -> bool {
        !self.solo.is_empty()
    }

    /// Whether the ref of hide or solo key `key` is a tip of the walk.
    pub(crate) fn walks(&self, key: &str) -> bool {
        if self.soloing() { self.solo.contains(key) } else { !self.hidden.contains(key) }
    }
}

/// A section's refs the filter keeps, and how many of those the graph walks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Count {
    pub(crate) shown: usize,
    pub(crate) total: usize,
}

pub(crate) fn counts(entries: &[(String, Entry)], walk: &Walk, filter: &str) -> [Count; 5] {
    let mut counts = [Count::default(); 5];
    for (path, entry) in entries.iter().filter(|(_, e)| e.matches(filter)) {
        let Some(at) = Section::of(path).and_then(|s| Section::ALL.iter().position(|a| *a == s)) else { continue };
        counts[at].total += 1;
        if entry.walked(walk) {
            counts[at].shown += 1;
        }
    }
    counts
}

/// The refs the graph labels: `refs` without those out of the walk, the current branch always kept; while soloing,
/// the stashes and the worktrees on no walked branch go too, and the tags only label (see [`tips`]).
pub(crate) fn without(refs: &Refs, walk: &Walk) -> Refs {
    if *walk == Walk::default() {
        return refs.clone();
    }
    let current = match &refs.head {
        Head::Branch { name, .. } => Some(name.clone()),
        Head::Detached { .. } => None,
    };
    let shown = |name: &str| current.as_deref() == Some(name) || walk.walks(&branch_key(name));
    let mut out = refs.clone();
    out.branches.retain(|b| shown(&b.name));
    out.remotes.retain(|r| walk.walks(&remote_key(&r.name)));
    out.stashes.retain(|s| walk.walks(&stash_key(&s.sha)));
    let soloing = walk.soloing();
    out.worktrees.retain(|w| w.branch.as_deref().map_or(!soloing, shown));
    out
}

/// The commits the graph walks from: `labelled`'s ([`without`]), its tags only while nothing is soloed.
pub(crate) fn tips(labelled: &Refs, walk: &Walk) -> Vec<String> {
    if !walk.soloing() {
        return labelled.tips();
    }
    Refs { tags: Vec::new(), ..labelled.clone() }.tips()
}

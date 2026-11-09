use std::path::PathBuf;

use crate::parse::text;
use crate::{GitError, Repo, Stash};

const REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)%00%(upstream:short)%00%(upstream:track)%00%(symref)%00%(HEAD)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// `sha` is `None` on a branch with no commit yet.
    Branch { name: String, sha: Option<String> },
    Detached { sha: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// `origin/main`.
    pub name: String,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream branch no longer exists.
    pub gone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub sha: String,
    pub upstream: Option<Upstream>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranch {
    /// `origin/main`.
    pub name: String,
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// The tag object of an annotated tag, else what it names.
    pub sha: String,
    pub annotated: bool,
    /// The commit it names, through an annotated tag; `None` for a tag of a tree or a blob.
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// `main` for `refs/heads/main`; `None` when detached or bare.
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
    /// The repository's own checkout, `worktree list`'s first.
    pub main: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refs {
    pub head: Head,
    pub branches: Vec<Branch>,
    /// Without the `<remote>/HEAD` pointers.
    pub remotes: Vec<RemoteBranch>,
    pub tags: Vec<Tag>,
    pub worktrees: Vec<Worktree>,
    /// Newest first.
    pub stashes: Vec<Stash>,
}

impl Refs {
    /// Every commit a ref or a worktree points at, and each stash's index commit, once each: the history's starting points.
    pub fn tips(&self) -> Vec<String> {
        let head = match &self.head {
            Head::Branch { sha, .. } => sha.clone(),
            Head::Detached { sha } => Some(sha.clone()),
        };
        let mut tips: Vec<String> = head
            .into_iter()
            .chain(self.branches.iter().map(|b| b.sha.clone()))
            .chain(self.remotes.iter().map(|r| r.sha.clone()))
            .chain(self.tags.iter().filter_map(|t| t.commit.clone()))
            .chain(self.worktrees.iter().filter_map(|w| w.head.clone()))
            .chain(self.stashes.iter().map(|s| s.index.clone()))
            .collect();
        let mut seen = std::collections::HashSet::new();
        tips.retain(|sha| seen.insert(sha.clone()));
        tips
    }
}

pub fn refs(repo: &Repo) -> Result<Refs, GitError> {
    let (mut refs, current) = repo.read(&["for-each-ref", REF_FORMAT, "refs/heads", "refs/remotes", "refs/tags"], None, parse_refs)?;
    refs.head = match current {
        Some(b) => Head::Branch { name: refs.branches[b].name.clone(), sha: Some(refs.branches[b].sha.clone()) },
        None => head(repo)?,
    };
    refs.worktrees = worktrees(repo)?;
    refs.stashes = crate::stashes(repo)?;
    Ok(refs)
}

/// `worktree list`: the repository's own checkout first.
pub fn worktrees(repo: &Repo) -> Result<Vec<Worktree>, GitError> {
    repo.read(&["worktree", "list", "--porcelain", "-z"], None, |out| Ok(parse_worktrees(out)))
}

/// HEAD on no listed branch: detached, or on a branch with no commit yet.
fn head(repo: &Repo) -> Result<Head, GitError> {
    let sha = repo.run(&["rev-parse", "-q", "--verify", "HEAD^{commit}"], None)?;
    if sha.success() {
        return Ok(Head::Detached { sha: text(&sha.stdout).trim().to_string() });
    }
    let name = repo.run(&["symbolic-ref", "-q", "HEAD"], None)?;
    let name = text(&name.stdout).trim().to_string();
    match name.strip_prefix("refs/heads/") {
        Some(branch) => Ok(Head::Branch { name: branch.to_string(), sha: None }),
        None => Err(GitError::Parse { args: "symbolic-ref HEAD".into(), what: format!("HEAD is neither a branch nor a commit: {name:?}") }),
    }
}

/// The refs, and which branch HEAD is on (`%(HEAD)`), if one is listed.
fn parse_refs(out: &[u8]) -> Result<(Refs, Option<usize>), String> {
    let mut refs = Refs { head: Head::Detached { sha: String::new() }, branches: Vec::new(), remotes: Vec::new(), tags: Vec::new(), worktrees: Vec::new(), stashes: Vec::new() };
    let mut current = None;
    for line in text(out).lines().filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\0').collect();
        let [refname, sha, kind, peeled, peeled_kind, upstream, track, symref, head] = f[..] else {
            return Err(format!("for-each-ref line with {} fields: {line:?}", f.len()));
        };
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            let upstream = (!upstream.is_empty()).then(|| parse_track(upstream, track)).transpose()?;
            if head == "*" {
                current = Some(refs.branches.len());
            }
            refs.branches.push(Branch { name: name.into(), sha: sha.into(), upstream });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            if symref.is_empty() {
                refs.remotes.push(RemoteBranch { name: name.into(), sha: sha.into() });
            }
        } else if let Some(name) = refname.strip_prefix("refs/tags/") {
            let annotated = kind == "tag";
            let commit = match (annotated, kind, peeled_kind) {
                (true, _, "commit") => Some(peeled.to_string()),
                (false, "commit", _) => Some(sha.to_string()),
                _ => None,
            };
            refs.tags.push(Tag { name: name.into(), sha: sha.into(), annotated, commit });
        }
    }
    Ok((refs, current))
}

/// `%(upstream:track)`: empty when even, `[ahead 1]`, `[behind 2]`, `[ahead 1, behind 2]` or `[gone]`.
fn parse_track(name: &str, track: &str) -> Result<Upstream, String> {
    let mut up = Upstream { name: name.into(), ahead: 0, behind: 0, gone: false };
    let inner = track.trim_start_matches('[').trim_end_matches(']');
    for part in inner.split(", ").filter(|p| !p.is_empty()) {
        match part.split_once(' ') {
            Some(("ahead", n)) => up.ahead = n.parse().map_err(|_| format!("track {track:?}"))?,
            Some(("behind", n)) => up.behind = n.parse().map_err(|_| format!("track {track:?}"))?,
            None if part == "gone" => up.gone = true,
            _ => return Err(format!("track {track:?}")),
        }
    }
    Ok(up)
}

/// `worktree list --porcelain -z`: attribute lines, each NUL-terminated, a worktree ending at an empty one.
fn parse_worktrees(out: &[u8]) -> Vec<Worktree> {
    let mut all = Vec::new();
    let mut current: Option<Worktree> = None;
    for record in out.split(|b| *b == 0) {
        let line = text(record);
        let (key, value) = line.split_once(' ').unwrap_or((&line, ""));
        match key {
            "" => all.extend(current.take()),
            "worktree" => current = Some(Worktree { path: PathBuf::from(value), main: all.is_empty() && current.is_none(), ..Worktree::default() }),
            _ => {
                let Some(w) = current.as_mut() else { continue };
                match key {
                    "HEAD" => w.head = Some(value.to_string()),
                    "branch" => w.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_string()),
                    "detached" => w.detached = true,
                    "bare" => w.bare = true,
                    "locked" => w.locked = true,
                    "prunable" => w.prunable = true,
                    _ => {}
                }
            }
        }
    }
    all.extend(current);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_and_worktrees_parse() {
        assert_eq!(parse_track("origin/main", "[ahead 1, behind 2]").unwrap(), Upstream { name: "origin/main".into(), ahead: 1, behind: 2, gone: false });
        assert!(parse_track("origin/x", "[gone]").unwrap().gone);
        assert_eq!(parse_track("origin/main", "").unwrap().ahead, 0);
        let wt = parse_worktrees(b"worktree C:/r\0HEAD abc\0branch refs/heads/main\0\0worktree C:/w\0HEAD def\0detached\0locked\0\0");
        assert_eq!(wt.len(), 2);
        assert_eq!(wt[0].branch.as_deref(), Some("main"));
        assert!(wt[1].detached && wt[1].locked && wt[1].branch.is_none());
    }
}

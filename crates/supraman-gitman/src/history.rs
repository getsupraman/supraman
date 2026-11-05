use crate::parse::{name_email, text};
use crate::{GitError, Repo};

/// The first page's size; each later page adds [`NEXT_PAGE`].
pub const FIRST_PAGE: usize = 2000;
pub const NEXT_PAGE: usize = 200;

const FIELDS: &str = "%P%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct";
const CO_AUTHORS: &str = "%(trailers:key=Co-authored-by,valueonly,unfold,separator=%x1f)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Seconds since the epoch.
    pub time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: Signature,
    pub committer: Signature,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub commits: Vec<Commit>,
    /// Commits follow the last one.
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    pub commit: Commit,
    /// The message's first line.
    pub summary: String,
    /// The rest, without the blank lines around it.
    pub body: String,
    pub co_authors: Vec<Person>,
}

/// `count` commits reachable from `tips` in date order, after the first `skip`.
pub fn history(repo: &Repo, tips: &[String], skip: usize, count: usize) -> Result<Page, GitError> {
    if tips.is_empty() || count == 0 {
        return Ok(Page { commits: Vec::new(), more: false });
    }
    let format = format!("--format=tformat:%x00{FIELDS}%x00%B%x00%x00");
    let (skip, max) = (format!("--skip={skip}"), format!("--max-count={}", count + 1));
    let input = tips.iter().flat_map(|t| [t.as_bytes(), b"\n"]).flatten().copied().collect();
    let args = ["rev-list", &format, "--date-order", "--ignore-missing", &skip, &max, "--stdin"];
    let mut commits = repo.read(&args, Some(input), |out| parse_commits(out, false).map(|v| v.into_iter().map(|(c, _)| c).collect::<Vec<_>>()))?;
    let more = commits.len() > count;
    commits.truncate(count);
    Ok(Page { commits, more })
}

/// One commit, its message split and its `Co-authored-by:` trailers.
pub fn details(repo: &Repo, sha: &str) -> Result<Details, GitError> {
    let format = format!("--format=tformat:%x00{FIELDS}%x00{CO_AUTHORS}%x00%B%x00%x00");
    let args = ["rev-list", &format, "--no-walk", "--end-of-options", sha];
    let mut found = repo.read(&args, None, |out| parse_commits(out, true))?;
    let Some((commit, co_authors)) = found.pop().filter(|_| found.is_empty()) else {
        return Err(GitError::Parse { args: args.join(" "), what: "not exactly one commit".into() });
    };
    let (summary, body) = split_message(&commit.message);
    let co_authors = co_authors
        .split('\x1f')
        .filter(|v| !v.trim().is_empty())
        .map(|v| {
            let (name, email) = name_email(v);
            Person { name, email }
        })
        .collect();
    Ok(Details { commit, summary, body, co_authors })
}

/// `summary` and `body` of a message.
fn split_message(message: &str) -> (String, String) {
    let (first, rest) = message.split_once('\n').unwrap_or((message, ""));
    (first.trim_end().to_string(), rest.trim_matches(|c| c == '\n' || c == '\r').trim_end().to_string())
}

/// `rev-list --format` records: `commit <sha>\n`, then NUL, the fields NUL-separated, the message, and `\0\0\n`.
/// With `trailers`, a field of them precedes the message.
fn parse_commits(out: &[u8], trailers: bool) -> Result<Vec<(Commit, String)>, String> {
    let fields = if trailers { 9 } else { 8 };
    let mut commits = Vec::new();
    let mut rest = out;
    while !rest.is_empty() {
        let end = rest.windows(3).position(|w| w == b"\0\0\n").ok_or("a record without its end")?;
        let record = text(&rest[..end]);
        rest = &rest[end + 3..];
        let (header, body) = record.split_once("\n\0").ok_or_else(|| format!("no header: {record:?}"))?;
        let sha = header.strip_prefix("commit ").ok_or_else(|| format!("header {header:?}"))?;
        let f: Vec<&str> = body.splitn(fields, '\0').collect();
        if f.len() != fields {
            return Err(format!("{} fields for {sha}", f.len()));
        }
        let time = |s: &str| s.parse::<i64>().map_err(|_| format!("time {s:?} of {sha}"));
        let commit = Commit {
            sha: sha.to_string(),
            parents: f[0].split_whitespace().map(String::from).collect(),
            author: Signature { name: f[1].into(), email: f[2].into(), time: time(f[3])? },
            committer: Signature { name: f[4].into(), email: f[5].into(), time: time(f[6])? },
            message: f[fields - 1].to_string(),
        };
        commits.push((commit, if trailers { f[7].to_string() } else { String::new() }));
    }
    Ok(commits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_with_an_empty_message_and_a_merge() {
        let out = b"commit aaa\n\0p1 p2\0Ann\0a@x\x001\0Cy\0c@x\x002\0\0\0\ncommit bbb\n\0\0B\0b@x\x003\0B\0b@x\x004\0subject\n\nbody\n\0\0\n";
        let commits = parse_commits(out, false).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].0.parents, ["p1", "p2"]);
        assert_eq!(commits[0].0.message, "");
        assert!(commits[1].0.parents.is_empty());
        assert_eq!(split_message(&commits[1].0.message), ("subject".into(), "body".into()));
    }
}

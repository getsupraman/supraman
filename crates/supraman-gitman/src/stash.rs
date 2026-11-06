use crate::parse::{nul_records, text};
use crate::{GitError, Repo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// The stash commit: the work tree, its first parent the base, its second the index commit.
    pub sha: String,
    /// The graph's row.
    pub index: String,
    pub base: String,
    /// `(no branch)` for a stash made on a detached HEAD.
    pub branch: String,
    /// `fix` of `On main: fix`, `abc1234 subject` of `WIP on main: abc1234 subject`.
    pub message: String,
    /// Seconds since the epoch.
    pub time: i64,
}

impl Stash {
    /// The message, or `WIP on <branch>` for an empty one.
    pub fn label(&self) -> String {
        if self.message.is_empty() { format!("WIP on {}", self.branch) } else { self.message.clone() }
    }
}

/// Every stash, newest first, from the stash reflog; none when there is no `refs/stash`.
pub fn stashes(repo: &Repo) -> Result<Vec<Stash>, GitError> {
    let args = ["log", "-g", "-z", "--ignore-missing", "--format=%H%x1f%P%x1f%ct%x1f%gs", "refs/stash", "--"];
    repo.read(&args, None, parse_stashes)
}

fn parse_stashes(out: &[u8]) -> Result<Vec<Stash>, String> {
    nul_records(out)
        .map(|record| {
            let record = text(record);
            let f: Vec<&str> = record.trim_start_matches('\n').splitn(4, '\x1f').collect();
            let [sha, parents, time, subject] = f[..] else { return Err(format!("stash record {record:?}")) };
            let parents: Vec<&str> = parents.split_whitespace().collect();
            let [base, index, ..] = parents[..] else { return Err(format!("stash {sha} with parents {parents:?}")) };
            let (branch, message) = parse_subject(subject);
            let time = time.parse().map_err(|_| format!("time {time:?} of stash {sha}"))?;
            Ok(Stash { sha: sha.into(), index: index.into(), base: base.into(), branch, message, time })
        })
        .collect()
}

/// `On <branch>: <message>` or `WIP on <branch>: <sha> <subject>` as the branch and the message.
fn parse_subject(subject: &str) -> (String, String) {
    let rest = subject.strip_prefix("WIP on ").or_else(|| subject.strip_prefix("On ")).unwrap_or(subject);
    match rest.split_once(": ") {
        Some((branch, message)) => (branch.to_string(), message.trim().to_string()),
        None => (rest.trim_end_matches(':').to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_subject_forms_and_an_empty_message() {
        let out = b"s0\x1fb0 i0\x1f100\x1fOn main: fix it\0s1\x1fb1 i1 u1\x1f90\x1fWIP on feature: abc1234 Subject line\0s2\x1fb2 i2\x1f80\x1fOn main: \0";
        let all = parse_stashes(out).unwrap();
        assert_eq!(all[0], Stash { sha: "s0".into(), index: "i0".into(), base: "b0".into(), branch: "main".into(), message: "fix it".into(), time: 100 });
        assert_eq!((all[1].branch.as_str(), all[1].label(), all[1].index.as_str()), ("feature", "abc1234 Subject line".into(), "i1"));
        assert_eq!((all[2].message.as_str(), all[2].label()), ("", "WIP on main".into()));
        assert_eq!(parse_subject("WIP on (no branch): abc s"), ("(no branch)".into(), "abc s".into()));
        assert!(parse_stashes(b"s\x1fb\x1f1\x1fOn main: x\0").is_err(), "a stash has an index commit");
        assert!(parse_stashes(b"").unwrap().is_empty());
    }
}

//! Repositories built in a temp dir with the git CLI, for tests (C151): never a user's repo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use supraman_core::process::{self, Kill};
use tempfile::TempDir;

use crate::{Git, Located, Repo};

const AUTHOR: (&str, &str) = ("Ada Fixture", "ada@example.test");
const FIRST_TIME: i64 = 1_700_000_000;
const TIMEOUT: Duration = Duration::from_secs(300);

/// The system git, or a panic naming why a test cannot run.
pub fn system_git() -> Git {
    let path = supraman_core::paths::executable_on_path("git").expect("git on PATH");
    match crate::check(&path) {
        Located::Found(git) => git,
        other => panic!("no usable git: {other:?}"),
    }
}

/// Runs `git -C dir args` with no user or system config (only the repo's own), `input` on stdin; panics unless it exits 0.
pub fn git_in(git: &Git, dir: &Path, args: &[&str], env: &[(&str, String)], input: Option<Vec<u8>>) -> String {
    let out = run_git(git, dir, args, env, input);
    assert!(out.success(), "git {args:?} in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_git(git: &Git, dir: &Path, args: &[&str], env: &[(&str, String)], input: Option<Vec<u8>>) -> process::RawOutput {
    let mut cmd = process::command(&git.path);
    cmd.arg("-C").arg(dir).args(args);
    cmd.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("GIT_EDITOR", "");
    env.iter().for_each(|(k, v)| {
        cmd.env(k, v);
    });
    process::run_bytes(&mut cmd, input, TIMEOUT, Kill::Tree).expect("git runs")
}

/// One repository with a commit of every kind: a root, a rename with co-authors on `feature`, a `--no-ff` merge,
/// tags (annotated `v1`, light `light`), `origin` (a bare repo beside it) with `main` one ahead and `stale` two
/// behind, a worktree on `side`, and work in progress (staged, unstaged, a staged rename, a deletion,
/// an untracked file, and `bin.dat` rewritten unchanged with an old mtime, so a status that may write refreshes the index).
pub struct Fixture {
    pub git: Git,
    pub temp: TempDir,
    pub dir: PathBuf,
    pub worktree: PathBuf,
    shas: BTreeMap<&'static str, String>,
    time: i64,
}

/// `bin.dat`'s bytes: a NUL early, so git calls it binary.
pub const BINARY: &[u8] = &[0x89, b'P', b'N', b'G', 0, 1, 2, 3, 0xff, 0xfe, b'\n'];

pub const CO_AUTHORED: &str = "Move old.txt to new.txt\n\nThe old name no longer fits.\n\nCo-authored-by: Bea Two <bea@example.test>\nCo-authored-by: Cy Three <cy@example.test>\n";

impl Fixture {
    pub fn new() -> Fixture {
        let git = system_git();
        let temp = tempfile::tempdir().expect("temp dir");
        let dir = temp.path().join("repo");
        let worktree = temp.path().join("wt");
        let mut f = Fixture { git, temp, dir, worktree, shas: BTreeMap::new(), time: FIRST_TIME };
        f.history();
        f.remote_and_worktree();
        f.work_in_progress();
        f
    }

    pub fn repo(&self) -> Repo {
        Repo::new(self.git.clone(), &self.dir)
    }

    /// A commit by its label: `root`, `second`, `rename`, `main_work`, `merge`, `ahead`.
    pub fn sha(&self, label: &str) -> &str {
        self.shas.get(label).unwrap_or_else(|| panic!("no commit {label}"))
    }

    /// `git args` in the repo; may write.
    pub fn git(&self, args: &[&str]) -> String {
        git_in(&self.git, &self.dir, args, &[], None)
    }

    pub fn write(&self, path: &str, bytes: &[u8]) {
        std::fs::write(self.dir.join(path), bytes).unwrap();
    }

    /// The worktree `wt` (on `side`) stops merging `main` on a conflict in `a.txt`.
    pub fn conflict(&self) {
        std::fs::write(self.worktree.join("a.txt"), b"side
").unwrap();
        git_in(&self.git, &self.worktree, &["commit", "-q", "--no-verify", "-am", "Side's a.txt"], &[], None);
        let merge = run_git(&self.git, &self.worktree, &["merge", "-q", "--no-ff", "--no-verify", "--no-edit", "main"], &[], None);
        assert!(!merge.success(), "the merge stops on a conflict");
    }

    fn commit(&mut self, label: &'static str, message: &str) {
        self.time += 60;
        let date = format!("@{} +0000", self.time);
        let env = [("GIT_AUTHOR_DATE", date.clone()), ("GIT_COMMITTER_DATE", date)];
        git_in(&self.git, &self.dir, &["commit", "-q", "--no-verify", "-m", message], &env, None);
        let sha = self.git(&["rev-parse", "HEAD"]);
        self.shas.insert(label, sha);
    }

    fn history(&mut self) {
        let temp = self.temp.path().to_path_buf();
        git_in(&self.git, &temp, &["init", "-q", "-b", "main", "repo"], &[], None);
        for (key, value) in [("user.name", AUTHOR.0), ("user.email", AUTHOR.1), ("core.autocrlf", "false"), ("commit.gpgsign", "false"), ("tag.gpgsign", "false")] {
            self.git(&["config", key, value]);
        }
        self.write("a.txt", b"one\n");
        self.write("old.txt", b"rename me\nline 2\nline 3\nline 4\n");
        self.write("b.txt", b"b\n");
        self.write("bin.dat", BINARY);
        self.git(&["add", "-A"]);
        self.commit("root", "Root");
        self.write("a.txt", b"one\ntwo\n");
        self.git(&["add", "a.txt"]);
        self.commit("second", "Second");
        self.git(&["tag", "-a", "v1", "-m", "Version one"]);
        self.git(&["checkout", "-q", "-b", "feature"]);
        self.git(&["mv", "old.txt", "new.txt"]);
        self.commit("rename", CO_AUTHORED);
        self.git(&["checkout", "-q", "main"]);
        self.write("b.txt", b"b\nb2\n");
        self.git(&["add", "b.txt"]);
        self.commit("main_work", "Main work");
        self.git(&["tag", "light"]);
        self.time += 60;
        let date = format!("@{} +0000", self.time);
        let env = [("GIT_AUTHOR_DATE", date.clone()), ("GIT_COMMITTER_DATE", date)];
        git_in(&self.git, &self.dir, &["merge", "-q", "--no-ff", "--no-verify", "-m", "Merge feature", "feature"], &env, None);
        let merge = self.git(&["rev-parse", "HEAD"]);
        self.shas.insert("merge", merge);
    }

    fn remote_and_worktree(&mut self) {
        let temp = self.temp.path().to_path_buf();
        git_in(&self.git, &temp, &["init", "-q", "--bare", "origin.git"], &[], None);
        let origin = temp.join("origin.git").to_string_lossy().into_owned();
        self.git(&["remote", "add", "origin", &origin]);
        self.git(&["push", "-q", "origin", "main"]);
        self.git(&["branch", "-q", "-u", "origin/main", "main"]);
        self.write("a.txt", b"one\ntwo\nthree\n");
        self.git(&["add", "a.txt"]);
        self.commit("ahead", "Ahead of origin");
        let main_work = self.sha("main_work").to_string();
        self.git(&["branch", "-q", "stale", &main_work]);
        self.git(&["branch", "-q", "-u", "origin/main", "stale"]);
        let worktree = self.worktree.to_string_lossy().into_owned();
        let second = self.sha("second").to_string();
        self.git(&["worktree", "add", "-q", "-b", "side", &worktree, &second]);
    }

    fn work_in_progress(&mut self) {
        self.write("a.txt", b"staged\n");
        self.git(&["add", "a.txt"]);
        self.write("a.txt", b"staged\nunstaged\n");
        self.write("s.txt", b"new\n");
        self.git(&["add", "s.txt"]);
        self.git(&["mv", "new.txt", "renamed.txt"]);
        std::fs::remove_file(self.dir.join("b.txt")).unwrap();
        self.write("u.txt", b"untracked\n");
        self.write("bin.dat", BINARY);
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        std::fs::File::options().write(true).open(self.dir.join("bin.dat")).unwrap().set_modified(old).unwrap();
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Fixture::new()
    }
}

/// A repository of `commits` commits made with `git fast-import`: `main`, and every tenth commit a merge of a
/// one-commit topic branch, every twentieth topic kept as `topic/<n>`; a tag every 1000 commits.
pub struct Generated {
    pub git: Git,
    pub temp: TempDir,
    pub dir: PathBuf,
}

impl Generated {
    pub fn new(commits: usize) -> Generated {
        let git = system_git();
        let temp = tempfile::tempdir().expect("temp dir");
        git_in(&git, temp.path(), &["init", "-q", "-b", "main", "repo"], &[], None);
        let dir = temp.path().join("repo");
        git_in(&git, &dir, &["fast-import", "--quiet"], &[], Some(fast_import_stream(commits).into_bytes()));
        Generated { git, temp, dir }
    }

    pub fn repo(&self) -> Repo {
        Repo::new(self.git.clone(), &self.dir)
    }
}

fn fast_import_stream(commits: usize) -> String {
    let mut s = String::new();
    let mut main: Option<usize> = None;
    let mut mark = 0;
    while mark < commits {
        mark += 1;
        let topic = mark % 10 == 9 && mark < commits && main.is_some();
        if topic {
            let name = if (mark / 10) % 20 == 0 { format!("topic/{}", mark / 10) } else { "scratch".into() };
            push_commit(&mut s, &name, mark, main, None);
            mark += 1;
            push_commit(&mut s, "main", mark, main, Some(mark - 1));
        } else {
            push_commit(&mut s, "main", mark, main, None);
        }
        main = Some(mark);
        if mark % 1000 == 0 {
            s.push_str(&format!("reset refs/tags/t{}\nfrom :{mark}\n\n", mark / 1000));
        }
    }
    s
}

fn push_commit(s: &mut String, branch: &str, mark: usize, from: Option<usize>, merge: Option<usize>) {
    let message = format!("Commit {mark}\n");
    let content = format!("{mark}\n");
    let time = FIRST_TIME + mark as i64;
    s.push_str(&format!("commit refs/heads/{branch}\nmark :{mark}\ncommitter {} <{}> {time} +0000\n", AUTHOR.0, AUTHOR.1));
    s.push_str(&format!("data {}\n{message}", message.len()));
    if let Some(from) = from {
        s.push_str(&format!("from :{from}\n"));
    }
    if let Some(merge) = merge {
        s.push_str(&format!("merge :{merge}\n"));
    }
    s.push_str(&format!("M 644 inline f{}.txt\ndata {}\n{content}\n", mark % 7, content.len()));
}

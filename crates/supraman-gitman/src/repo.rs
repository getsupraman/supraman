use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use supraman_core::process::{self, GIT_TIMEOUT, Kill, RawOutput, RunError, Streaming};

use crate::{Dirs, Git};

/// Before every subcommand (docs/research/gitkraken-12.6.1.md §2): no lock a reader could leave behind, no pager, paths and
/// messages as UTF-8 text, diffs with `a/` `b/` prefixes from the top, and no auto maintenance.
const READ_ONLY: &[&str] = &[
    "--no-pager",
    "--no-optional-locks",
    "--literal-pathspecs",
    "-c", "core.quotePath=false",
    "-c", "color.ui=never",
    "-c", "i18n.logOutputEncoding=UTF-8",
    "-c", "log.showSignature=false",
    "-c", "advice.statusHints=false",
    "-c", "diff.noprefix=false",
    "-c", "diff.mnemonicPrefix=false",
    "-c", "diff.relative=false",
    "-c", "diff.submodule=short",
    "-c", "gc.auto=0",
    "-c", "maintenance.auto=false",
];

/// Inherited from a hook or a shell, these would point git at another repository than `dir`.
const REPO_ENV: &[&str] = &["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR", "GIT_NAMESPACE"];

/// `git -C dir` with no prompt, no editor, a dumb terminal, and none of [`REPO_ENV`]: it acts on `dir` only.
pub(crate) fn git_command(git: &Git, dir: &Path) -> Command {
    let mut cmd = process::command(&git.path);
    cmd.arg("-C").arg(dir);
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("GIT_EDITOR", "").env("TERM", "dumb");
    REPO_ENV.iter().for_each(|var| {
        cmd.env_remove(var);
    });
    cmd
}

/// git's refusal of `args` run with `-C dir`; a `-C` that failed names whether `dir` exists.
pub(crate) fn failed(args: String, dir: &Path, out: &RawOutput) -> GitError {
    match GitError::failed(args, out) {
        GitError::Failed { args, code, stderr } if stderr.contains("cannot change to") => {
            GitError::Failed { args, code, stderr: format!("{stderr} (-C {}, exists: {})", dir.display(), dir.exists()) }
        }
        e => e,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git {args}: {source}")]
    Run { args: String, source: RunError },
    #[error("git {args} exited with {code:?}: {stderr}")]
    Failed { args: String, code: Option<i32>, stderr: String },
    #[error("git {args}: unexpected output: {what}")]
    Parse { args: String, what: String },
    #[error("read {path}: {source}")]
    Read { path: String, source: std::io::Error },
}

impl GitError {
    /// git's refusal of `args`: its exit code and stderr.
    pub fn failed(args: impl Into<String>, out: &RawOutput) -> GitError {
        GitError::Failed { args: args.into(), code: out.code, stderr: String::from_utf8_lossy(&out.stderr).trim().to_string() }
    }

    /// git's refusal of `args` with what it wrote to stderr, then stdout, then `note`, each when not empty.
    pub fn said(args: impl Into<String>, out: &RawOutput, note: Option<String>) -> GitError {
        let said = [&out.stderr, &out.stdout].map(|b| String::from_utf8_lossy(b).trim().to_string());
        let stderr = said.into_iter().chain(note).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n");
        GitError::Failed { args: args.into(), code: out.code, stderr }
    }
}

/// A repository (or a folder inside one) read with one git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    git: Git,
    dir: PathBuf,
    dirs: Option<Arc<Dirs>>,
}

impl Repo {
    pub fn new(git: Git, dir: impl Into<PathBuf>) -> Repo {
        Repo { git, dir: dir.into(), dirs: None }
    }

    /// The repo whose readers take `dirs` (as [`crate::dirs`] read them for `dir`) in place of asking git again.
    pub fn with_dirs(self, dirs: Dirs) -> Repo {
        Repo { dirs: Some(Arc::new(dirs)), ..self }
    }

    pub fn known_dirs(&self) -> Option<&Dirs> {
        self.dirs.as_deref()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    /// `git <READ_ONLY> <args>` in `dir`, whatever its exit code.
    pub(crate) fn run(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<RawOutput, GitError> {
        self.run_with(READ_ONLY.iter(), args, input)
    }

    /// [`Repo::run`] without `--literal-pathspecs`, for a command that takes paths and refuses pathspec magic
    /// (`check-ignore`).
    pub(crate) fn run_paths(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<RawOutput, GitError> {
        self.run_with(READ_ONLY.iter().filter(|a| **a != "--literal-pathspecs"), args, input)
    }

    fn run_with<'a>(&self, read_only: impl Iterator<Item = &'a &'a str>, args: &[&str], input: Option<Vec<u8>>) -> Result<RawOutput, GitError> {
        let mut cmd = self.command(read_only, args);
        process::run_bytes(&mut cmd, input, GIT_TIMEOUT, Kill::Tree).map_err(|source| GitError::Run { args: args.join(" "), source })
    }

    /// [`Repo::run`] as a child whose stdout is read as git writes it.
    pub(crate) fn stream(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<Streaming, GitError> {
        let mut cmd = self.command(READ_ONLY.iter(), args);
        process::stream(&mut cmd, input).map_err(|e| GitError::Run { args: args.join(" "), source: RunError::Spawn(e) })
    }

    fn command<'a>(&self, read_only: impl Iterator<Item = &'a &'a str>, args: &[&str]) -> Command {
        let mut cmd = git_command(&self.git, &self.dir);
        cmd.args(read_only).args(args);
        cmd
    }

    /// [`Repo::run`]'s stdout, an error unless git exited 0.
    pub(crate) fn output(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>, GitError> {
        let out = self.run(args, input)?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(failed(args.join(" "), &self.dir, &out))
        }
    }

    /// [`Repo::read`] without `--no-optional-locks`: git may write the index's stat cache.
    pub(crate) fn read_locking<T>(&self, args: &[&str], parse: impl FnOnce(&[u8]) -> Result<T, String>) -> Result<T, GitError> {
        let out = self.run_with(READ_ONLY.iter().filter(|a| **a != "--no-optional-locks"), args, None)?;
        if !out.success() {
            return Err(failed(args.join(" "), &self.dir, &out));
        }
        parse(&out.stdout).map_err(|what| GitError::Parse { args: args.join(" "), what })
    }

    /// [`Repo::output`] parsed, a parse failure naming the command.
    pub(crate) fn read<T>(&self, args: &[&str], input: Option<Vec<u8>>, parse: impl FnOnce(&[u8]) -> Result<T, String>) -> Result<T, GitError> {
        let out = self.output(args, input)?;
        parse(&out).map_err(|what| GitError::Parse { args: args.join(" "), what })
    }
}

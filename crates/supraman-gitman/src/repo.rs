use std::path::{Path, PathBuf};

use supraman_core::process::{self, GIT_TIMEOUT, Kill, RawOutput, RunError};

use crate::Git;

/// Before every subcommand (research §2): no lock a reader could leave behind, no pager, paths and
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

/// A repository (or a folder inside one) read with one git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    git: Git,
    dir: PathBuf,
}

impl Repo {
    pub fn new(git: Git, dir: impl Into<PathBuf>) -> Repo {
        Repo { git, dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    /// `git <READ_ONLY> <args>` in `dir`, whatever its exit code.
    pub(crate) fn run(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<RawOutput, GitError> {
        let mut cmd = process::command(&self.git.path);
        cmd.arg("-C").arg(&self.dir).args(READ_ONLY).args(args);
        cmd.env("GIT_TERMINAL_PROMPT", "0").env("GIT_EDITOR", "").env("TERM", "dumb");
        REPO_ENV.iter().for_each(|var| {
            cmd.env_remove(var);
        });
        process::run_bytes(&mut cmd, input, GIT_TIMEOUT, Kill::Tree).map_err(|source| GitError::Run { args: args.join(" "), source })
    }

    /// [`Repo::run`]'s stdout, an error unless git exited 0.
    pub(crate) fn output(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>, GitError> {
        let out = self.run(args, input)?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(GitError::Failed { args: args.join(" "), code: out.code, stderr: String::from_utf8_lossy(&out.stderr).trim().to_string() })
        }
    }

    /// [`Repo::output`] parsed, a parse failure naming the command.
    pub(crate) fn read<T>(&self, args: &[&str], input: Option<Vec<u8>>, parse: impl FnOnce(&[u8]) -> Result<T, String>) -> Result<T, GitError> {
        let out = self.output(args, input)?;
        parse(&out).map_err(|what| GitError::Parse { args: args.join(" "), what })
    }
}

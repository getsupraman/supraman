//! GitMan's one write layer: every change to a repository runs through a [`Writer`], which holds the operation's
//! [`LockSet`] on the repo's [`Scheduler`], pauses the repo's watchers ([`Quiet`]) and runs git with [`PINNED`].
//! Blocking: callers run it on a background executor.

pub mod branch;
pub mod commit;
pub mod conflicts;
pub mod files;
mod guard;
pub mod journal;
pub mod lines;
mod locks;
pub mod maintain;
mod quiet;
pub mod rebase;
pub mod remote;
mod snapshot;
pub mod stash;
pub mod wip;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError};

use supraman_core::process::{self, GIT_TIMEOUT, Kill, RawOutput, RunError};

pub use guard::{Occupancy, working_in};
pub use locks::{Access, Held, LockSet, Resource, Scheduler};
pub use quiet::{Quiet, Settled};

use crate::parse::text;
use crate::repo::git_command;
use crate::{Dirs, GitError, Repo};

/// Before every write's subcommand: no auto maintenance, paths as UTF-8 text, no colour, no signature checks.
pub const PINNED: &[&str] = &[
    "-c", "gc.auto=0",
    "-c", "maintenance.auto=0",
    "-c", "core.quotePath=false",
    "-c", "color.ui=never",
    "-c", "color.diff=never",
    "-c", "log.showSignature=false",
];

/// Why a write is refused while a merge, rebase, cherry-pick, revert or am is under way.
pub const UNDER_WAY: &str = "Finish or abort the operation under way first";

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error(transparent)]
    Git(GitError),
    /// git refuses a repository another user owns until it is trusted (`safe.directory`).
    #[error("git does not trust {path}: another user owns it")]
    DubiousOwnership { path: String },
    #[error("git {args}: more than one path and an input, both on stdin")]
    PathsAndInput { args: String },
    #[error("{what}: {source}")]
    Io { what: String, source: std::io::Error },
    #[error("undo journal: {0}")]
    Journal(String),
    /// git, or one of its hooks, refused the commit: its exit code and what it wrote.
    #[error("the commit failed with exit code {code}: {stderr}")]
    CommitFailed { code: i32, stderr: String },
    /// Refused before git ran.
    #[error("{0}")]
    Refused(&'static str),
    /// The user stopped a fetch or a push; its git tree was killed.
    #[error("cancelled")]
    Cancelled,
    /// A remote refused, or some of several remotes failed: each with its reason.
    #[error("{0}")]
    Remote(String),
}

impl WriteError {
    /// The failure of `verb` (`read`, `write`, …) on `path`.
    pub fn io(verb: &str, path: &Path) -> impl FnOnce(std::io::Error) -> WriteError + use<> {
        let what = format!("{verb} {}", path.display());
        move |source| WriteError::Io { what, source }
    }
}

impl From<GitError> for WriteError {
    fn from(e: GitError) -> WriteError {
        let stderr = match &e {
            GitError::Failed { stderr, .. } => stderr.as_str(),
            _ => "",
        };
        match dubious(stderr) {
            Some(path) => WriteError::DubiousOwnership { path },
            None => WriteError::Git(e),
        }
    }
}

/// What git said of a failure: its `fatal:` line, else its last.
pub fn reason(said: &str) -> String {
    let fatal = said.lines().find_map(|l| l.split_once("fatal: ").map(|(_, r)| r));
    fatal.or_else(|| said.lines().rev().find(|l| !l.trim().is_empty())).unwrap_or(said).trim().to_string()
}

/// The repository git's "dubious ownership" refusal names.
fn dubious(stderr: &str) -> Option<String> {
    let rest = stderr.split_once("detected dubious ownership in repository at '")?.1;
    Some(rest.split_once('\'').map_or(rest, |(path, _)| path).to_string())
}

/// One git subcommand of a write: its arguments, the paths it acts on (each a literal pathspec), its stdin,
/// its own environment and editor.
#[derive(Debug, Clone, Default)]
pub struct Call {
    args: Vec<String>,
    paths: Vec<String>,
    input: Option<Vec<u8>>,
    env: Vec<(String, OsString)>,
    editor: Option<OsString>,
}

impl Call {
    pub fn new(args: &[&str]) -> Call {
        Call { args: args.iter().map(|a| a.to_string()).collect(), ..Call::default() }
    }

    /// Paths from the checkout's top, `/`-separated.
    pub fn paths<S: Into<String>>(mut self, paths: impl IntoIterator<Item = S>) -> Call {
        self.paths.extend(paths.into_iter().map(Into::into));
        self
    }

    pub fn input(mut self, bytes: Vec<u8>) -> Call {
        self.input = Some(bytes);
        self
    }

    pub fn env(mut self, key: &str, value: impl Into<OsString>) -> Call {
        self.env.push((key.to_string(), value.into()));
        self
    }

    pub fn editor(mut self, command: impl Into<OsString>) -> Call {
        self.editor = Some(command.into());
        self
    }

    fn label(&self) -> String {
        self.args.join(" ")
    }
}

struct PerRepo {
    scheduler: Arc<Scheduler>,
    quiet: Arc<Quiet>,
}

/// The scheduler and the pause of the repository `dirs` belong to, one per common git dir in the process: every
/// Git tab and every worktree of one repository share them.
fn per_repo(dirs: &Dirs) -> (Arc<Scheduler>, Arc<Quiet>) {
    static REPOS: Mutex<Option<HashMap<String, PerRepo>>> = Mutex::new(None);
    let mut repos = REPOS.lock().unwrap_or_else(PoisonError::into_inner);
    let repo = repos
        .get_or_insert_with(HashMap::new)
        .entry(supraman_core::paths::path_key(&dirs.common_dir))
        .or_insert_with(|| PerRepo { scheduler: Arc::default(), quiet: Arc::default() });
    (repo.scheduler.clone(), repo.quiet.clone())
}

pub fn scheduler(dirs: &Dirs) -> Arc<Scheduler> {
    per_repo(dirs).0
}

pub fn quiet(dirs: &Dirs) -> Arc<Quiet> {
    per_repo(dirs).1
}

/// A reader's hold on the index of `repo` (one whose dirs are known), so it waits for an index write; blocking.
pub fn refresh_lock(repo: &Repo) -> Option<Held> {
    repo.known_dirs().map(|dirs| scheduler(dirs).acquire(LockSet::refresh()))
}

/// Writes to one checkout: its top, git dirs, `core.ignorecase`, and its repository's scheduler and pause.
#[derive(Clone)]
pub struct Writer {
    repo: Repo,
    dirs: Dirs,
    icase: bool,
    scheduler: Arc<Scheduler>,
    quiet: Arc<Quiet>,
}

impl Writer {
    /// The writer of the checkout `repo` is in; blocking (it asks git for the dirs and `core.ignorecase`).
    pub fn open(repo: &Repo) -> Result<Writer, WriteError> {
        let dirs = crate::dirs(repo)?;
        let top = Repo::new(repo.git().clone(), &dirs.top).with_dirs(dirs.clone());
        let out = top.run(&["config", "--type=bool", "--get", "core.ignorecase"], None)?;
        if let Some(path) = dubious(&text(&out.stderr)) {
            return Err(WriteError::DubiousOwnership { path });
        }
        let icase = out.success() && text(&out.stdout).trim() == "true";
        let (scheduler, quiet) = per_repo(&dirs);
        Ok(Writer { repo: top, dirs, icase, scheduler, quiet })
    }

    pub fn dirs(&self) -> &Dirs {
        &self.dirs
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    pub fn scheduler(&self) -> &Arc<Scheduler> {
        &self.scheduler
    }

    pub fn quiet(&self) -> &Arc<Quiet> {
        &self.quiet
    }

    /// `path` as git's literal pathspec, ignoring case where the repo does.
    pub fn pathspec(&self, path: &str) -> String {
        format!(":({}){path}", if self.icase { "literal,icase" } else { "literal" })
    }

    /// The git child of `call` in the checkout's top and its stdin; spawns nothing.
    pub fn prepare(&self, call: &Call) -> Result<(Command, Option<Vec<u8>>), WriteError> {
        let mut cmd = git_command(self.repo.git(), &self.dirs.top);
        cmd.env("LC_ALL", "C").env_remove("LANG").env_remove("LANGUAGE");
        if let Some(editor) = &call.editor {
            cmd.env("GIT_EDITOR", editor);
        }
        call.env.iter().for_each(|(k, v)| {
            cmd.env(k, v);
        });
        cmd.args(PINNED).args(&call.args);
        let mut input = call.input.clone();
        match call.paths.as_slice() {
            [] => {}
            [one] => {
                cmd.arg("--").arg(self.pathspec(one));
            }
            many => {
                if input.is_some() {
                    return Err(WriteError::PathsAndInput { args: call.label() });
                }
                cmd.args(["--pathspec-from-file=-", "--pathspec-file-nul"]);
                input = Some(many.iter().flat_map(|p| self.pathspec(p).into_bytes().into_iter().chain([0])).collect());
            }
        }
        Ok((cmd, input))
    }

    /// Runs `f` holding `locks`; while they include a write the repo's watchers are paused, and they hear once the
    /// last write ends. Blocks until the locks are free.
    pub fn run<T>(&self, locks: LockSet, f: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
        let held = self.scheduler.acquire(locks);
        let _pausing = held.locks().has_write().then(|| Pausing::new(self.quiet.clone(), held.id()));
        let session = Session { writer: self, held };
        f(&session)
    }

    /// [`Writer::run`] of a background job, `None` when `locks` are not free now; the watchers are not paused,
    /// so they refresh only for what it changed.
    pub fn try_run_watched<T>(&self, locks: LockSet, f: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Option<Result<T, WriteError>> {
        let held = self.scheduler.try_acquire(locks)?;
        Some(f(&Session { writer: self, held }))
    }

    /// A dangling commit of the index and the working files of `paths`, `what` its message; see [`Session::snapshot`].
    pub fn snapshot(&self, paths: &[&str], what: &str) -> Result<String, WriteError> {
        self.run(LockSet::refresh(), |session| session.snapshot(paths, what))
    }
}

/// A write's pause of the watchers, ended after its locks are released (it drops after the [`Session`]).
struct Pausing {
    quiet: Arc<Quiet>,
    id: u64,
}

impl Pausing {
    fn new(quiet: Arc<Quiet>, id: u64) -> Pausing {
        quiet.begin(id);
        Pausing { quiet, id }
    }
}

impl Drop for Pausing {
    fn drop(&mut self) {
        self.quiet.end(self.id);
    }
}

/// A write under way: its locks held.
pub struct Session<'a> {
    writer: &'a Writer,
    held: Held,
}

impl Session<'_> {
    pub fn writer(&self) -> &Writer {
        self.writer
    }

    pub fn held(&self) -> &Held {
        &self.held
    }

    /// `call` run, whatever its exit code.
    pub fn run(&self, call: &Call) -> Result<RawOutput, WriteError> {
        let (mut cmd, input) = self.writer.prepare(call)?;
        process::run_bytes(&mut cmd, input, GIT_TIMEOUT, Kill::Tree).map_err(|source: RunError| WriteError::Git(GitError::Run { args: call.label(), source }))
    }

    /// `call`'s stdout, an error unless git exited 0.
    pub fn output(&self, call: &Call) -> Result<Vec<u8>, WriteError> {
        let out = self.run(call)?;
        if out.success() {
            return Ok(out.stdout);
        }
        Err(crate::repo::failed(call.label(), &self.writer.dirs.top, &out).into())
    }

    /// `call`'s stdout as one trimmed line, an error unless git exited 0.
    pub fn line(&self, call: &Call) -> Result<String, WriteError> {
        Ok(text(&self.output(call)?).trim().to_string())
    }

    /// The object `name` names, `None` when it names none.
    pub fn verify(&self, name: &str) -> Result<Option<String>, WriteError> {
        let out = self.run(&Call::new(&["rev-parse", "-q", "--verify", name]))?;
        Ok(out.success().then(|| text(&out.stdout).trim().to_string()))
    }

    /// The object `name` names, an error when it names none.
    pub fn rev(&self, name: &str) -> Result<String, WriteError> {
        self.line(&Call::new(&["rev-parse", "--verify", name]))
    }

    /// Runs `f` holding `locks` as well, what this session holds not asked again.
    pub fn nested<T>(&self, locks: LockSet, f: impl FnOnce(&Session<'_>) -> Result<T, WriteError>) -> Result<T, WriteError> {
        match self.held.nested(locks) {
            Some(held) => {
                let _pausing = held.locks().has_write().then(|| Pausing::new(self.writer.quiet.clone(), held.id()));
                f(&Session { writer: self.writer, held })
            }
            None => f(self),
        }
    }

    fn path_in_git_dir(&self, name: &str) -> std::path::PathBuf {
        Path::new(&self.writer.dirs.git_dir).join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_gits_fatal_line_else_its_last() {
        assert_eq!(reason("remote: x\nfatal: could not read from remote\nmore"), "could not read from remote");
        assert_eq!(reason("first\nlast line\n\n"), "last line");
    }

    #[test]
    fn dubious_ownership_is_its_own_error_naming_the_folder() {
        let stderr = "fatal: detected dubious ownership in repository at 'C:/work/repo'\n'C:/work/repo' is owned by:\n\tBUILTIN/Administrators";
        assert!(matches!(WriteError::from(GitError::Failed { args: "add".into(), code: Some(128), stderr: stderr.into() }), WriteError::DubiousOwnership { path } if path == "C:/work/repo"));
        assert!(matches!(WriteError::from(GitError::Failed { args: "add".into(), code: Some(1), stderr: "fatal: other".into() }), WriteError::Git(_)));
    }
}

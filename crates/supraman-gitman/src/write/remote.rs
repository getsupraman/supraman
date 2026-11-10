//! GitMan's fetch, pull and push (docs/research/gitkraken-12.6.1.md §4 part B); never `git pull`.

use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use supraman_core::process::{self, Pipe, RawOutput, Waited};

use super::rebase::{self, Rebased};
use super::{Call, Session, WriteError, Writer};
use crate::parse::text;
use crate::{GitError, OperationState, operation_state};

pub const DETACHED: &str = "HEAD is detached: check out a branch first";
pub const NO_REMOTES: &str = "This repository has no remote to fetch from";

/// How often a transfer looks at its Cancel.
const TICK: Duration = Duration::from_millis(20);
/// A transfer silent this long is ended: no progress, and no prompt answered.
const SILENT: Duration = Duration::from_secs(15 * 60);

/// One phase of git's `--progress`: `Receiving objects`, `Writing objects`, `Resolving deltas`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub phase: String,
    pub percent: u8,
}

/// `line` as git's progress, a `remote: ` prefix dropped: `Receiving objects:  45% (9/20)`.
pub fn progress_of(line: &str) -> Option<Progress> {
    let line = line.trim().trim_start_matches("remote: ").trim();
    let (phase, rest) = line.split_once(": ")?;
    let percent = rest.trim_start().split_once('%')?.0.trim().parse::<u8>().ok()?;
    (!phase.is_empty() && phase.chars().all(|c| c.is_ascii_alphabetic() || c == ' ')).then(|| Progress { phase: phase.to_string(), percent: percent.min(100) })
}

/// How a remote call reaches the user: its progress, its Cancel, and the askpass environment of its ssh.
#[derive(Clone)]
pub struct Transfer {
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<dyn Fn(Progress) + Send + Sync>,
    pub env: Vec<(String, OsString)>,
}

impl Default for Transfer {
    fn default() -> Transfer {
        Transfer { cancel: Arc::default(), progress: Arc::new(|_| {}), env: Vec::new() }
    }
}

/// What a transfer left: git's exit, its stdout, and what it wrote to stderr besides its progress.
struct Moved {
    code: Option<i32>,
    stdout: String,
    said: String,
}

impl Moved {
    fn success(&self) -> bool {
        self.code == Some(0)
    }

    fn failed(self, call: &Call) -> WriteError {
        GitError::Failed { args: call.label(), code: self.code, stderr: self.said.trim().to_string() }.into()
    }
}

/// `call` with `--progress` read off its stderr, stopped by [`Transfer::cancel`].
fn transfer(writer: &Writer, call: &Call, t: &Transfer) -> Result<Moved, WriteError> {
    let mut call = call.clone();
    for (k, v) in &t.env {
        call = call.env(k, v.clone());
    }
    let (mut cmd, input) = writer.prepare(&call)?;
    let failed = |source| WriteError::Git(GitError::Run { args: call.label(), source });
    let mut child = process::stream_pipe(&mut cmd, input, Pipe::Stderr).map_err(|e| failed(process::RunError::Spawn(e)))?;
    let (mut said, mut line) = (String::new(), String::new());
    let mut heard = Instant::now();
    loop {
        if t.cancel.load(Ordering::SeqCst) {
            drop(child);
            return Err(WriteError::Cancelled);
        }
        match child.wait(TICK) {
            Waited::Chunk(bytes) => {
                heard = Instant::now();
                for c in text(&bytes).chars() {
                    if c != '\r' && c != '\n' {
                        line.push(c);
                        continue;
                    }
                    match progress_of(&line) {
                        Some(p) => (t.progress)(p),
                        None if !line.trim().is_empty() => {
                            said.push_str(&line);
                            said.push('\n');
                        }
                        None => {}
                    }
                    line.clear();
                }
            }
            Waited::Idle if heard.elapsed() > SILENT => {
                drop(child);
                return Err(failed(process::RunError::Timeout(SILENT)));
            }
            Waited::Idle => {}
            Waited::End => break,
        }
    }
    said.push_str(&line);
    let RawOutput { code, stdout, .. } = child.finish(process::GIT_TIMEOUT).map_err(failed)?;
    Ok(Moved { code, stdout: text(&stdout), said })
}

fn idle(s: &Session) -> Result<(), WriteError> {
    match operation_state(&s.writer().dirs().git_dir) {
        OperationState::None => Ok(()),
        _ => Err(WriteError::Refused(super::UNDER_WAY)),
    }
}

fn current(s: &Session) -> Result<Option<String>, WriteError> {
    let name = text(&s.run(&Call::new(&["symbolic-ref", "-q", "--short", "HEAD"]))?.stdout).trim().to_string();
    Ok((!name.is_empty()).then_some(name))
}

fn config(s: &Session, key: &str) -> Result<Option<String>, WriteError> {
    let out = s.run(&Call::new(&["config", "--get", key]))?;
    let value = text(&out.stdout).trim().to_string();
    Ok((out.success() && !value.is_empty()).then_some(value))
}

/// The fetch of `remote`: its branches forced, its tags, its gone branches pruned when `prune`.
pub fn fetch_call(remote: &str, prune: bool) -> Call {
    let prune = format!("fetch.prune={prune}");
    Call::new(&["-c", &prune, "fetch", "--force", "--tags", "--progress", "--end-of-options", remote])
}

pub fn fetch(s: &Session, remote: &str, prune: bool, t: &Transfer) -> Result<(), WriteError> {
    let call = fetch_call(remote, prune);
    let moved = transfer(s.writer(), &call, t)?;
    if moved.success() { Ok(()) } else { Err(moved.failed(&call)) }
}

/// Every remote fetched at once; the ones that failed, each with its reason, as one error.
pub fn fetch_all(s: &Session, remotes: &[String], prune: bool, t: &Transfer) -> Result<(), WriteError> {
    if remotes.is_empty() {
        return Err(WriteError::Refused(NO_REMOTES));
    }
    let results = fetch_each(s, remotes, prune, t);
    if results.iter().any(|(_, r)| matches!(r, Err(WriteError::Cancelled))) {
        return Err(WriteError::Cancelled);
    }
    let failed: Vec<String> = results.into_iter().filter_map(|(remote, r)| r.err().map(|e| format!("{remote}: {e}"))).collect();
    if failed.is_empty() { Ok(()) } else { Err(WriteError::Remote(failed.join("
"))) }
}

/// Enough for a usual repo's remotes to go together, without a git and an ssh for each of many.
const FETCHES_AT_ONCE: usize = 8;

/// The remotes fetched [`FETCHES_AT_ONCE`] at a time; each remote's own outcome.
pub fn fetch_each(s: &Session, remotes: &[String], prune: bool, t: &Transfer) -> Vec<(String, Result<(), WriteError>)> {
    let writer = s.writer();
    let (said, heard) = std::sync::mpsc::channel::<Progress>();
    let fetch = move |remote: &String| {
        let call = fetch_call(remote, prune);
        let said = std::sync::Mutex::new(said.clone());
        let each = Transfer { progress: Arc::new(move |p| drop(said.lock().map(|s| s.send(p)))), ..t.clone() };
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| transfer(writer, &call, &each).and_then(|m| if m.success() { Ok(()) } else { Err(m.failed(&call)) })));
        (remote.clone(), run.unwrap_or(Err(WriteError::Refused("a fetch panicked"))))
    };
    std::thread::scope(|scope| {
        let runs = scope.spawn(move || supraman_core::pool::map(remotes.iter().collect(), FETCHES_AT_ONCE, fetch));
        heard.iter().for_each(|p| (t.progress)(p));
        runs.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// A fetch nobody waits for: no prompt from git, the credential helper or ssh, so a remote that needs one fails.
pub fn silent() -> Transfer {
    let env = [("GIT_TERMINAL_PROMPT", "0"), ("GCM_INTERACTIVE", "never"), ("SSH_ASKPASS_REQUIRE", "never")];
    Transfer { env: env.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect(), ..Transfer::default() }
}

/// What a background fetch did: each remote's outcome, and whether a ref moved.
#[derive(Debug, Default)]
pub struct Fetched {
    pub remotes: Vec<(String, Result<(), String>)>,
    pub moved: bool,
}

/// Every remote fetched [`silent`]ly, nothing while an operation is under way.
pub fn background_fetch(s: &Session, remotes: &[String], prune: bool) -> Result<Fetched, WriteError> {
    if !matches!(operation_state(&s.writer().dirs().git_dir), OperationState::None) {
        return Ok(Fetched::default());
    }
    let tips = |s: &Session| s.output(&Call::new(&["for-each-ref", "--format=%(objectname) %(refname)"]));
    let before = tips(s)?;
    let remotes = fetch_each(s, remotes, prune, &silent()).into_iter().map(|(r, done)| (r, done.map_err(|e| e.to_string()))).collect();
    Ok(Fetched { remotes, moved: tips(s)? != before })
}

/// A branch's upstream as its config names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// `.` for a local branch.
    pub remote: String,
    /// The branch on the remote, without `refs/heads/`.
    pub branch: String,
}

impl Upstream {
    /// What a merge or a rebase takes: `refs/remotes/<remote>/<branch>`, or the local branch.
    pub fn tracking(&self) -> String {
        if self.remote == "." { format!("refs/heads/{}", self.branch) } else { format!("refs/remotes/{}/{}", self.remote, self.branch) }
    }
}

pub fn upstream_of(s: &Session, branch: &str) -> Result<Option<Upstream>, WriteError> {
    let remote = config(s, &format!("branch.{branch}.remote"))?;
    let merge = config(s, &format!("branch.{branch}.merge"))?;
    Ok(remote.zip(merge).map(|(remote, merge)| Upstream { remote, branch: merge.strip_prefix("refs/heads/").unwrap_or(&merge).to_string() }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    /// A merge commit when the branches went apart.
    FastForwardIfPossible,
    FastForwardOnly,
    Rebase,
}

/// What the toolbar's Pull does: `[gitman] pull`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PullChoice {
    Fetch,
    #[default]
    FastForwardIfPossible,
    FastForwardOnly,
    Rebase,
}

impl PullChoice {
    pub const ALL: [PullChoice; 4] = [PullChoice::Fetch, PullChoice::FastForwardIfPossible, PullChoice::FastForwardOnly, PullChoice::Rebase];

    /// The pull it runs; `None` for a fetch of every remote.
    pub fn mode(self) -> Option<PullMode> {
        match self {
            PullChoice::Fetch => None,
            PullChoice::FastForwardIfPossible => Some(PullMode::FastForwardIfPossible),
            PullChoice::FastForwardOnly => Some(PullMode::FastForwardOnly),
            PullChoice::Rebase => Some(PullMode::Rebase),
        }
    }
}

impl From<PullMode> for PullChoice {
    fn from(mode: PullMode) -> PullChoice {
        match mode {
            PullMode::FastForwardIfPossible => PullChoice::FastForwardIfPossible,
            PullMode::FastForwardOnly => PullChoice::FastForwardOnly,
            PullMode::Rebase => PullChoice::Rebase,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pulled {
    UpToDate,
    Merged,
    Rebased(Rebased),
    /// The merge stopped on conflicts: the WIP holds them.
    Conflicted,
    /// `branch` has no upstream: ask for one.
    NoUpstream { branch: String },
    /// The upstream's branch is gone from the remote.
    Gone { upstream: String },
}

/// HEAD's branch brought up to its upstream: that remote fetched, then GitMan's merge or rebase; never `git pull`.
pub fn pull(s: &Session, mode: PullMode, prune: bool, t: &Transfer) -> Result<Pulled, WriteError> {
    idle(s)?;
    let Some(branch) = current(s)? else { return Err(WriteError::Refused(DETACHED)) };
    let Some(upstream) = upstream_of(s, &branch)? else { return Ok(Pulled::NoUpstream { branch }) };
    if upstream.remote != "." {
        fetch(s, &upstream.remote, prune, t)?;
    }
    let tracking = upstream.tracking();
    let Some(theirs) = s.verify(&format!("{tracking}^{{commit}}"))? else { return Ok(Pulled::Gone { upstream: format!("{}/{}", upstream.remote, upstream.branch) }) };
    if s.run(&Call::new(&["merge-base", "--is-ancestor", &theirs, "HEAD"]))?.success() {
        return Ok(Pulled::UpToDate);
    }
    if mode == PullMode::Rebase {
        return rebase::onto(s, &branch, &tracking).map(Pulled::Rebased);
    }
    let mut args = vec!["merge", "--autostash", "--no-edit"];
    if mode == PullMode::FastForwardOnly {
        args.push("--ff-only");
    }
    args.push(&tracking);
    let call = Call::new(&args);
    let out = s.run(&call)?;
    if out.success() {
        return Ok(Pulled::Merged);
    }
    if operation_state(&s.writer().dirs().git_dir) == OperationState::Merge {
        return Ok(Pulled::Conflicted);
    }
    Err(GitError::Failed { args: call.label(), code: out.code, stderr: [text(&out.stdout), text(&out.stderr)].join("\n").trim().to_string() }.into())
}

/// How a push treats a remote branch that moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Force {
    No,
    /// `--force-with-lease`, expecting the remote branch where this repo last saw it.
    Lease,
    /// `--force`: whatever the remote holds.
    Anyway,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pushed {
    Done,
    /// The remote has commits this branch lacks: pull, or force with lease.
    Behind,
    /// The remote branch moved since it was fetched: fetch, or force anyway.
    StaleLease,
}

/// `refs/heads/<local>` pushed to `refs/heads/<branch>` on `remote`.
pub fn push(s: &Session, local: &str, remote: &str, branch: &str, force: Force, t: &Transfer) -> Result<Pushed, WriteError> {
    let dest = format!("refs/heads/{branch}");
    let lease = match force {
        Force::Lease => Some(format!("--force-with-lease={dest}:{}", s.verify(&format!("refs/remotes/{remote}/{branch}^{{commit}}"))?.unwrap_or_default())),
        _ => None,
    };
    let spec = format!("refs/heads/{local}:{dest}");
    let mut args = vec!["push", "--porcelain", "--progress"];
    args.extend(lease.as_deref());
    if force == Force::Anyway {
        args.push("--force");
    }
    args.extend(["--end-of-options", remote, &spec]);
    push_refs(s, &Call::new(&args), t)
}

fn push_refs(s: &Session, call: &Call, t: &Transfer) -> Result<Pushed, WriteError> {
    let moved = transfer(s.writer(), call, t)?;
    if moved.success() {
        return Ok(Pushed::Done);
    }
    match rejection(&moved.stdout) {
        Some(pushed) => Ok(pushed),
        None => Err(moved.failed(call)),
    }
}

/// `push --porcelain`'s rejected line, as what the user can do about it.
fn rejection(porcelain: &str) -> Option<Pushed> {
    let line = porcelain.lines().find(|l| l.starts_with('!'))?;
    if line.contains("(stale info)") {
        Some(Pushed::StaleLease)
    } else if line.contains("(fetch first)") || line.contains("(non-fast-forward)") {
        Some(Pushed::Behind)
    } else {
        None
    }
}

/// `local`'s upstream set; written to the config directly while unfetched, since git refuses a tracking ref it lacks.
pub fn set_upstream(s: &Session, local: &str, remote: &str, branch: &str) -> Result<(), WriteError> {
    if s.verify(&format!("refs/remotes/{remote}/{branch}^{{commit}}"))?.is_some() {
        s.output(&Call::new(&["branch", &format!("--set-upstream-to={remote}/{branch}"), "--", local]))?;
        return Ok(());
    }
    s.output(&Call::new(&["config", &format!("branch.{local}.remote"), remote]))?;
    s.output(&Call::new(&["config", &format!("branch.{local}.merge"), &format!("refs/heads/{branch}")]))?;
    Ok(())
}

/// A branch with no upstream pushed to `branch` on `remote`, then that made its upstream.
pub fn publish(s: &Session, local: &str, remote: &str, branch: &str, t: &Transfer) -> Result<Pushed, WriteError> {
    let pushed = push(s, local, remote, branch, Force::No, t)?;
    if pushed == Pushed::Done {
        set_upstream(s, local, remote, branch)?;
    }
    Ok(pushed)
}

pub fn push_tag(s: &Session, remote: &str, tag: &str, t: &Transfer) -> Result<Pushed, WriteError> {
    let spec = format!("refs/tags/{tag}:refs/tags/{tag}");
    push_refs(s, &Call::new(&["push", "--porcelain", "--progress", "--end-of-options", remote, &spec]), t)
}

/// `refname` (`refs/heads/<b>`, `refs/tags/<t>`) deleted on `remote`; not undoable.
pub fn delete_remote_ref(s: &Session, remote: &str, refname: &str, t: &Transfer) -> Result<(), WriteError> {
    let call = Call::new(&["push", "--porcelain", "--progress", "--delete", "--end-of-options", remote, refname]);
    match push_refs(s, &call, t)? {
        Pushed::Done => Ok(()),
        _ => Err(WriteError::Remote(format!("{remote} refused to delete {refname}"))),
    }
}

pub fn set_url(s: &Session, remote: &str, url: &str) -> Result<(), WriteError> {
    if url.trim().is_empty() {
        return Err(WriteError::Refused("A remote needs a URL"));
    }
    s.output(&Call::new(&["remote", "set-url", "--", remote, url.trim()]))?;
    Ok(())
}

/// `remote`, its remote-tracking branches and the upstream settings naming it removed; not undoable.
pub fn remove_remote(s: &Session, remote: &str) -> Result<(), WriteError> {
    s.output(&Call::new(&["remote", "remove", "--", remote]))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_parse_and_others_do_not() {
        assert_eq!(progress_of("Receiving objects:  45% (9/20), 1.2 MiB | 2 MiB/s"), Some(Progress { phase: "Receiving objects".into(), percent: 45 }));
        assert_eq!(progress_of("remote: Counting objects: 100% (3/3), done."), Some(Progress { phase: "Counting objects".into(), percent: 100 }));
        assert_eq!(progress_of("Writing objects: 7% (1/14)"), Some(Progress { phase: "Writing objects".into(), percent: 7 }));
        assert_eq!(progress_of("fatal: repository 'x' not found"), None);
        assert_eq!(progress_of("To C:/origin.git"), None);
    }

    #[test]
    fn a_rejection_says_behind_or_stale() {
        assert_eq!(rejection("To C:/o.git\n!\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)\nDone\n"), Some(Pushed::Behind));
        assert_eq!(rejection("!\trefs/heads/a:refs/heads/a\t[rejected] (non-fast-forward)\n"), Some(Pushed::Behind));
        assert_eq!(rejection("!\trefs/heads/a:refs/heads/a\t[rejected] (stale info)\n"), Some(Pushed::StaleLease));
        assert_eq!(rejection("!\trefs/heads/a:refs/heads/a\t[remote rejected] (hook declined)\n"), None);
    }
}

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::store::{load, schedules_dir};
use crate::{CoreError, Registry, Result, validate_name};

/// Where [`append_run`] rotates a workspace's run records.
pub const RUNS_CAP: u64 = 1024 * 1024;

/// One line of `schedules/runs/<workspace>.jsonl`; never a token, never stdout, stderr only masked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub ts: i64,
    pub run: String,
    pub schedule: String,
    #[serde(flatten)]
    pub event: RunEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RunEvent {
    Started {
        trigger: Fired,
        fingerprint: String,
        fm_pid: u32,
    },
    Stage(Stage),
    Ended {
        outcome: Outcome,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        why: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<Tokens>,
        ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    Script {
        code: Option<i32>,
        ms: u64,
        /// The last 2 KiB, masked for the schedule's secrets.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stderr: String,
    },
    Session {
        name: String,
        id: String,
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workstate_session: Option<String>,
        /// Why the run has no Workman, when it has none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        without_workstate: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fired {
    Task,
    RunNow,
    CatchUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    NothingToDo,
    Ok,
    Done,
    NeedsYou,
    Failed,
    Refused,
    Skipped,
    CappedRuntime,
    CappedTokens,
    Interrupted,
}

impl Outcome {
    /// Goes to the Inbox and the tray.
    pub fn needs_attention(self) -> bool {
        matches!(self, Self::NeedsYou | Self::Failed | Self::Refused | Self::CappedRuntime | Self::CappedTokens | Self::Interrupted)
    }

    /// Task Scheduler's Last Run Result 0.
    pub fn succeeded(self) -> bool {
        matches!(self, Self::Done | Self::Ok | Self::NothingToDo | Self::Skipped)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl Tokens {
    /// What `max_tokens` caps: cache reads are left out.
    pub fn capped(&self) -> u64 {
        self.input + self.output + self.cache_creation
    }
}

/// A run's lines folded together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub run: String,
    pub schedule: String,
    pub started: Option<(i64, Fired)>,
    pub stages: Vec<Stage>,
    /// `(ts, outcome, why)`.
    pub ended: Option<(i64, Outcome, String)>,
    /// The `ended` line's token totals.
    pub tokens: Option<Tokens>,
}

pub fn runs_path(home: &Path, workspace: &str) -> Result<PathBuf> {
    validate_name(workspace)?;
    Ok(schedules_dir(home).join("runs").join(format!("{workspace}.jsonl")))
}

pub fn append_run(home: &Path, workspace: &str, record: &RunRecord) -> Result<()> {
    let path = runs_path(home, workspace)?;
    let dir = path.parent().expect("under the home");
    fs::create_dir_all(dir).map_err(|e| CoreError::io(dir, e))?;
    let mut line = serde_json::to_vec(record).expect("plain data serializes");
    line.push(b'\n');
    crate::fs::append_capped(&path, &line, RUNS_CAP).map_err(|e| CoreError::io(&path, e))
}

/// The rotated file's lines, then the current file's, oldest first; a line that does not parse is skipped.
pub fn read_runs(home: &Path, workspace: &str) -> Result<Vec<RunRecord>> {
    let path = runs_path(home, workspace)?;
    let mut out = Vec::new();
    for file in [crate::fs::rotated(&path), path] {
        match fs::read_to_string(&file) {
            Ok(text) => out.extend(text.lines().filter_map(|l| serde_json::from_str(l).ok())),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(CoreError::io(&file, e)),
        }
    }
    Ok(out)
}

/// [`read_runs`] folded by run, in the order each run first appears.
pub fn runs(home: &Path, workspace: &str) -> Result<Vec<Run>> {
    let mut out: Vec<Run> = Vec::new();
    for r in read_runs(home, workspace)? {
        let i = match out.iter().position(|x| x.run == r.run) {
            Some(i) => i,
            None => {
                out.push(Run { run: r.run.clone(), schedule: r.schedule.clone(), started: None, stages: Vec::new(), ended: None, tokens: None });
                out.len() - 1
            }
        };
        let run = &mut out[i];
        match r.event {
            RunEvent::Started { trigger, .. } => run.started = Some((r.ts, trigger)),
            RunEvent::Stage(stage) => run.stages.push(stage),
            RunEvent::Ended { outcome, why, tokens, .. } => {
                run.ended = Some((r.ts, outcome, why));
                run.tokens = tokens;
            }
        }
    }
    Ok(out)
}

/// Every `<folder>/.claude/worktrees/fm-sched-*` of the registered workspaces: the runs' worktrees still on disk.
pub fn run_worktrees(home: &Path) -> Vec<PathBuf> {
    let Ok((workspaces, _)) = Registry::at(home).load_all() else { return Vec::new() };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for folder in workspaces.iter().flat_map(|w| &w.folders) {
        let Ok(entries) = fs::read_dir(folder.path.join(".claude").join("worktrees")) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if e.file_name().to_string_lossy().starts_with("fm-sched-") && e.file_type().is_ok_and(|t| t.is_dir()) && seen.insert(crate::paths::path_key(&path)) {
                out.push(path);
            }
        }
    }
    out
}

/// The schedules' part of [`crate::prune::prune`]: rotated run records, the state dirs of removed schedules,
/// and run worktrees older than `cutoff` that hold nothing uncommitted and no commit only they have.
pub(crate) fn prune(home: &Path, cutoff: i64, errors: &mut Vec<String>) {
    let dir = schedules_dir(home);
    crate::prune::remove_older(&dir.join("runs"), cutoff, |n| n.ends_with(".jsonl.1"), errors);
    if let Ok(workspaces) = fs::read_dir(dir.join("state")) {
        for ws in workspaces.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
            let name = ws.file_name().to_string_lossy().into_owned();
            let Ok(kept) = load(home, &name) else { continue };
            let Ok(ids) = fs::read_dir(ws.path()) else { continue };
            for id in ids.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
                if !kept.iter().any(|s| *s.id == *id.file_name().to_string_lossy())
                    && let Err(e) = fs::remove_dir_all(id.path())
                {
                    errors.push(format!("Cannot remove {}: {e}", id.path().display()));
                }
            }
        }
    }
    for wt in run_worktrees(home) {
        let old = fs::metadata(&wt).is_ok_and(|m| crate::time::modified_ms(&m) < cutoff);
        if old && let Err(e) = remove_if_spent(&wt) {
            errors.push(format!("Cannot remove {}: {e}", wt.display()));
        }
    }
}

fn git(dir: &Path, args: &[&str]) -> std::result::Result<crate::process::Output, String> {
    crate::process::git(dir, args).map_err(|e| e.to_string())
}

/// `wt`'s main checkout and branch when it is a linked worktree, clean, with no commit only it reaches.
pub(super) fn spent(wt: &Path) -> std::result::Result<Option<(PathBuf, String)>, String> {
    let Some(main) = crate::paths::main_checkout(wt).filter(|m| crate::paths::path_key(m) != crate::paths::path_key(wt)) else { return Ok(None) };
    let status = git(wt, &["status", "--porcelain"])?;
    if !status.success() || !status.stdout.trim().is_empty() {
        return Ok(None);
    }
    let branch = git(wt, &["symbolic-ref", "-q", "--short", "HEAD"])?.stdout.trim().to_string();
    let mut only_here = vec!["rev-list", "--count", "HEAD", "--not"];
    let exclude = format!("--exclude={branch}");
    if !branch.is_empty() {
        only_here.push(&exclude);
    }
    only_here.extend(["--branches", "--remotes", "--tags"]);
    let unique = git(wt, &only_here)?;
    if !unique.success() || unique.stdout.trim() != "0" {
        return Ok(None);
    }
    Ok(Some((main, branch)))
}

/// Removes `wt` and its branch when it is [`spent`]; leaves it otherwise, for René to see.
fn remove_if_spent(wt: &Path) -> std::result::Result<(), String> {
    let Some((main, branch)) = spent(wt)? else { return Ok(()) };
    let wt_arg = wt.to_string_lossy();
    let removed = git(&main, &["worktree", "remove", &wt_arg])?;
    if !removed.success() {
        return Err(removed.stderr.trim().to_string());
    }
    if !branch.is_empty() {
        let deleted = git(&main, &["branch", "-D", &branch])?;
        if !deleted.success() {
            return Err(deleted.stderr.trim().to_string());
        }
    }
    Ok(())
}

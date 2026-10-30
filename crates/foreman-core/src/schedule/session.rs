use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone};

use super::{Permissions, Profile, Prompt};
use crate::{CoreError, Result};

/// `--disallowedTools` of every run: it cannot force-push, rm a session or schedule a task.
pub const DEFAULT_DENY: [&str; 5] = [
    "Bash(git push --force:*)",
    "Bash(claude rm:*)",
    "Bash(schtasks:*)",
    "PowerShell(Register-ScheduledTask:*)",
    "PowerShell(Unregister-ScheduledTask:*)",
];

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::WorktreeEdit => "worktree-edit",
            Self::Auto => "auto",
        }
    }

    /// claude's `--permission-mode`.
    pub fn permission_mode(self) -> &'static str {
        match self {
            Self::Read => "default",
            Self::WorktreeEdit => "acceptEdits",
            Self::Auto => "auto",
        }
    }

    pub fn tools(self) -> &'static [&'static str] {
        match self {
            Self::Read => &["Read", "Glob", "Grep"],
            Self::WorktreeEdit => &[
                "Read",
                "Glob",
                "Grep",
                "Edit",
                "Write",
                "Bash(git status:*)",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(git add:*)",
                "Bash(git commit:*)",
            ],
            Self::Auto => &[],
        }
    }
}

impl std::str::FromStr for Profile {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "read" => Ok(Self::Read),
            "worktree-edit" => Ok(Self::WorktreeEdit),
            "auto" => Ok(Self::Auto),
            "bypassPermissions" => Err("bypassPermissions is never a schedule's mode; the profiles are read, worktree-edit and auto".into()),
            other => Err(format!("'{other}' is not a profile: read, worktree-edit or auto")),
        }
    }
}

impl Permissions {
    /// `--allowedTools`: `always` (Foreman's own, `fm_allowed_tools`), the profile's, then the schedule's. A
    /// `worktree-edit` run may push its own `branch` and no other.
    pub fn allowed(&self, always: &[String], branch: &str) -> Vec<String> {
        let profile = self.profile.tools().iter().map(|t| t.to_string());
        let push = (self.profile == Profile::WorktreeEdit).then(|| format!("Bash(git push -u origin {branch})"));
        dedup(always.iter().cloned().chain(profile).chain(push).chain(self.allow_tools.iter().cloned()))
    }

    /// `--disallowedTools`: [`DEFAULT_DENY`], then the schedule's.
    pub fn denied(&self) -> Vec<String> {
        dedup(DEFAULT_DENY.iter().map(|t| t.to_string()).chain(self.deny_tools.iter().cloned()))
    }
}

fn dedup(tools: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tools {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// `yyyymmdd-hhmmss` of a run started at `at`.
pub fn run_stamp<Tz: TimeZone>(at: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.format("%Y%m%d-%H%M%S").to_string()
}

pub fn run_id(id: &str, stamp: &str) -> String {
    format!("{id}-{stamp}")
}

/// `fm-<workspace>-sched-<id>-<stamp>`; see docs/research/scheduler.md "The session name".
pub fn session_name(workspace: &str, id: &str, stamp: &str) -> String {
    format!("{}{workspace}-{}{id}-{stamp}", crate::FM_PREFIX, super::SCHED_SLUG_PREFIX)
}

/// `fm-sched/<id>/<stamp>`: the branch of a run's worktree.
pub fn run_branch(id: &str, stamp: &str) -> String {
    format!("{}sched/{id}/{stamp}", crate::FM_PREFIX)
}

/// `<folder>/.claude/worktrees/fm-sched-<id>-<stamp>`.
pub fn run_worktree(folder: &Path, id: &str, stamp: &str) -> PathBuf {
    folder.join(".claude").join("worktrees").join(format!("{}{}{id}-{stamp}", crate::FM_PREFIX, super::SCHED_SLUG_PREFIX))
}

/// What a run tells its session in front of the base prompt; `fm` is the command its `--allowedTools` lets run
/// (`fm_allowed_tools`).
pub fn preamble(run: &str, fm: &str) -> String {
    format!(
        "This is an unattended scheduled run `{run}`; nobody watches it. When you are finished, run \
         `{fm} schedule report {run} done \"<one line>\"`. If you cannot go on without the human, run \
         `{fm} schedule report {run} needs-you \"<why>\"`. Run `{fm}` with the Bash tool, exactly as written: \
         only that is allowed without a prompt."
    )
}

/// The base prompt in `folder`: the slash command as typed, the prompt file's text, or the text. A slash
/// command or prompt file that is not there is an error naming the folder.
pub fn base_prompt(folder: &Path, prompt: &Prompt) -> Result<String> {
    match prompt {
        Prompt::Command(name) => {
            let (shown, file) = super::command_file(folder, name);
            if !file.is_file() {
                return Err(CoreError::NoPrompt(format!("no slash command {name} in folder {}: {shown} does not exist", folder.display())));
            }
            Ok(name.clone())
        }
        Prompt::File(rel) => {
            let path = folder.join(rel);
            std::fs::read_to_string(&path)
                .map_err(|e| CoreError::NoPrompt(format!("cannot read the prompt file {rel} in folder {}: {e}", folder.display())))
        }
        Prompt::Text(text) => Ok(text.clone()),
    }
}

//! A folder's schedules at `<home>/schedules/<folder key>.toml`, their approval, next fires and run
//! records; the contract is `docs/research/scheduler.md`.

mod approval;
mod foreign;
mod live;
mod migrate;
mod next;
mod registrar;
mod runs;
mod session;
mod store;
mod task_xml;

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::validate_name;

pub use approval::{APPROVAL_KEY, Change, Covered, Refusal, approve, command_file, covered, covered_diff, covered_fields, effective_status, mac, verify, verify_approval};
pub use live::{FIRE_LOG_CAP, Live, LiveSession, MASK, Reported, STDERR_TAIL, fire_log, live_dir, masked_tail, secret_key, state_dir};
pub use next::{next_fire, next_fire_local, previous_fire};
pub use foreign::{Foreign, ForeignTask, NOT_RUN, SCRIPT_CAP, Via, is_own_root, names_claude, of_task, ole_date_ms, read_script, runs_claude, scan as scan_foreign, scan_with as scan_foreign_with, trigger_words};
pub use migrate::{Migrated, RESYNC, migrate};
pub use registrar::{SMW_EXE, FakeRegistrar, Listing, Registrar, TaskState, SyncTarget, Synced, prune_tasks, smw_exe, sync, sync_all, task_root, task_root_for};
pub use runs::{Fired, Outcome, RUNS_CAP, Run, RunEvent, RunRecord, Stage, Tokens, append_run, read_runs, runs, runs_path};
pub use session::{DEFAULT_DENY, base_prompt, preamble, run_branch, run_id, run_stamp, run_worktree, schedule_of_session, session_name};
pub use store::{FILE_VERSION, Scheduled, edit, file_path, folder_key, is_folder_key, load, load_all, load_key, schedules_dir};
pub use task_xml::{RunLevel, TaskSpec, definition_digest, task_xml, version_of};
pub(crate) use runs::prune as prune_schedules;
pub use runs::run_worktrees;

/// `fm-<folder key>-sched-<id>-<stamp>` names a scheduled run.
pub const SCHED_SLUG_PREFIX: &str = "sched-";
pub const MAX_ID_LEN: usize = 32;
pub const MIN_EVERY: Span = Span(15 * 60);
pub const MAX_RUNTIME: Span = Span(8 * 3600);
pub const MAX_SCRIPT_TIMEOUT: Span = Span(3600);
pub const MAX_TOKENS: u64 = 20_000_000;
pub const SCRIPT_EXTENSIONS: [&str; 4] = ["ps1", "cmd", "bat", "exe"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleFile {
    #[serde(default = "version_one")]
    pub version: u32,
    /// The folder the schedules run in; the file is named by its [`folder_key`].
    #[serde(default)]
    pub folder: PathBuf,
    #[serde(default)]
    pub schedule: Vec<Schedule>,
}

impl Default for ScheduleFile {
    fn default() -> Self {
        Self { version: FILE_VERSION, folder: PathBuf::new(), schedule: Vec::new() }
    }
}

fn version_one() -> u32 {
    1
}

// Plain keys first: TOML emits the tables after them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub created_by: CreatedBy,
    #[serde(default)]
    pub created_at: i64,
    pub trigger: Trigger,
    pub action: Action,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub caps: Caps,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Pending,
    Active,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreatedBy {
    #[default]
    Fm,
    App,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Trigger {
    Weekly { days: Vec<Day>, at: Clock },
    Daily { at: Clock },
    /// From `at` every `every` while within the day that starts at `at`.
    Interval {
        #[serde(default)]
        at: Clock,
        every: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    pub const ALL: [Day; 7] = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri, Day::Sat, Day::Sun];

    pub fn weekday(self) -> chrono::Weekday {
        use chrono::Weekday::*;
        [Mon, Tue, Wed, Thu, Fri, Sat, Sun][self as usize]
    }

    /// `mon`..`sun`, as the TOML writes it.
    pub fn as_str(self) -> &'static str {
        ["mon", "tue", "wed", "thu", "fri", "sat", "sun"][self as usize]
    }
}

/// `weekly mon,fri 09:00`, `daily 09:00` or `interval 2h from 08:00`: the form `sm schedule add --every` takes.
impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trigger::Weekly { days, at } => write!(f, "weekly {} {at}", days.iter().map(|d| d.as_str()).collect::<Vec<_>>().join(",")),
            Trigger::Daily { at } => write!(f, "daily {at}"),
            Trigger::Interval { at, every } => write!(f, "interval {every} from {at}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub kind: ActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Prompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default)]
    pub script_args: Vec<String>,
    #[serde(default = "default_script_timeout")]
    pub script_timeout: Span,
    /// A run in a Supraman run worktree, set only by an earlier release; a covered edit clears it.
    #[serde(default)]
    pub worktree: bool,
    #[serde(default)]
    pub model: String,
}

fn default_script_timeout() -> Span {
    Span(10 * 60)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Session,
    ScriptThenSession,
    Script,
}

/// `{ command = "/x" }`, `{ file = "prompts/x.md" }` (relative to the folder) or `{ text = "…" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Prompt {
    Command(String),
    File(String),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Permissions {
    #[serde(default)]
    pub profile: Profile,
    #[serde(default)]
    pub allow_tools: Vec<String>,
    #[serde(default)]
    pub deny_tools: Vec<String>,
    #[serde(default)]
    pub add_dirs: Vec<String>,
    /// Names only; each value is in the `SecretStore` under [`secret_key`].
    #[serde(default)]
    pub secrets: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    #[default]
    Read,
    #[serde(alias = "worktree-edit")]
    Edit,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caps {
    #[serde(default = "default_max_runtime")]
    pub max_runtime: Span,
    /// Input + output + cache-creation tokens of the run's transcript.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
}

impl Default for Caps {
    fn default() -> Self {
        Self { max_runtime: default_max_runtime(), max_tokens: default_max_tokens() }
    }
}

fn default_max_runtime() -> Span {
    Span(3600)
}

fn default_max_tokens() -> u64 {
    3_000_000
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub overlap: Overlap,
    #[serde(default)]
    pub catch_up: CatchUp,
    #[serde(default)]
    pub wake: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Overlap {
    #[default]
    Skip,
    Queue,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatchUp {
    #[default]
    Once,
    Skip,
}

/// Written only by [`approve`] (C100).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub fingerprint: String,
    pub mac: String,
    pub approved_at: i64,
    /// The covered JSON as approved: what Approve diffs against.
    pub covered: String,
    /// Shown as "changed since approval" when it no longer matches; never refuses a fire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_sha256: Option<String>,
}

/// A local wall-clock `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Clock {
    pub hour: u32,
    pub minute: u32,
}

impl Clock {
    pub fn new(hour: u32, minute: u32) -> Option<Self> {
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }

    pub fn time(self) -> chrono::NaiveTime {
        chrono::NaiveTime::from_hms_opt(self.hour, self.minute, 0).expect("a checked clock")
    }
}

impl std::str::FromStr for Clock {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let bad = || format!("'{s}' is not a time of day HH:MM");
        let (h, m) = s.split_once(':').ok_or_else(bad)?;
        if h.is_empty() || h.len() > 2 || m.len() != 2 || !(h.bytes().chain(m.bytes())).all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        Clock::new(h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?).ok_or_else(bad)
    }
}

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl Serialize for Clock {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Clock {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Whole seconds written `30s`, `10m` or `2h`; saved in the largest unit that divides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span(pub u64);

impl Span {
    pub fn duration(self) -> Duration {
        Duration::from_secs(self.0)
    }
}

impl std::str::FromStr for Span {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let bad = || format!("'{s}' is not a duration such as 30s, 10m or 2h");
        let unit = match s.as_bytes().last() {
            Some(b's') => 1,
            Some(b'm') => 60,
            Some(b'h') => 3600,
            _ => return Err(bad()),
        };
        let n = &s[..s.len() - 1];
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        n.parse::<u64>().ok().and_then(|n| n.checked_mul(unit)).map(Span).ok_or_else(bad)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            s if s > 0 && s % 3600 == 0 => write!(f, "{}h", s / 3600),
            s if s > 0 && s % 60 == 0 => write!(f, "{}m", s / 60),
            s => write!(f, "{s}s"),
        }
    }
}

impl Serialize for Span {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Span {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

impl Schedule {
    /// What `sm schedule add` and the app check before a save.
    pub fn validate(&self) -> Result<(), String> {
        validate_name(&self.id).map_err(|_| "the id takes lowercase letters, digits and '-' only".to_string())?;
        if self.id.len() > MAX_ID_LEN {
            return Err(format!("the id is longer than {MAX_ID_LEN} characters"));
        }
        match &self.trigger {
            Trigger::Weekly { days, .. } if days.is_empty() => return Err("a weekly trigger needs at least one day".into()),
            Trigger::Interval { every, .. } if *every < MIN_EVERY => return Err(format!("an interval is at least {MIN_EVERY}")),
            _ => {}
        }
        let a = &self.action;
        let scripted = a.kind != ActionKind::Session;
        match (&a.script, scripted) {
            (None, true) => return Err("this action needs a script".into()),
            (Some(_), false) => return Err("a session action takes no script".into()),
            (Some(script), true) => {
                relative(script)?;
                let ext = Path::new(script).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                if !SCRIPT_EXTENSIONS.contains(&ext.as_str()) {
                    return Err(format!("a script is .ps1, .cmd, .bat or .exe, not '{script}'"));
                }
            }
            (None, false) => {}
        }
        match (&a.prompt, a.kind == ActionKind::Script) {
            (Some(_), true) => return Err("a script action takes no prompt".into()),
            (None, false) => return Err("this action needs a prompt".into()),
            (Some(Prompt::Command(c)), false) if !c.starts_with('/') || c.len() < 2 || c.contains(['\\', '.']) => {
                return Err(format!("'{c}' is not a slash command"));
            }
            (Some(Prompt::File(file)), false) => relative(file)?,
            _ => {}
        }
        if a.script_timeout.0 == 0 || a.script_timeout > MAX_SCRIPT_TIMEOUT {
            return Err(format!("script_timeout is between 1s and {MAX_SCRIPT_TIMEOUT}"));
        }
        let p = &self.permissions;
        if let Some(bad) = p.allow_tools.iter().chain(&p.deny_tools).find(|t| t.trim().is_empty() || t.starts_with('-')) {
            return Err(format!("'{bad}' is not a tool rule"));
        }
        if let Some(bad) = p.add_dirs.iter().find(|d| d.trim().is_empty() || d.starts_with('-')) {
            return Err(format!("'{bad}' is not a directory"));
        }
        if !a.model.is_empty() && (a.model.starts_with('-') || a.model.contains(char::is_whitespace)) {
            return Err(format!("'{}' is not a model", a.model));
        }
        if let Some(bad) = self.permissions.secrets.iter().find(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')) {
            return Err(format!("secret name '{bad}' takes A-Z, 0-9 and '_' only"));
        }
        if self.caps.max_runtime.0 == 0 || self.caps.max_runtime > MAX_RUNTIME {
            return Err(format!("max_runtime is between 1s and {MAX_RUNTIME}"));
        }
        if self.caps.max_tokens == 0 || self.caps.max_tokens > MAX_TOKENS {
            return Err(format!("max_tokens is between 1 and {MAX_TOKENS}"));
        }
        Ok(())
    }
}

fn relative(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if path.is_empty() || !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("'{path}' is not a path inside the folder"));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;

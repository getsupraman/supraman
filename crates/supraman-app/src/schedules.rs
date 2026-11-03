//! The Schedules group of the tree: what each schedule shows, read off the UI thread with each poll.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use supraman_core::Workspace;
use supraman_core::schedule::{self, ActionKind, CatchUp, Change, Covered, Day, Fired, Foreign, ForeignTask, Live, Outcome, Overlap, Policy, Profile, Prompt, Run, Schedule, Span, Stage, Status, Trigger};
use supraman_core::secret::SecretStore;
use supraman_core::time::{self, date_words};
use supraman_ui::details::Section;

/// A row keeps at most this many runs.
pub const HISTORY: usize = 30;
/// The Details page lists this many.
const RUNS_SHOWN: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleRow {
    pub workspace: String,
    pub schedule: Schedule,
    /// [`schedule::effective_status`].
    pub status: Status,
    /// Active, but the slash command file no longer has the approved hash.
    pub command_changed: bool,
    pub next_fire: Option<i64>,
    pub running: Option<Live>,
    /// Newest first.
    pub runs: Vec<Run>,
    /// While pending or `command_changed`: what Approve approves, or why it cannot.
    pub approvable: Option<std::result::Result<Covered, String>>,
    pub folder_path: Option<PathBuf>,
    /// The slash command's file as `schedule::command_file` shows it.
    pub command_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primary {
    Approve,
    RunNow,
}

/// The last result as a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Result {
    Done,
    NothingToDo,
    Failed,
    NeedsYou,
    Skipped,
}

impl Result {
    pub fn of(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Done | Outcome::Ok => Self::Done,
            Outcome::NothingToDo => Self::NothingToDo,
            Outcome::NeedsYou => Self::NeedsYou,
            Outcome::Skipped => Self::Skipped,
            Outcome::Failed | Outcome::Refused | Outcome::CappedRuntime | Outcome::CappedTokens | Outcome::Interrupted => Self::Failed,
        }
    }
}

pub fn outcome_text(o: Outcome) -> &'static str {
    match o {
        Outcome::NothingToDo => "nothing to do",
        Outcome::Ok => "ok",
        Outcome::Done => "done",
        Outcome::NeedsYou => "needs you",
        Outcome::Failed => "failed",
        Outcome::Refused => "refused",
        Outcome::Skipped => "skipped",
        Outcome::CappedRuntime => "stopped at max runtime",
        Outcome::CappedTokens => "stopped at max tokens",
        Outcome::Interrupted => "interrupted",
    }
}

impl ScheduleRow {
    pub fn title(&self) -> &str {
        if self.schedule.name.is_empty() { &self.schedule.id } else { &self.schedule.name }
    }

    pub fn status_text(&self) -> &'static str {
        match self.status {
            Status::Pending => "pending",
            Status::Paused => "paused",
            Status::Active if self.command_changed => "changed since approval",
            Status::Active => "active",
        }
    }

    pub fn primary(&self) -> Primary {
        if self.status == Status::Pending { Primary::Approve } else { Primary::RunNow }
    }

    /// The latest run that ended.
    pub fn last(&self) -> Option<(&Run, Outcome)> {
        self.runs.iter().find_map(|r| r.ended.as_ref().map(|(_, o, _)| (r, *o)))
    }

    pub fn last_result(&self) -> Option<Result> {
        self.last().map(|(_, o)| Result::of(o))
    }
}

fn duration_text(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min {} s", s / 60, s % 60),
        _ => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

fn tokens_text(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{}k", n / 1_000),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// Every workspace's schedules, its errors as lines. Rows of a workspace whose file cannot be read are left out.
pub fn read(home: &Path, workspaces: &[Workspace], store: &dyn SecretStore, now_ms: i64) -> (BTreeMap<String, Vec<ScheduleRow>>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    for w in workspaces {
        let schedules = match schedule::load(home, &w.name) {
            Ok(s) if s.is_empty() => continue,
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("Cannot read the schedules of {}: {e}", w.name));
                continue;
            }
        };
        let mut runs = schedule::runs(home, &w.name).unwrap_or_else(|e| {
            errors.push(format!("Cannot read the run records of {}: {e}", w.name));
            Vec::new()
        });
        runs.reverse();
        let live = schedule::live_dir(home, &w.name).ok();
        let rows = schedules
            .into_iter()
            .map(|s| {
                let (status, command_changed) = match s.status {
                    Status::Active => match schedule::verify_approval(store, w, &s) {
                        Ok(now) => (Status::Active, s.approval.as_ref().is_some_and(|a| a.command_sha256 != now.command_sha256)),
                        Err(_) => (Status::Pending, false),
                    },
                    other => (other, false),
                };
                let approvable = (status == Status::Pending || command_changed).then(|| {
                    s.validate(w).map_err(|why| format!("it cannot be approved: {why}"))?;
                    schedule::covered(w, &s).map_err(|e| e.to_string())
                });
                let folder_path = w.folder(&s.folder).map(|f| f.path.clone());
                let command_path = match (&s.action.prompt, &folder_path) {
                    (Some(schedule::Prompt::Command(name)), Some(folder)) => Some(schedule::command_file(folder, name).0),
                    _ => None,
                };
                let running = live
                    .as_ref()
                    .and_then(|d| std::fs::read(d.join(format!("{}.json", s.id))).ok())
                    .and_then(|b| serde_json::from_slice::<Live>(&b).ok());
                ScheduleRow {
                    workspace: w.name.clone(),
                    next_fire: (status == Status::Active).then(|| schedule::next_fire_local(&s.trigger, now_ms)).flatten(),
                    runs: runs.iter().filter(|r| r.schedule == s.id).take(HISTORY).cloned().collect(),
                    running,
                    status,
                    command_changed,
                    approvable,
                    folder_path,
                    command_path,
                    schedule: s,
                }
            })
            .collect();
        out.insert(w.name.clone(), rows);
    }
    (out, errors)
}

/// Each live file of `workspaces` whose sm is gone, recovered (`sm::fire::recover_if_dead`); the runs recovered.
pub fn recover(home: &Path, workspaces: &[Workspace]) -> Vec<String> {
    let mut out = Vec::new();
    for w in workspaces {
        let Some(entries) = schedule::live_dir(home, &w.name).ok().and_then(|d| std::fs::read_dir(d).ok()) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Some(id) = path.file_stem().and_then(|x| x.to_str())
                && let Some(run) = sm::fire::recover_if_dead(home, &w.name, id)
            {
                out.push(format!("{}/{run}", w.name));
            }
        }
    }
    out
}

fn session_of(run: &Run) -> Option<&str> {
    run.stages.iter().find_map(|s| match s {
        Stage::Session { name, .. } => Some(name.as_str()),
        _ => None,
    })
}

/// `Monday`, `Monday and Friday`, `Monday, Wednesday and Friday`; all seven read `day`.
fn days_words(days: &[Day]) -> String {
    if days.len() == 7 {
        return "day".into();
    }
    let names: Vec<&str> = days.iter().map(|d| ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][*d as usize]).collect();
    match names.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => "no day".into(),
    }
}

/// `2 hours`, `30 minutes`, `1 hour 30 minutes`, `45 seconds`.
pub fn span_words(span: Span) -> String {
    let plural = |n: u64, unit: &str| if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") };
    let (h, m, s) = (span.0 / 3600, span.0 % 3600 / 60, span.0 % 60);
    let parts: Vec<String> = [(h, "hour"), (m, "minute"), (s, "second")].into_iter().filter(|(n, _)| *n > 0).map(|(n, u)| plural(n, u)).collect();
    if parts.is_empty() { "0 seconds".into() } else { parts.join(" ") }
}

/// `Every Monday at 08:00`, `Every day at 09:00`, `Every 2 hours from 08:00, through the day`.
pub fn trigger_words(t: &Trigger) -> String {
    match t {
        Trigger::Weekly { days, at } => format!("Every {} at {at}", days_words(days)),
        Trigger::Daily { at } => format!("Every day at {at}"),
        Trigger::Interval { at, every } => format!("Every {} from {at}, through the day", span_words(*every)),
    }
}

fn policy_words(p: &Policy) -> String {
    let catch_up = match p.catch_up {
        CatchUp::Once => "A fire missed while the computer was off or asleep runs once when it is back",
        CatchUp::Skip => "A fire missed while the computer was off or asleep is skipped",
    };
    let overlap = match p.overlap {
        Overlap::Skip => "a fire while the last run still works is skipped",
        Overlap::Queue => "a fire while the last run still works waits for it",
        Overlap::Restart => "a fire while the last run still works stops it and starts again",
    };
    let wake = if p.wake { "It wakes the computer to run." } else { "It does not wake the computer." };
    format!("{catch_up}; {overlap}. {wake}")
}

pub fn profile_words(p: Profile) -> &'static str {
    match p {
        Profile::Read => "Reads and searches the folder; it changes nothing.",
        Profile::Edit => "Reads and edits files and commits with git in the folder's checkout; it does not push.",
        Profile::Auto => "Claude's auto mode: it decides what to run, except what the denied rules forbid.",
    }
}

fn fired_words(f: Fired) -> &'static str {
    match f {
        Fired::Task => "on schedule",
        Fired::RunNow => "Run now",
        Fired::CatchUp => "caught up",
    }
}

fn tokens_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A covered field by its meaning; `files.<path>` reads `File <path>`.
fn field_words(field: &str) -> String {
    if let Some(file) = field.strip_prefix("files.") {
        return format!("File {file}");
    }
    match field {
        "v" => "Format",
        "workspace" => "Workspace",
        "id" => "Id",
        "folder" => "Folder",
        "trigger.kind" => "Trigger",
        "trigger.at" => "Trigger time",
        "trigger.days" => "Trigger days",
        "trigger.every" => "Trigger interval",
        "action.kind" => "Action",
        "action.prompt.command.name" => "Slash command",
        "action.prompt.command.path" => "Slash command file",
        "action.prompt.file" => "Prompt file",
        "action.prompt.text" => "Prompt",
        "action.script" => "Script",
        "action.script_args" => "Script arguments",
        "action.script_timeout" => "Script timeout",
        "action.worktree" => "Where it runs",
        "action.model" => "Model",
        "permissions.profile" => "Profile",
        "permissions.allow_tools" => "Allowed rules",
        "permissions.deny_tools" => "Denied rules",
        "permissions.add_dirs" => "Extra folders",
        "permissions.secrets" => "Secrets",
        "caps.max_runtime" => "Max runtime",
        "caps.max_tokens" => "Max tokens",
        "policy.overlap" => "While a run still works",
        "policy.catch_up" => "Missed fires",
        "policy.wake" => "Wakes the computer",
        other => other,
    }
    .to_string()
}

/// A covered value as the page shows it: `worktree-edit` is `edit` (sched2/F-03).
fn value_words(field: &str, value: &str) -> String {
    if field == "permissions.profile" && value == "worktree-edit" {
        return "edit".into();
    }
    if value.is_empty() || value == "[]" || value == "none" {
        return "none".into();
    }
    value.to_string()
}

/// One covered change in words, `Trigger time: 09:00 → 10:30`; none when only its spelling differs.
pub fn change_words(c: &Change) -> Option<String> {
    if c.field == "action.worktree" {
        return Some(match c.after.as_deref() {
            Some("false") | None => "Runs in the folder's checkout instead of its own worktree".into(),
            _ => "Runs in its own worktree instead of the folder's checkout".into(),
        });
    }
    let name = field_words(&c.field);
    if c.field.starts_with("files.") {
        return Some(match (&c.before, &c.after) {
            (Some(_), Some(_)) => format!("{name}: its content changed"),
            (None, Some(_)) => format!("{name}: newly covered"),
            (Some(_), None) => format!("{name}: no longer covered"),
            (None, None) => name,
        });
    }
    let before = c.before.as_deref().map(|v| value_words(&c.field, v));
    let after = c.after.as_deref().map(|v| value_words(&c.field, v));
    match (before, after) {
        (Some(b), Some(a)) if b == a => None,
        (Some(b), Some(a)) => Some(format!("{name}: {b} → {a}")),
        (None, Some(a)) => Some(format!("{name}: {a} (new)")),
        (Some(b), None) => Some(format!("{name}: {b} (gone)")),
        (None, None) => Some(name),
    }
}

/// The local zone's name and offset, as the When section shows it.
pub fn zone_words(now_ms: i64) -> String {
    let offset = time::local(now_ms).map(|t| t.format("UTC%:z").to_string()).unwrap_or_default();
    match iana_time_zone::get_timezone() {
        Ok(name) => format!("This computer's time ({name}, {offset})"),
        Err(_) => format!("This computer's time ({offset})"),
    }
}

/// The status as the page's pill says it, and whether it asks for René.
pub fn status_words(row: &ScheduleRow) -> (&'static str, bool) {
    match (row.running.is_some(), row.status, row.command_changed) {
        (true, _, _) => ("Running", false),
        (_, Status::Pending, _) => ("Waits for your approval", true),
        (_, Status::Paused, _) => ("Paused", false),
        (_, Status::Active, true) => ("Active · its slash command changed", true),
        (_, Status::Active, false) => ("Active", false),
    }
}

/// The Details page's sections, in words; `dry_run` is the last Dry run's output.
pub fn sections(row: &ScheduleRow, zone: &str, dry_run: Option<&str>, now_ms: i64) -> Vec<Section> {
    let s = &row.schedule;
    let mut out = Vec::new();
    if !s.description.is_empty() {
        out.push(Section::new("about", "About").text(s.description.clone()));
    }

    let mut fires = Vec::new();
    let mut after = now_ms;
    while fires.len() < 3 {
        let Some(next) = schedule::next_fire_local(&s.trigger, after) else { break };
        fires.push(date_words(next));
        after = next;
    }
    let next_label = if row.status == Status::Active { "Next" } else { "Next, once active" };
    out.push(
        Section::new("when", "When")
            .row("Runs", trigger_words(&s.trigger))
            .row("Zone", zone.to_string())
            .row(next_label, if fires.is_empty() { "none".into() } else { fires.join("\n") })
            .text(policy_words(&s.policy)),
    );

    let folder = match &row.folder_path {
        Some(p) => format!("{} ({})", s.folder, p.display()),
        None => format!("{} (not a folder of {})", s.folder, row.workspace),
    };
    let checkout = if s.action.worktree {
        "In its own worktree (saved by an earlier release; the next change moves it to the checkout)"
    } else {
        "In the folder's checkout"
    };
    out.push(Section::new("where", "Where").row("Workspace", row.workspace.clone()).row("Folder", folder).text(checkout));

    let a = &s.action;
    let mut what = Section::new("what", "What").text(match a.kind {
        ActionKind::Session => "Starts a Claude session with the prompt below.",
        ActionKind::Script => "Runs the script below; no Claude session.",
        ActionKind::ScriptThenSession => "Runs the script below; when it exits 10, a Claude session follows with the prompt and the script's output.",
    });
    if let Some(script) = &a.script {
        let args = if a.script_args.is_empty() { String::new() } else { format!(" {}", a.script_args.join(" ")) };
        what = what.row("Script", format!("{script}{args}")).row("Script timeout", span_words(a.script_timeout));
    }
    match &a.prompt {
        Some(Prompt::Command(name)) => {
            what = what.row("Slash command", name.clone());
            if let Some(path) = &row.command_path {
                what = what.row("Its file", path.clone());
            }
        }
        Some(Prompt::File(file)) => what = what.row("Prompt file", file.clone()),
        Some(Prompt::Text(text)) => what = what.long(text.clone()),
        None => {}
    }
    if !a.model.is_empty() {
        what = what.row("Model", a.model.clone());
    }
    out.push(what);

    let p = &s.permissions;
    let listed = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join("\n") };
    let mut may = Section::new("may", "May")
        .row("Profile", p.profile.as_str())
        .text(profile_words(p.profile))
        .row("Also allowed", listed(&p.allow_tools))
        .row("Denied", p.denied().join("\n"));
    if !p.add_dirs.is_empty() {
        may = may.row("Extra folders", p.add_dirs.join("\n"));
    }
    out.push(may.row("Secrets", listed(&p.secrets)));

    out.push(Section::new("stops", "Stops at").row("Max runtime", span_words(s.caps.max_runtime)).row("Max tokens", tokens_count(s.caps.max_tokens)));

    match (&row.approvable, &s.approval) {
        (Some(Err(why)), _) => out.push(Section::new("changed", "Cannot be approved").text(why.clone())),
        (Some(Ok(now)), Some(approval)) => {
            let mut changed = Section::new("changed", "Changed since your last approval").row("Approved", date_words(approval.approved_at));
            let lines: Vec<String> = schedule::covered_diff(&approval.covered, &now.json).iter().filter_map(change_words).collect();
            if lines.is_empty() {
                changed = changed.text("Nothing it covers changed.");
            }
            for line in lines {
                changed = changed.text(line);
            }
            if approval.command_sha256 != now.command_sha256 {
                changed = changed
                    .row("Slash command", row.command_path.clone().unwrap_or_default())
                    .row("Approved file", sha_words(approval.command_sha256.as_deref()))
                    .row("File now", sha_words(now.command_sha256.as_deref()))
                    .text("Its runs read the file as it is now. Approve makes this file the approved one.");
            }
            out.push(changed);
        }
        (Some(Ok(_)), None) => out.push(Section::new("changed", "Approval").text("Never approved. Approve lets its task run it as this page shows it.")),
        (None, _) => {}
    }

    let mut runs = Section::new("runs", "Last runs");
    if row.runs.is_empty() {
        runs = runs.text("No run yet.");
    }
    for r in row.runs.iter().take(RUNS_SHOWN) {
        let start = r.started.map_or("not started".to_string(), |(ts, _)| time::near(ts, now_ms));
        let outcome = match &r.ended {
            Some((_, o, why)) if why.is_empty() => outcome_text(*o).to_string(),
            Some((_, o, why)) => format!("{}: {why}", outcome_text(*o)),
            None if row.running.as_ref().is_some_and(|l| l.run == r.run) => "running".into(),
            None => "no end recorded".into(),
        };
        let mut value = match r.started {
            Some((_, f)) => format!("{} · {outcome}", fired_words(f)),
            None => outcome,
        };
        if let (Some((start, _)), Some((end, _, _))) = (r.started, &r.ended) {
            value.push_str(&format!(" · {}", duration_text(end - start)));
        }
        if let Some(tokens) = &r.tokens {
            value.push_str(&format!(" · {} tokens", tokens_text(tokens.capped())));
        }
        if let Some(name) = session_of(r) {
            value.push_str(&format!(" · {name}"));
        }
        runs = runs.row(start, value);
    }
    out.push(runs);

    if let Some(text) = dry_run {
        out.push(Section::new("dry-run", "Dry run").long(text.trim_end().to_string()));
    }
    out
}

/// A slash command file's sha256 as the page shows it: its first 12 digits, or `missing`.
fn sha_words(sha: Option<&str>) -> String {
    sha.map_or_else(|| "missing".to_string(), |s| format!("sha256 {}…", &s[..s.len().min(12)]))
}

/// The Inbox entry of a pending schedule, or of an active one whose slash command changed; `key` changes when
/// what it covers, or that command file, changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub workspace: String,
    pub id: String,
    pub title: String,
    pub key: String,
    pub command_changed: bool,
}

pub fn waiting(rows: &BTreeMap<String, Vec<ScheduleRow>>) -> Vec<Waiting> {
    rows.values()
        .flatten()
        .filter(|row| row.status == Status::Pending || row.command_changed)
        .map(|row| {
            let state = match &row.approvable {
                Some(Ok(c)) if row.command_changed => format!("command-{}", c.command_sha256.as_deref().unwrap_or("missing")),
                Some(Ok(c)) => c.fingerprint.clone(),
                Some(Err(e)) => e.clone(),
                None => String::new(),
            };
            Waiting {
                workspace: row.workspace.clone(),
                id: row.schedule.id.clone(),
                title: row.title().to_string(),
                key: format!("{}/{}@{state}", row.workspace, row.schedule.id),
                command_changed: row.command_changed,
            }
        })
        .collect()
}

/// Every workspace with schedules, and the keys of its pending ones: what a dismissal of a waiting entry is kept against.
pub fn waiting_keys(rows: &BTreeMap<String, Vec<ScheduleRow>>) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = rows.keys().map(|w| (w.clone(), Vec::new())).collect();
    for w in waiting(rows) {
        out.entry(w.workspace).or_default().push(w.key);
    }
    out
}

/// A schedule's latest run, when it ended in a way that needs René: the Inbox and the tray show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attention {
    pub workspace: String,
    pub id: String,
    pub run: String,
    pub outcome: Outcome,
    pub why: String,
    pub at: i64,
}

pub fn attention(rows: &BTreeMap<String, Vec<ScheduleRow>>) -> Vec<Attention> {
    rows.values()
        .flatten()
        .filter_map(|row| {
            let (run, outcome) = row.last()?;
            let (at, _, why) = run.ended.as_ref()?;
            outcome.needs_attention().then(|| Attention {
                workspace: row.workspace.clone(),
                id: row.schedule.id.clone(),
                run: run.run.clone(),
                outcome,
                why: why.clone(),
                at: *at,
            })
        })
        .collect()
}

/// The tray's tooltip: Supraman, how many runs need René and how many schedules wait for his approval.
pub fn tray_tooltip(attention: usize, waiting: usize) -> String {
    let mut out = "Supraman".to_string();
    match attention {
        0 => {}
        1 => out.push_str(" · 1 scheduled run needs you"),
        n => out.push_str(&format!(" · {n} scheduled runs need you")),
    }
    match waiting {
        0 => {}
        1 => out.push_str(" · 1 schedule waits for your approval"),
        n => out.push_str(&format!(" · {n} schedules wait for your approval")),
    }
    out
}

/// Task Scheduler's tasks that run claude outside Supraman: under the workspace that holds them
/// ([`schedule::owner`]), the rest apart. Supraman never changes, runs or deletes them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Others {
    pub by_workspace: BTreeMap<String, Vec<ForeignTask>>,
    pub unowned: Vec<ForeignTask>,
}

impl Others {
    pub fn of(found: &Foreign, workspaces: &[Workspace]) -> Self {
        let mut out = Self::default();
        for t in &found.tasks {
            match schedule::owner(t, workspaces) {
                Some(w) => out.by_workspace.entry(w.name.clone()).or_default().push(t.clone()),
                None => out.unowned.push(t.clone()),
            }
        }
        out
    }

    pub fn of_workspace(&self, workspace: &str) -> &[ForeignTask] {
        self.by_workspace.get(workspace).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn find(&self, path: &str) -> Option<&ForeignTask> {
        self.by_workspace.values().flatten().chain(&self.unowned).find(|t| t.path() == path)
    }
}

/// An other schedule's row after its name: `daily at 09:00 · next Mon 09:00 · last OK`.
pub fn other_summary(t: &ForeignTask, now_ms: i64) -> String {
    let mut parts = vec![t.trigger.clone()];
    if !t.enabled {
        parts.push("disabled".into());
    }
    if let Some(next) = t.next_run_ms {
        parts.push(format!("next {}", time::near(next, now_ms)));
    }
    match t.result_text() {
        Some(r) => parts.push(format!("last {r}")),
        None if t.last_run_ms.is_none() => parts.push("never ran".into()),
        None => {}
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use supraman_core::schedule::{Approval, Fired, Tokens};

    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn schedule() -> Schedule {
        serde_json::from_value(serde_json::json!({
            "id": "nightly", "name": "Nightly gate", "folder": "Alpha",
            "trigger": {"kind": "daily", "at": "09:00"}, "action": {"kind": "script_then_session", "script": "gate.cmd", "prompt": {"text": "fix"}}
        }))
        .unwrap()
    }

    fn run(id: &str, at: i64, outcome: Option<Outcome>, why: &str) -> Run {
        Run {
            run: id.into(),
            schedule: "nightly".into(),
            started: Some((at, Fired::Task)),
            stages: vec![],
            ended: outcome.map(|o| (at + 65_000, o, why.into())),
            tokens: None,
        }
    }

    fn row(status: Status, runs: Vec<Run>) -> ScheduleRow {
        ScheduleRow {
            workspace: "demo".into(),
            schedule: schedule(),
            status,
            command_changed: false,
            next_fire: None,
            running: None,
            runs,
            approvable: None,
            folder_path: None,
            command_path: None,
        }
    }

    #[test]
    fn a_row_shows_its_status_its_one_action_and_its_last_result() {
        let pending = row(Status::Pending, vec![]);
        assert_eq!((pending.status_text(), pending.primary(), pending.last_result()), ("pending", Primary::Approve, None));
        let mut active = row(Status::Active, vec![run("r2", NOW, None, ""), run("r1", NOW - 1, Some(Outcome::NothingToDo), "")]);
        assert_eq!((active.status_text(), active.primary()), ("active", Primary::RunNow));
        assert_eq!(active.last_result(), Some(Result::NothingToDo), "the newest run that ended; one in flight has none");
        active.command_changed = true;
        assert_eq!(active.status_text(), "changed since approval");
        assert_eq!(active.primary(), Primary::RunNow, "it still runs (Decision Q2)");
        assert_eq!(row(Status::Paused, vec![]).status_text(), "paused");
        assert_eq!(pending.title(), "Nightly gate");
        for (o, r) in [
            (Outcome::Done, Result::Done),
            (Outcome::Ok, Result::Done),
            (Outcome::NeedsYou, Result::NeedsYou),
            (Outcome::Skipped, Result::Skipped),
            (Outcome::Failed, Result::Failed),
            (Outcome::Refused, Result::Failed),
            (Outcome::Interrupted, Result::Failed),
            (Outcome::CappedTokens, Result::Failed),
        ] {
            assert_eq!(Result::of(o), r, "{o:?}");
        }
    }

    fn page(row: &ScheduleRow) -> String {
        supraman_ui::details::text("t", &sections(row, "zone", None, NOW))
    }

    #[test]
    fn the_page_says_the_schedule_in_words_and_never_worktree_edit() {
        let mut r = row(Status::Pending, vec![]);
        r.schedule.trigger = Trigger::Weekly { days: vec![Day::Mon], at: "08:00".parse().unwrap() };
        r.schedule.permissions.profile = Profile::Edit;
        r.approvable = Some(Ok(Covered { json: "{}".into(), fingerprint: "f".into(), command_sha256: None }));
        let text = page(&r);
        for words in [
            "Runs  Every Monday at 08:00",
            "Zone  zone",
            "Next, once active  ",
            "runs once when it is back; a fire while the last run still works is skipped. It does not wake the computer.",
            "Folder  Alpha (not a folder of demo)",
            "In the folder's checkout",
            "Script  gate.cmd",
            "Script timeout  10 minutes",
            "\nfix\n",
            "Profile  edit",
            "it does not push.",
            "Denied  Bash(git push --force:*)",
            "Max runtime  1 hour",
            "Max tokens  3,000,000",
            "Never approved.",
            "No run yet.",
        ] {
            assert!(text.contains(words), "{words:?} not in\n{text}");
        }
        assert!(!text.contains("worktree-edit") && !text.contains("trigger.") && !text.contains("caps."), "{text}");
        let next = text.lines().skip_while(|l| !l.starts_with("Next")).take(3).count();
        assert_eq!(next, 3, "three fires");
        assert_eq!(trigger_words(&Trigger::Weekly { days: vec![Day::Mon, Day::Wed, Day::Fri], at: "09:30".parse().unwrap() }), "Every Monday, Wednesday and Friday at 09:30");
        assert_eq!(trigger_words(&Trigger::Interval { at: "08:00".parse().unwrap(), every: Span(5400) }), "Every 1 hour 30 minutes from 08:00, through the day");
        assert_eq!(trigger_words(&Trigger::Weekly { days: Day::ALL.to_vec(), at: "07:00".parse().unwrap() }), "Every day at 07:00");
    }

    #[test]
    fn the_change_since_the_approval_reads_as_words() {
        let mut r = row(Status::Pending, vec![]);
        let now = r#"{"action":{"worktree":false},"caps":{"max_tokens":1000000},"files":{"gate.cmd":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"permissions":{"profile":"worktree-edit"},"trigger":{"at":"10:30","kind":"daily"}}"#;
        r.approvable = Some(Ok(Covered { json: now.into(), fingerprint: "f".into(), command_sha256: None }));
        r.schedule.approval = Some(Approval {
            fingerprint: "e".into(),
            mac: "m".into(),
            approved_at: NOW,
            covered: r#"{"action":{"worktree":true},"caps":{"max_tokens":3000000},"files":{"gate.cmd":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"permissions":{"profile":"worktree-edit"},"trigger":{"at":"09:00","kind":"daily"},"old":1}"#.into(),
            command_sha256: Some("c".into()),
        });
        r.command_path = Some(".claude/commands/gate.md".into());
        let text = page(&r);
        let changed: Vec<&str> = text.lines().skip_while(|l| *l != "Changed since your last approval").skip(2).take_while(|l| !l.is_empty()).collect();
        assert_eq!(
            changed,
            [
                "Runs in the folder's checkout instead of its own worktree",
                "Max tokens: 3000000 → 1000000",
                "File gate.cmd: its content changed",
                "Trigger time: 09:00 → 10:30",
                "old: 1 (gone)",
                "Slash command  .claude/commands/gate.md",
                "Approved file  sha256 c…",
                "File now  missing",
                "Its runs read the file as it is now. Approve makes this file the approved one.",
            ]
        );
        let renamed = Change { field: "permissions.profile".into(), before: Some("worktree-edit".into()), after: Some("edit".into()) };
        assert_eq!(change_words(&renamed), None, "the same profile, spelt as an earlier release did");
        r.approvable = Some(Err("it cannot be approved: no folder".into()));
        assert!(page(&r).contains("Cannot be approved\nit cannot be approved: no folder"));
    }

    #[test]
    fn last_runs_list_newest_first_with_how_it_fired_and_its_outcome() {
        let mut done = run("nightly-2", NOW, Some(Outcome::Done), "fixed 2 bugs");
        done.tokens = Some(Tokens { input: 80_000, output: 9_000, cache_creation: 40_000, cache_read: 1_000_000 });
        done.stages.push(supraman_core::schedule::Stage::Session {
            name: "fm-demo-sched-nightly-2".into(),
            id: "aaaa0001".into(),
            session_id: "aaaa0001-x".into(),
            worktree: None,
            workstate_session: None,
            without_workstate: None,
        });
        let mut manual = run("nightly-1", NOW - 86_400_000, Some(Outcome::NothingToDo), "");
        manual.started = Some((NOW - 86_400_000, Fired::RunNow));
        let r = row(Status::Active, vec![done, manual]);
        let text = page(&r);
        let runs: Vec<&str> = text.lines().skip_while(|l| *l != "Last runs").skip(1).collect();
        assert!(runs[0].ends_with("  on schedule · done: fixed 2 bugs · 1 min 5 s · 129k tokens · fm-demo-sched-nightly-2"), "{text}");
        assert!(runs[1].ends_with("  Run now · nothing to do · 1 min 5 s"), "{text}");
        assert!(sections(&r, "z", Some("next fires:\n  x\n"), NOW).last().is_some_and(|s| s.key == "dry-run" && s.lines == [supraman_ui::details::Line::Long("next fires:\n  x".into())]));
    }

    #[test]
    fn a_pending_schedule_waits_under_a_key_that_changes_with_what_it_covers() {
        let mut rows = BTreeMap::new();
        let mut pending = row(Status::Pending, vec![]);
        pending.approvable = Some(Ok(Covered { json: "{}".into(), fingerprint: "f1".into(), command_sha256: None }));
        rows.insert("demo".to_string(), vec![pending, row(Status::Active, vec![])]);
        let w = waiting(&rows);
        assert_eq!(w, [Waiting { workspace: "demo".into(), id: "nightly".into(), title: "Nightly gate".into(), key: "demo/nightly@f1".into(), command_changed: false }]);
        rows.get_mut("demo").unwrap()[0].approvable = Some(Ok(Covered { json: "{}".into(), fingerprint: "f2".into(), command_sha256: None }));
        assert_eq!(waiting(&rows)[0].key, "demo/nightly@f2");
        rows.get_mut("demo").unwrap()[0].status = Status::Active;
        assert!(waiting(&rows).is_empty());

        let changed = &mut rows.get_mut("demo").unwrap()[0];
        changed.command_changed = true;
        changed.approvable = Some(Ok(Covered { json: "{}".into(), fingerprint: "f2".into(), command_sha256: Some("c2".into()) }));
        let w = waiting(&rows);
        assert_eq!((w.len(), w[0].key.as_str(), w[0].command_changed), (1, "demo/nightly@command-c2", true), "an active schedule whose command changed waits too");
        rows.get_mut("demo").unwrap()[0].approvable = Some(Ok(Covered { json: "{}".into(), fingerprint: "f2".into(), command_sha256: Some("c3".into()) }));
        assert_eq!(waiting(&rows)[0].key, "demo/nightly@command-c3", "another edit of the file is another entry");
        rows.get_mut("demo").unwrap()[0].command_changed = false;
        assert!(waiting(&rows).is_empty(), "approved again, the entry goes");
        assert_eq!(tray_tooltip(0, 1), "Supraman · 1 schedule waits for your approval");
        assert_eq!(tray_tooltip(1, 2), "Supraman · 1 scheduled run needs you · 2 schedules wait for your approval");
    }

    #[test]
    fn a_dismissed_wait_stays_dismissed_across_a_restart_until_its_fingerprint_changes() {
        use crate::dismissed_messages::DismissedMessages;
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("schedules").join("waiting-dismissed.json");
        let pending = |fingerprint: &str| {
            let mut r = row(Status::Pending, vec![]);
            r.approvable = Some(Ok(Covered { json: "{}".into(), fingerprint: fingerprint.into(), command_sha256: None }));
            BTreeMap::from([("demo".to_string(), vec![r])])
        };
        let shown = |d: &DismissedMessages, rows: &BTreeMap<String, Vec<ScheduleRow>>| waiting(rows).into_iter().filter(|w| !d.hides(&w.workspace, &w.key)).count();
        let rows = pending("f1");
        let (mut d, notice) = DismissedMessages::load_at(Some(file.clone()), "x");
        assert!(notice.is_none() && shown(&d, &rows) == 1);
        let w = &waiting(&rows)[0];
        d.dismiss(&w.workspace, &w.key).unwrap().write().unwrap();
        let (mut restarted, _) = DismissedMessages::load_at(Some(file.clone()), "x");
        assert!(!restarted.prune_keys(&waiting_keys(&rows)).0);
        assert_eq!(shown(&restarted, &rows), 0, "still dismissed after a restart");
        assert!(!restarted.prune_keys(&waiting_keys(&BTreeMap::new())).0, "a workspace the poll did not read keeps it");
        let edited = pending("f2");
        assert_eq!(shown(&restarted, &edited), 1, "a covered edit shows it again");
        let (changed, save) = restarted.prune_keys(&waiting_keys(&edited));
        assert!(changed);
        save.unwrap().write().unwrap();
        let (again, _) = DismissedMessages::load_at(Some(file), "x");
        assert_eq!(shown(&again, &rows), 1, "the old fingerprint's dismissal is gone from the file");
    }

    #[test]
    fn only_a_latest_run_that_needs_rene_goes_to_the_inbox_and_the_tray() {
        let mut rows = BTreeMap::new();
        rows.insert("demo".to_string(), vec![row(Status::Active, vec![run("r2", NOW, Some(Outcome::Failed), "exit 1"), run("r1", NOW - 1, Some(Outcome::Done), "")])]);
        let a = attention(&rows);
        assert_eq!(a, [Attention { workspace: "demo".into(), id: "nightly".into(), run: "r2".into(), outcome: Outcome::Failed, why: "exit 1".into(), at: NOW + 65_000 }]);
        rows.get_mut("demo").unwrap()[0].runs.insert(0, run("r3", NOW + 1, Some(Outcome::Done), ""));
        assert!(attention(&rows).is_empty(), "a later good run clears it");
        rows.get_mut("demo").unwrap()[0].runs.insert(0, run("r4", NOW + 2, Some(Outcome::NeedsYou), "a prompt"));
        assert_eq!(attention(&rows)[0].outcome, Outcome::NeedsYou);
        assert_eq!(tray_tooltip(0, 0), "Supraman");
        assert_eq!(tray_tooltip(2, 0), "Supraman · 2 scheduled runs need you");
    }

    fn other(folder: &str, name: &str, wd: Option<&str>, via: schedule::Via) -> ForeignTask {
        ForeignTask {
            folder: folder.into(),
            name: name.into(),
            enabled: true,
            trigger: "daily at 09:00".into(),
            last_run_ms: None,
            last_result: None,
            next_run_ms: None,
            working_dir: wd.map(PathBuf::from),
            via,
            action: "claude -p x".into(),
        }
    }

    #[test]
    fn other_schedules_go_under_the_workspace_holding_them_and_the_rest_apart() {
        use supraman_core::{Folder, FolderKind};
        let alpha = PathBuf::from(r"C:\code\alpha");
        let ws = Workspace::new("alpha", &alpha, vec![Folder { kind: FolderKind::Plain, ..Folder::new("alpha", &alpha) }]);
        let found = Foreign {
            tasks: vec![
                other(r"\", "in alpha", Some(r"C:\code\alpha\sub"), schedule::Via::Direct),
                other(r"\Mine", "script in alpha", None, schedule::Via::Script(PathBuf::from(r"C:\code\alpha\jobs\run.ps1"))),
                other(r"\Mine", "elsewhere", Some(r"D:\other"), schedule::Via::Shell),
            ],
            unreadable: 0,
        };
        let others = Others::of(&found, &[ws]);
        let names = |ts: &[ForeignTask]| ts.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(others.of_workspace("alpha")), ["in alpha", "script in alpha"]);
        assert_eq!(names(&others.unowned), ["elsewhere"]);
        assert!(others.of_workspace("beta").is_empty());
        assert_eq!(others.find(r"\Mine\elsewhere").map(|t| t.name.as_str()), Some("elsewhere"));
        assert_eq!(others.find(r"\in alpha").map(|t| t.name.as_str()), Some("in alpha"));
    }

    #[test]
    fn an_other_schedule_says_its_trigger_next_run_and_last_result() {
        let mut t = other(r"\", "t", None, schedule::Via::Direct);
        assert_eq!(other_summary(&t, NOW), "daily at 09:00 · never ran");
        t.next_run_ms = Some(NOW + 3_600_000);
        t.last_run_ms = Some(NOW - 3_600_000);
        t.last_result = Some(0);
        assert_eq!(other_summary(&t, NOW), format!("daily at 09:00 · next {} · last OK", time::near(NOW + 3_600_000, NOW)));
        t.last_result = Some(0x8007_0005_u32 as i32);
        t.enabled = false;
        t.next_run_ms = None;
        assert_eq!(other_summary(&t, NOW), "daily at 09:00 · disabled · last 0x80070005");
    }
}

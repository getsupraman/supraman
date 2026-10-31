//! The Schedules group of the tree: what each schedule shows, read off the UI thread with each poll.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{Local, TimeZone};
use foreman_core::Workspace;
use foreman_core::schedule::{self, Change, Covered, Live, Outcome, Run, Schedule, Stage, Status};
use foreman_core::secret::SecretStore;
use foreman_ui::hover_card::HoverCard;

/// History lists at most this many runs.
pub const HISTORY: usize = 30;

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

/// `Mon 09:00` this week, else `Mon 3 Nov 09:00`.
pub fn local_text(ms: i64, now_ms: i64) -> String {
    let (Some(at), Some(now)) = (Local.timestamp_millis_opt(ms).single(), Local.timestamp_millis_opt(now_ms).single()) else {
        return ms.to_string();
    };
    if (ms - now_ms).abs() < 6 * 24 * 3_600_000 && at.date_naive() != now.date_naive() {
        at.format("%a %H:%M").to_string()
    } else if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%a %-d %b %H:%M").to_string()
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
                    schedule: s,
                }
            })
            .collect();
        out.insert(w.name.clone(), rows);
    }
    (out, errors)
}

/// Each live file of `workspaces` whose fm is gone, recovered (`fm::fire::recover_if_dead`); the runs recovered.
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
                && let Some(run) = fm::fire::recover_if_dead(home, &w.name, id)
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

/// One line per run, newest first: outcome, start, duration, tokens and the session.
pub fn history_text(row: &ScheduleRow, now_ms: i64) -> String {
    let mut out = format!("History of {} ({} · {})", row.title(), row.workspace, row.schedule.id);
    if row.runs.is_empty() {
        out.push_str("\nNo run yet.");
    }
    for r in &row.runs {
        let started = r.started.map(|(ts, _)| ts);
        let outcome = match &r.ended {
            Some((_, o, _)) => outcome_text(*o),
            None if row.running.as_ref().is_some_and(|l| l.run == r.run) => "running",
            None => "no end recorded",
        };
        let mut line = format!("\n{outcome} · {}", started.map_or("not started".to_string(), |t| local_text(t, now_ms)));
        if let (Some(start), Some((end, _, _))) = (started, &r.ended) {
            line.push_str(&format!(" · {}", duration_text(end - start)));
        }
        if let Some(tokens) = &r.tokens {
            line.push_str(&format!(" · {} tokens", tokens_text(tokens.capped())));
        }
        if let Some(name) = session_of(r) {
            line.push_str(&format!(" · {name}"));
        }
        if let Some((_, _, why)) = r.ended.as_ref().filter(|(_, _, why)| !why.is_empty()) {
            line.push_str(&format!("\n    {why}"));
        }
        out.push_str(&line);
    }
    out
}

/// The schedule's facts for its menu's Details.
pub fn details(row: &ScheduleRow, now_ms: i64) -> HoverCard {
    let s = &row.schedule;
    let mut rows = vec![("id", s.id.clone()), ("status", row.status_text().to_string()), ("folder", s.folder.clone()), ("every", s.trigger.to_string())];
    rows.extend(row.next_fire.map(|t| ("next", local_text(t, now_ms))));
    let a = &s.action;
    let kind = match a.kind {
        schedule::ActionKind::Session => "session",
        schedule::ActionKind::ScriptThenSession => "script, then a session",
        schedule::ActionKind::Script => "script",
    };
    rows.push(("action", kind.to_string()));
    rows.extend(a.script.clone().map(|x| ("script", x)));
    rows.extend(a.prompt.as_ref().map(|p| {
        let text = match p {
            schedule::Prompt::Command(c) => c.clone(),
            schedule::Prompt::File(f) => format!("file {f}"),
            schedule::Prompt::Text(t) => format!("\"{}\"", t.chars().take(60).collect::<String>()),
        };
        ("prompt", text)
    }));
    rows.push(("profile", s.permissions.profile.as_str().to_string()));
    rows.push(("caps", format!("{} · {} tokens", s.caps.max_runtime, tokens_text(s.caps.max_tokens))));
    rows.extend(s.approval.as_ref().map(|a| ("approved", local_text(a.approved_at, now_ms))));
    rows.extend(row.running.as_ref().map(|l| ("running", format!("{} since {}", l.run, local_text(l.started_at, now_ms)))));
    rows.extend(row.last().map(|(r, o)| ("last", format!("{} · {}", outcome_text(o), r.run))));
    if !s.description.is_empty() {
        rows.push(("about", s.description.clone()));
    }
    HoverCard { title: format!("{} · {}", row.title(), row.workspace), rows }
}

/// A 64-hex value (a file's or a fingerprint's sha256) as its first 12 characters.
fn shown_value(v: &str) -> String {
    if v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()) { format!("{}…", &v[..12]) } else { v.to_string() }
}

fn change_line(c: &Change) -> String {
    match (&c.before, &c.after) {
        (Some(b), Some(a)) => format!("  {}: {} → {}", c.field, shown_value(b), shown_value(a)),
        (None, Some(a)) => format!("  {}: (new) {}", c.field, shown_value(a)),
        (Some(b), None) => format!("  {}: {} (gone)", c.field, shown_value(b)),
        (None, None) => format!("  {}", c.field),
    }
}

/// What Approve asks: the change since the last approval, else what the approval covers.
pub fn approve_text(workspace: &str, s: &Schedule, now: &Covered, now_ms: i64) -> String {
    let mut out = format!("Approve schedule {} of {workspace}? Its task then runs it {}.", s.id, s.trigger);
    match &s.approval {
        Some(a) => {
            let changes = schedule::covered_diff(&a.covered, &now.json);
            out.push_str(&format!("\nChanged since the approval of {}:", local_text(a.approved_at, now_ms)));
            if changes.is_empty() {
                out.push_str("\n  nothing covered");
            }
            for c in &changes {
                out.push('\n');
                out.push_str(&change_line(c));
            }
            if a.command_sha256 != now.command_sha256 {
                out.push_str("\n  the slash command's file (not covered; shown only)");
            }
        }
        None => {
            out.push_str("\nNever approved. The approval covers:");
            for (field, value) in schedule::covered_fields(&now.json) {
                out.push_str(&format!("\n  {field}: {}", shown_value(&value)));
            }
        }
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

/// The tray's tooltip: Foreman, and how many runs need René.
pub fn tray_tooltip(attention: usize) -> String {
    match attention {
        0 => "Supraman".into(),
        1 => "Supraman · 1 scheduled run needs you".into(),
        n => format!("Supraman · {n} scheduled runs need you"),
    }
}

#[cfg(test)]
mod tests {
    use foreman_core::schedule::{Approval, Fired, Tokens};

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
        ScheduleRow { workspace: "demo".into(), schedule: schedule(), status, command_changed: false, next_fire: None, running: None, runs }
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

    #[test]
    fn approve_shows_the_change_since_the_approval_else_every_covered_field() {
        let s = schedule();
        let now = Covered { json: r#"{"caps":{"max_tokens":1000000},"files":{"gate.cmd":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"trigger":{"at":"10:00","kind":"daily"}}"#.into(), fingerprint: "f".into(), command_sha256: None };
        let first = approve_text("demo", &s, &now, NOW);
        assert!(first.starts_with("Approve schedule nightly of demo? Its task then runs it daily 09:00.\nNever approved. The approval covers:\n"), "{first}");
        assert!(first.contains("\n  caps.max_tokens: 1000000\n  files.gate.cmd: bbbbbbbbbbbb…\n  trigger.at: 10:00"), "{first}");

        let mut approved = s.clone();
        approved.approval = Some(Approval {
            fingerprint: "e".into(),
            mac: "m".into(),
            approved_at: NOW,
            covered: r#"{"caps":{"max_tokens":3000000},"files":{"gate.cmd":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"trigger":{"at":"10:00","kind":"daily"},"old":1}"#.into(),
            command_sha256: Some("c".into()),
        });
        let text = approve_text("demo", &approved, &now, NOW);
        let lines: Vec<&str> = text.lines().skip(1).collect();
        assert!(lines[0].starts_with("Changed since the approval of "), "{text}");
        assert_eq!(
            lines[1..],
            ["  caps.max_tokens: 3000000 → 1000000", "  files.gate.cmd: aaaaaaaaaaaa… → bbbbbbbbbbbb…", "  old: 1 (gone)", "  the slash command's file (not covered; shown only)"]
        );
    }

    #[test]
    fn history_lists_runs_newest_first_with_outcome_duration_tokens_and_session() {
        let mut done = run("nightly-2", NOW, Some(Outcome::Done), "fixed 2 bugs");
        done.tokens = Some(Tokens { input: 80_000, output: 9_000, cache_creation: 40_000, cache_read: 1_000_000 });
        done.stages.push(foreman_core::schedule::Stage::Session {
            name: "fm-demo-sched-nightly-2".into(),
            id: "aaaa0001".into(),
            session_id: "aaaa0001-x".into(),
            worktree: None,
            workstate_session: None,
            without_workstate: None,
        });
        let r = row(Status::Active, vec![done, run("nightly-1", NOW - 86_400_000, Some(Outcome::NothingToDo), "")]);
        let text = history_text(&r, NOW);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "History of Nightly gate (demo · nightly)");
        assert!(lines[1].starts_with("done · ") && lines[1].ends_with(" · 1 min 5 s · 129k tokens · fm-demo-sched-nightly-2"), "{text}");
        assert_eq!(lines[2], "    fixed 2 bugs");
        assert!(lines[3].starts_with("nothing to do · ") && lines[3].ends_with(" · 1 min 5 s"), "{text}");
        assert!(history_text(&row(Status::Pending, vec![]), NOW).ends_with("\nNo run yet."));
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
        assert_eq!(tray_tooltip(0), "Supraman");
        assert_eq!(tray_tooltip(2), "Supraman · 2 scheduled runs need you");
    }
}

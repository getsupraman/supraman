use std::fmt::Write as _;
use std::path::Path;

use chrono::NaiveDate;
use sha2::{Digest, Sha256};

use super::approval::hex;
use super::{CatchUp, Day, Schedule, Span, Trigger};

/// The `<RunLevel>` of a schedule's task: the claude service's elevation (docs/research/scheduler.md, Decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLevel {
    LeastPrivilege,
    HighestAvailable,
}

impl RunLevel {
    pub fn for_service(elevated: bool) -> Self {
        if elevated { Self::HighestAvailable } else { Self::LeastPrivilege }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LeastPrivilege => "LeastPrivilege",
            Self::HighestAvailable => "HighestAvailable",
        }
    }
}

/// Everything one task definition is built from.
#[derive(Debug, Clone)]
pub struct TaskSpec<'a> {
    /// The principal: the current user's SID.
    pub user: &'a str,
    pub level: RunLevel,
    pub fmw: &'a Path,
    pub home: &'a Path,
    pub workspace: &'a str,
    pub schedule: &'a Schedule,
    pub fingerprint: &'a str,
    /// The local date of the trigger's first start.
    pub start: NaiveDate,
}

/// The backstop beyond `max_runtime + script_timeout`; fm ends runs itself.
const SLACK: u64 = 30 * 60;

/// The task's XML (docs/research/scheduler.md, "Task Scheduler registration"). `<Version>` holds
/// [`definition_digest`], which `sync` compares instead of the XML Task Scheduler reads back.
pub fn task_xml(spec: &TaskSpec) -> String {
    render(spec, &definition_digest(spec), spec.start)
}

/// A digest of everything in the definition but the start date, which follows the day of registering.
pub fn definition_digest(spec: &TaskSpec) -> String {
    let fixed = NaiveDate::from_ymd_opt(2000, 1, 1).expect("a valid date");
    hex(&Sha256::digest(render(spec, "", fixed).as_bytes()))[..16].to_string()
}

/// The `<Version>` of a task's XML, as written or as read back.
pub fn version_of(xml: &str) -> Option<&str> {
    let start = xml.find("<Version>")? + "<Version>".len();
    let len = xml[start..].find("</Version>")?;
    Some(&xml[start..start + len])
}

fn render(spec: &TaskSpec, version: &str, start: NaiveDate) -> String {
    let s = spec.schedule;
    let home = trimmed(spec.home);
    let limit = Span(s.caps.max_runtime.0 + s.action.script_timeout.0 + SLACK);
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n");
    x.push_str("<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n");
    x.push_str("  <RegistrationInfo>\n");
    x.push_str("    <Source>Foreman</Source>\n");
    let _ = writeln!(x, "    <Version>{}</Version>", esc(version));
    let _ = writeln!(x, "    <Description>{}</Description>", esc(&format!("Foreman schedule {}/{}; Foreman rewrites this task.", spec.workspace, s.id)));
    x.push_str("  </RegistrationInfo>\n");
    x.push_str("  <Triggers>\n    <CalendarTrigger>\n");
    let at = match &s.trigger {
        Trigger::Weekly { at, .. } | Trigger::Daily { at } => *at,
        Trigger::Interval { at, every } => {
            let _ = writeln!(x, "      <Repetition>\n        <Interval>{}</Interval>\n        <Duration>P1D</Duration>\n        <StopAtDurationEnd>false</StopAtDurationEnd>\n      </Repetition>", iso(*every));
            *at
        }
    };
    let _ = writeln!(x, "      <StartBoundary>{}T{:02}:{:02}:00</StartBoundary>", start.format("%Y-%m-%d"), at.hour, at.minute);
    x.push_str("      <Enabled>true</Enabled>\n");
    match &s.trigger {
        Trigger::Weekly { days, .. } => {
            x.push_str("      <ScheduleByWeek>\n        <DaysOfWeek>\n");
            let mut days = days.clone();
            days.sort();
            days.dedup();
            for d in days {
                let _ = writeln!(x, "          <{}/>", day_name(d));
            }
            x.push_str("        </DaysOfWeek>\n        <WeeksInterval>1</WeeksInterval>\n      </ScheduleByWeek>\n");
        }
        Trigger::Daily { .. } | Trigger::Interval { .. } => x.push_str("      <ScheduleByDay>\n        <DaysInterval>1</DaysInterval>\n      </ScheduleByDay>\n"),
    }
    x.push_str("    </CalendarTrigger>\n  </Triggers>\n");
    x.push_str("  <Principals>\n    <Principal id=\"Author\">\n");
    let _ = writeln!(x, "      <UserId>{}</UserId>", esc(spec.user));
    x.push_str("      <LogonType>InteractiveToken</LogonType>\n");
    let _ = writeln!(x, "      <RunLevel>{}</RunLevel>", spec.level.as_str());
    x.push_str("    </Principal>\n  </Principals>\n");
    x.push_str("  <Settings>\n");
    x.push_str("    <MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>\n");
    x.push_str("    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n");
    x.push_str("    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n");
    let _ = writeln!(x, "    <StartWhenAvailable>{}</StartWhenAvailable>", s.policy.catch_up == CatchUp::Once);
    let _ = writeln!(x, "    <WakeToRun>{}</WakeToRun>", s.policy.wake);
    let _ = writeln!(x, "    <ExecutionTimeLimit>{}</ExecutionTimeLimit>", iso(limit));
    x.push_str("    <Enabled>true</Enabled>\n");
    x.push_str("  </Settings>\n");
    x.push_str("  <Actions Context=\"Author\">\n    <Exec>\n");
    let _ = writeln!(x, "      <Command>{}</Command>", esc(&spec.fmw.to_string_lossy()));
    let args = format!("schedule fire --home \"{home}\" {} {} --fingerprint {}", spec.workspace, s.id, spec.fingerprint);
    let _ = writeln!(x, "      <Arguments>{}</Arguments>", esc(&args));
    let _ = writeln!(x, "      <WorkingDirectory>{}</WorkingDirectory>", esc(&home));
    x.push_str("    </Exec>\n  </Actions>\n</Task>\n");
    x
}

/// A trailing separator would escape the closing quote of `--home "…"`.
fn trimmed(path: &Path) -> String {
    let s = path.to_string_lossy();
    let t = s.trim_end_matches(['\\', '/']);
    if t.is_empty() || t.ends_with(':') { s.into_owned() } else { t.to_string() }
}

fn day_name(d: Day) -> &'static str {
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][d as usize]
}

/// An `xs:duration` such as `PT1H40M`.
fn iso(span: Span) -> String {
    let (h, m, s) = (span.0 / 3600, span.0 % 3600 / 60, span.0 % 60);
    let mut out = String::from("PT");
    if h > 0 {
        let _ = write!(out, "{h}H");
    }
    if m > 0 {
        let _ = write!(out, "{m}M");
    }
    if s > 0 || span.0 == 0 {
        let _ = write!(out, "{s}S");
    }
    out
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

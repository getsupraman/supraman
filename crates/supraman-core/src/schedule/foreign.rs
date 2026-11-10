//! Task Scheduler tasks outside Supraman's roots that run `claude`: read only, never changed, run or deleted.

use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use roxmltree::{Document, Node};

use super::registrar::{Registrar, TaskState};
use crate::platform::shell::expand_env;
use crate::{Result, Workspace, holder_of};

/// A script is read this far for the word `claude`.
pub const SCRIPT_CAP: u64 = 256 * 1024;
/// `SCHED_S_TASK_HAS_NOT_RUN`.
pub const NOT_RUN: i32 = 0x0004_1303;

const CLAUDE: [&str; 3] = ["claude", "claude.exe", "claude.cmd"];
const SHELLS: [&str; 6] = ["pwsh", "pwsh.exe", "powershell", "powershell.exe", "cmd", "cmd.exe"];
const SCRIPTS: [&str; 3] = [".ps1", ".cmd", ".bat"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignTask {
    pub folder: String,
    pub name: String,
    pub enabled: bool,
    /// [`trigger_words`].
    pub trigger: String,
    pub last_run_ms: Option<i64>,
    /// Only once it ran.
    pub last_result: Option<i32>,
    pub next_run_ms: Option<i64>,
    pub working_dir: Option<PathBuf>,
    pub via: Via,
    /// The matching action as `command arguments`.
    pub action: String,
}

/// How an action reaches `claude`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Via {
    Direct,
    Shell,
    Script(PathBuf),
}

impl ForeignTask {
    pub fn path(&self) -> String {
        format!(r"{}\{}", self.folder.trim_end_matches('\\'), self.name)
    }

    /// `OK` for 0, else the code in hex.
    pub fn result_text(&self) -> Option<String> {
        self.last_result.map(|r| if r == 0 { "OK".to_string() } else { format!("0x{:08X}", r as u32) })
    }

    /// The working directory, then the script's folder.
    pub fn dirs(&self) -> impl Iterator<Item = &Path> {
        let script = match &self.via {
            Via::Script(p) => p.parent(),
            _ => None,
        };
        self.working_dir.as_deref().into_iter().chain(script)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Foreign {
    /// By path.
    pub tasks: Vec<ForeignTask>,
    /// Folders and tasks Task Scheduler would not let us read.
    pub unreadable: usize,
}

/// `\Supraman…`: what [`super::sync`] owns, any home.
pub fn is_own_root(folder: &str) -> bool {
    let top = folder.trim_start_matches('\\').split('\\').next().unwrap_or_default().to_ascii_lowercase();
    top.starts_with("supraman")
}

/// Every task outside [`is_own_root`] that runs `claude`, its scripts read from disk.
pub fn scan(registrar: &dyn Registrar) -> Result<Foreign> {
    scan_with(registrar, &read_script)
}

pub fn scan_with(registrar: &dyn Registrar, read: &dyn Fn(&Path) -> Option<String>) -> Result<Foreign> {
    let listing = registrar.all_tasks(&is_own_root)?;
    let mut tasks: Vec<ForeignTask> = listing.tasks.iter().filter(|t| !is_own_root(&t.folder)).filter_map(|t| of_task(t, read)).collect();
    tasks.sort_by_key(|t| t.path().to_lowercase());
    Ok(Foreign { tasks, unreadable: listing.unreadable })
}

/// The task when one of its `Exec` actions runs `claude` ([`runs_claude`]).
pub fn of_task(task: &TaskState, read: &dyn Fn(&Path) -> Option<String>) -> Option<ForeignTask> {
    let doc = Document::parse(&task.xml).ok()?;
    doc.descendants().filter(|n| n.tag_name().name() == "Exec").find_map(|exec| {
        let command = child_text(exec, "Command")?;
        let args = child_text(exec, "Arguments").unwrap_or_default();
        let working_dir = child_text(exec, "WorkingDirectory").map(|d| PathBuf::from(expand_env(d.trim_matches('"'))));
        let via = runs_claude(command, args, working_dir.as_deref(), read)?;
        let ran = task.last_run_ms.is_some() && task.last_result != NOT_RUN;
        Some(ForeignTask {
            folder: task.folder.clone(),
            name: task.name.clone(),
            enabled: task.enabled,
            trigger: trigger_words(&task.xml),
            last_run_ms: task.last_run_ms,
            last_result: ran.then_some(task.last_result),
            next_run_ms: task.next_run_ms,
            working_dir,
            via,
            action: if args.is_empty() { command.to_string() } else { format!("{command} {args}") },
        })
    })
}

/// `claude` as the command; a shell whose arguments name `claude`; or a `.ps1`/`.cmd`/`.bat`, as the command or
/// in the arguments, whose text names it. Relative scripts resolve against `working_dir`.
pub fn runs_claude(command: &str, args: &str, working_dir: Option<&Path>, read: &dyn Fn(&Path) -> Option<String>) -> Option<Via> {
    let command = expand_env(command.trim().trim_matches('"'));
    let exe = Path::new(&command).file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    if CLAUDE.contains(&exe.as_str()) {
        return Some(Via::Direct);
    }
    let args = expand_env(args);
    if SHELLS.contains(&exe.as_str()) && names_claude(&args) {
        return Some(Via::Shell);
    }
    std::iter::once(command.as_str()).chain(words(&args)).filter(|w| is_script(w)).find_map(|w| {
        let path = match (Path::new(w), working_dir) {
            (p, Some(dir)) if p.is_relative() => dir.join(p),
            (p, _) => p.to_path_buf(),
        };
        read(&path).is_some_and(|text| names_claude(&text)).then_some(Via::Script(path))
    })
}

/// `claude` as a word of its own, any case: `claude.exe` and `claude-code` count, `claudette` does not.
pub fn names_claude(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    lower.match_indices("claude").any(|(i, m)| !word(lower[..i].chars().next_back()) && !word(lower[i + m.len()..].chars().next()))
}

/// Quoted runs whole (either quote), the rest split at whitespace, each without a leading `&` or trailing `;`/`,`.
fn words(args: &str) -> impl Iterator<Item = &str> {
    args.split(['"', '\''])
        .enumerate()
        .flat_map(|(i, part)| if i % 2 == 1 { vec![part] } else { part.split_whitespace().collect() })
        .map(|w| w.trim().trim_start_matches('&').trim_end_matches([';', ',']).trim())
        .filter(|w| !w.is_empty())
}

fn is_script(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    SCRIPTS.iter().any(|x| lower.ends_with(x))
}

/// The first [`SCRIPT_CAP`] bytes as text (UTF-16 with its BOM, else UTF-8); `None` when it cannot be read.
pub fn read_script(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(SCRIPT_CAP).read_to_end(&mut bytes).ok()?;
    Some(crate::text::decode(&bytes).0)
}

/// `on logon`, `at startup`, `daily at 09:00`, `weekly Mon, Thu at 08:00`, `once at 2026-11-02 07:30`, several
/// joined by `, `; `custom` for anything else.
pub fn trigger_words(xml: &str) -> String {
    let Ok(doc) = Document::parse(xml) else { return "custom".into() };
    let Some(triggers) = doc.descendants().find(|n| n.tag_name().name() == "Triggers") else { return "on demand".into() };
    let words: Vec<String> = triggers.children().filter(Node::is_element).map(trigger_word).collect();
    if words.is_empty() { "on demand".into() } else { words.join(", ") }
}

fn trigger_word(t: Node) -> String {
    let custom = || "custom".to_string();
    let start = child_text(t, "StartBoundary").filter(|s| s.len() >= 16);
    let at = start.map(|s| &s[11..16]);
    if child(t, "Repetition").is_some() {
        return custom();
    }
    match t.tag_name().name() {
        "LogonTrigger" => "on logon".into(),
        "BootTrigger" => "at startup".into(),
        "IdleTrigger" => "when idle".into(),
        "TimeTrigger" => start.map_or_else(custom, |s| format!("once at {} {}", &s[..10], &s[11..16])),
        "CalendarTrigger" => {
            let Some(at) = at else { return custom() };
            let interval = |n: Node, name| child_text(n, name).unwrap_or("1").to_string();
            if let Some(day) = child(t, "ScheduleByDay") {
                return match interval(day, "DaysInterval").as_str() {
                    "1" => format!("daily at {at}"),
                    n => format!("every {n} days at {at}"),
                };
            }
            if let Some(week) = child(t, "ScheduleByWeek")
                && interval(week, "WeeksInterval") == "1"
                && let Some(days) = child(week, "DaysOfWeek")
            {
                let days: Vec<&str> = days.children().filter(Node::is_element).map(|d| d.tag_name().name().get(..3).unwrap_or_default()).collect();
                if !days.is_empty() {
                    return format!("weekly {} at {at}", days.join(", "));
                }
            }
            custom()
        }
        _ => custom(),
    }
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.tag_name().name() == name)
}

fn child_text<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(n, name).and_then(|c| c.text()).map(str::trim).filter(|s| !s.is_empty())
}

/// The workspace holding the first of [`ForeignTask::dirs`] that one holds ([`holder_of`]).
pub fn owner<'w>(task: &ForeignTask, workspaces: &'w [Workspace]) -> Option<&'w Workspace> {
    task.dirs().find_map(|d| holder_of(d, workspaces))
}

/// An OLE automation date (local days since 1899-12-30) as epoch ms; `None` for never, which Task Scheduler
/// reports as `0` or a date before 2000.
pub fn ole_date_ms(days: f64) -> Option<i64> {
    if !days.is_finite() || days <= 0.0 {
        return None;
    }
    let base = NaiveDate::from_ymd_opt(1899, 12, 30)?.and_hms_opt(0, 0, 0)?;
    let at = base.checked_add_signed(chrono::Duration::milliseconds((days * 86_400_000.0).round() as i64))?;
    if at.year() < 2000 {
        return None;
    }
    Local.from_local_datetime(&at).earliest().map(|t| t.timestamp_millis())
}

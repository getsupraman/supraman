use std::path::{Path, PathBuf};

use super::*;
use crate::{Folder, FolderKind};

fn task_xml(triggers: &str, execs: &[(&str, &str, &str)]) -> String {
    let actions: String = execs
        .iter()
        .map(|(cmd, args, wd)| {
            let opt = |tag: &str, v: &str| if v.is_empty() { String::new() } else { format!("<{tag}>{v}</{tag}>") };
            format!("<Exec><Command>{cmd}</Command>{}{}</Exec>", opt("Arguments", args), opt("WorkingDirectory", wd))
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>{triggers}</Triggers>
  <Actions Context="Author">{actions}</Actions>
</Task>"#
    )
}

const DAILY: &str = "<CalendarTrigger><StartBoundary>2026-11-01T09:00:00</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>";

fn no_scripts(_: &Path) -> Option<String> {
    None
}

fn via(cmd: &str, args: &str) -> Option<Via> {
    runs_claude(cmd, args, None, &no_scripts)
}

#[test]
fn claude_as_the_command_is_kept_by_its_file_name() {
    assert_eq!(via(r"C:\Users\rene\.local\bin\claude.exe", "-p \"triage\""), Some(Via::Direct));
    assert_eq!(via(r#""C:\Program Files\nodejs\claude.cmd""#, ""), Some(Via::Direct));
    assert_eq!(via("CLAUDE", "-p hi"), Some(Via::Direct));
    assert_eq!(via(r"C:\tools\claudette.exe", "-p hi"), None);
    assert_eq!(via(r"C:\tools\backup.exe", "--to claude"), None, "only a shell's arguments count");
}

#[test]
fn a_shell_whose_arguments_name_claude_is_kept() {
    assert_eq!(via("pwsh.exe", r#"-NoProfile -Command "claude -p 'review the PRs'""#), Some(Via::Shell));
    assert_eq!(via(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe", "-c claude.exe -p x"), Some(Via::Shell));
    assert_eq!(via("cmd", "/c claude -p nightly > out.txt"), Some(Via::Shell));
    assert_eq!(via("cmd.exe", "/c claudette --run"), None);
    assert_eq!(via("pwsh", "-Command Get-Date"), None);
}

#[test]
fn a_script_is_kept_when_its_text_names_claude() {
    let dir = Path::new(r"C:\jobs");
    let read = |p: &Path| match p.to_str()? {
        r"C:\jobs\nightly.ps1" => Some("Set-Location C:\\code\r\nclaude -p 'fix the gate'\r\n".to_string()),
        r"C:\jobs\other.ps1" => Some("Write-Host 'claudette is a name'".to_string()),
        r"C:\jobs\run me.cmd" => Some("@echo off\r\nCLAUDE.EXE -p hi\r\n".to_string()),
        r"C:\jobs\job.bat" => Some("call claude -p x".to_string()),
        _ => None,
    };
    let script = |cmd: &str, args: &str| runs_claude(cmd, args, Some(dir), &read);
    assert_eq!(script("pwsh.exe", "-File nightly.ps1"), Some(Via::Script(dir.join("nightly.ps1"))));
    assert_eq!(script("powershell", r#"-ExecutionPolicy Bypass -Command "& 'C:\jobs\nightly.ps1'""#), Some(Via::Script(PathBuf::from(r"C:\jobs\nightly.ps1"))));
    assert_eq!(script("cmd", r#"/c "C:\jobs\run me.cmd""#), Some(Via::Script(PathBuf::from(r"C:\jobs\run me.cmd"))));
    assert_eq!(script(r"C:\jobs\job.bat", ""), Some(Via::Script(PathBuf::from(r"C:\jobs\job.bat"))), "the script as the command");
    assert_eq!(script("pwsh", "-File other.ps1"), None, "claudette is not claude");
    assert_eq!(script("pwsh", "-File missing.ps1"), None, "a script that cannot be read is not kept");
}

#[test]
fn claude_is_a_word_of_its_own() {
    for yes in ["claude", "run Claude -p", "claude.exe", "npx claude-code", "x;claude", "& \"claude\""] {
        assert!(names_claude(yes), "{yes}");
    }
    for no in ["claudette", "myclaude", "claude_job", "claudes", ""] {
        assert!(!names_claude(no), "{no}");
    }
}

#[test]
fn a_script_is_read_as_utf16_or_utf8_up_to_its_cap() {
    let dir = tempfile::tempdir().unwrap();
    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("claude -p x".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    fs::write(dir.path().join("a.ps1"), utf16).unwrap();
    assert_eq!(read_script(&dir.path().join("a.ps1")).as_deref(), Some("claude -p x"));
    fs::write(dir.path().join("b.cmd"), b"\xEF\xBB\xBFclaude").unwrap();
    assert_eq!(read_script(&dir.path().join("b.cmd")).as_deref(), Some("claude"));
    let mut long = vec![b' '; SCRIPT_CAP as usize];
    long.extend_from_slice(b"claude");
    fs::write(dir.path().join("c.bat"), long).unwrap();
    assert!(!names_claude(&read_script(&dir.path().join("c.bat")).unwrap()), "past the cap is not read");
    assert_eq!(read_script(&dir.path().join("none.ps1")), None);
}

#[test]
fn triggers_read_as_words() {
    let words = |t: &str| trigger_words(&task_xml(t, &[]));
    assert_eq!(words("<LogonTrigger><Enabled>true</Enabled></LogonTrigger>"), "on logon");
    assert_eq!(words("<BootTrigger/>"), "at startup");
    assert_eq!(words(DAILY), "daily at 09:00");
    assert_eq!(
        words("<CalendarTrigger><StartBoundary>2026-11-01T08:00:00+01:00</StartBoundary><ScheduleByWeek><DaysOfWeek><Monday/><Thursday/></DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek></CalendarTrigger>"),
        "weekly Mon, Thu at 08:00"
    );
    assert_eq!(words("<CalendarTrigger><StartBoundary>2026-11-01T06:15:00</StartBoundary><ScheduleByDay><DaysInterval>3</DaysInterval></ScheduleByDay></CalendarTrigger>"), "every 3 days at 06:15");
    assert_eq!(words("<TimeTrigger><StartBoundary>2026-11-02T07:30:00</StartBoundary></TimeTrigger>"), "once at 2026-11-02 07:30");
    assert_eq!(
        words("<CalendarTrigger><Repetition><Interval>PT1H</Interval></Repetition><StartBoundary>2026-11-01T00:00:00</StartBoundary><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>"),
        "custom"
    );
    assert_eq!(words("<CalendarTrigger><StartBoundary>2026-11-01T00:00:00</StartBoundary><ScheduleByMonth><DaysOfMonth><Day>1</Day></DaysOfMonth></ScheduleByMonth></CalendarTrigger>"), "custom");
    assert_eq!(words(&format!("<LogonTrigger/>{DAILY}")), "on logon, daily at 09:00");
    assert_eq!(words(""), "on demand");
    assert_eq!(trigger_words("not xml"), "custom");
}

fn planted(folder: &str, name: &str, xml: String) -> TaskState {
    TaskState { folder: folder.into(), name: name.into(), xml, enabled: true, ..TaskState::default() }
}

#[test]
fn the_scan_reads_every_folder_but_supramans_and_writes_nothing() {
    let fake = FakeRegistrar::default();
    let claude = task_xml(DAILY, &[("claude.exe", "-p nightly", r"C:\code\app")]);
    fake.plant_task(TaskState { last_run_ms: Some(1_000), last_result: 0x8007_0002_u32 as i32, next_run_ms: Some(2_000), ..planted(r"\", "Nightly review", claude.clone()) });
    fake.plant_task(planted(r"\Mine\Deep", "pwsh job", task_xml("<LogonTrigger/>", &[("pwsh", "-c claude -p hi", "")])));
    fake.plant_task(TaskState { last_run_ms: Some(1_000), last_result: NOT_RUN, ..planted(r"\Mine", "never ran", claude.clone()) });
    fake.plant_task(planted(r"\Mine", "backup", task_xml(DAILY, &[("robocopy.exe", "C:\\a D:\\a", "")])));
    fake.plant_task(planted(r"\Supraman\demo", "gate", claude.clone()));
    fake.plant_task(planted(r"\Supraman-0a1b2c3d\demo", "gate", claude.clone()));
    fake.plant_unreadable(r"\Locked");
    fake.plant_task(planted(r"\Locked", "hidden", claude.clone()));
    fake.plant_unreadable(r"\Supraman\other");

    let found = scan_foreign_with(&fake, &no_scripts).unwrap();
    let paths: Vec<String> = found.tasks.iter().map(ForeignTask::path).collect();
    assert_eq!(paths, [r"\Mine\Deep\pwsh job", r"\Mine\never ran", r"\Nightly review"]);
    assert_eq!(found.unreadable, 1, "\\Locked counts, a Supraman folder is not looked at");
    assert!(fake.writes().is_empty(), "{:?}", fake.writes());

    let nightly = &found.tasks[2];
    assert_eq!(nightly.trigger, "daily at 09:00");
    assert_eq!(nightly.action, "claude.exe -p nightly");
    assert_eq!(nightly.working_dir.as_deref(), Some(Path::new(r"C:\code\app")));
    assert_eq!((nightly.last_run_ms, nightly.next_run_ms, nightly.result_text().as_deref()), (Some(1_000), Some(2_000), Some("0x80070002")));
    assert_eq!(found.tasks[1].last_result, None, "SCHED_S_TASK_HAS_NOT_RUN is no result");
    assert_eq!(found.tasks[0].via, Via::Shell);
    assert_eq!(found.tasks[0].trigger, "on logon");
}

#[test]
fn own_roots_are_supramans_of_any_home() {
    for own in [r"\Supraman", r"\Supraman-0a1b2c3d\ws", r"\supraman-99"] {
        assert!(is_own_root(own), "{own}");
    }
    for other in [r"\", r"\Microsoft\Windows", r"\Mine\Supraman", r"\Claude"] {
        assert!(!is_own_root(other), "{other}");
    }
}

#[test]
fn a_task_belongs_to_the_workspace_holding_its_working_dir_or_script() {
    let app = PathBuf::from(r"C:\code\app");
    let code = PathBuf::from(r"C:\code");
    let parent = Workspace::new("code", &code, vec![Folder { kind: FolderKind::Plain, ..Folder::new("code", &code) }]);
    let ws = Workspace::new("app", &app, vec![Folder { kind: FolderKind::Plain, ..Folder::new("app", &app) }]);
    let workspaces = [parent, ws];
    let read = |_: &Path| Some("claude".to_string());
    let task = |cmd: &str, args: &str, wd: &str| {
        of_task(&planted(r"\", "t", task_xml(DAILY, &[(cmd, args, wd)])), &read).unwrap()
    };
    assert_eq!(owner(&task("claude", "", r"C:\code\app\sub"), &workspaces).map(|w| w.name.as_str()), Some("app"));
    assert_eq!(owner(&task("pwsh", r"-File C:\code\app\scripts\job.ps1", ""), &workspaces).map(|w| w.name.as_str()), Some("app"));
    assert_eq!(owner(&task("claude", "", r"C:\code\other"), &workspaces).map(|w| w.name.as_str()), Some("code"), "the deepest workspace wins");
    assert_eq!(owner(&task("claude", "", r"C:\elsewhere"), &workspaces), None);
    assert_eq!(owner(&task("claude", "", ""), &workspaces), None);
}

#[test]
fn ole_dates_are_local_epoch_ms_and_never_is_none() {
    use chrono::TimeZone;
    assert_eq!(ole_date_ms(0.0), None);
    assert_eq!(ole_date_ms(36_494.0), None, "1999-11-30 is Task Scheduler's never");
    let want = chrono::Local.with_ymd_and_hms(2025, 1, 1, 12, 0, 0).earliest().unwrap().timestamp_millis();
    assert_eq!(ole_date_ms(45_658.5), Some(want));
}

#[test]
fn expand_env_keeps_what_is_not_a_set_variable() {
    let path = std::env::var("PATH").unwrap();
    assert_eq!(crate::platform::shell::expand_env("%PATH%;x"), format!("{path};x"));
    assert_eq!(crate::platform::shell::expand_env(r"%NO_SUCH_VAR_SCHED2_7%\x"), r"%NO_SUCH_VAR_SCHED2_7%\x");
    assert_eq!(crate::platform::shell::expand_env("100% sure %%"), "100% sure %%");
}

/// The real Task Scheduler, read only: prints what the scan finds. Never in the gate; run it by hand with `--ignored`.
#[cfg(windows)]
#[test]
#[ignore = "reads the real Task Scheduler"]
fn a_real_scan_reads_task_scheduler_and_prints_what_runs_claude() {
    use crate::platform::tasks::WindowsRegistrar;
    let all = WindowsRegistrar.all_tasks(&is_own_root).unwrap();
    let found = scan_foreign(&WindowsRegistrar).unwrap();
    println!("read {} tasks, {} unreadable, {} run claude", all.tasks.len(), found.unreadable, found.tasks.len());
    for t in &found.tasks {
        println!("{} | {} | {} | next {:?} | last {:?} {:?} | wd {:?}", t.path(), t.trigger, t.action, t.next_run_ms, t.last_run_ms, t.result_text(), t.working_dir);
    }
}

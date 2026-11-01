use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};

use super::*;
use crate::secret::{MemoryStore, SecretStore};
use crate::{CoreError, Folder, Registry};

const DOC_EXAMPLE: &str = r#"
version = 1

[[schedule]]
id = "autofix-prod-bugs"
name = "Autofix prod bugs"
description = ""
folder = "Nomos"
status = "pending"
created_by = "fm"
created_at = 1793337600000

[schedule.trigger]
kind = "weekly"
days = ["mon"]
at = "09:00"

[schedule.action]
kind = "script_then_session"
prompt = { command = "/autofix-prod-bugs" }
script = "scripts/gate.ps1"
script_args = []
script_timeout = "10m"
worktree = true
model = ""

[schedule.permissions]
profile = "worktree-edit"
allow_tools = ["Bash(dotnet test:*)"]
deny_tools = []
add_dirs = []
secrets = ["SENTRY_TOKEN"]

[schedule.caps]
max_runtime = "1h"
max_tokens = 3000000

[schedule.policy]
overlap = "skip"
catch_up = "once"
wake = false
"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    folder: PathBuf,
    ws: Workspace,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let folder = tmp.path().join("Nomos");
    fs::create_dir_all(folder.join("scripts")).unwrap();
    fs::create_dir_all(folder.join(".claude").join("commands")).unwrap();
    fs::create_dir_all(folder.join("prompts")).unwrap();
    fs::write(folder.join("scripts").join("gate.ps1"), "exit 10").unwrap();
    fs::write(folder.join(".claude").join("commands").join("autofix-prod-bugs.md"), "Fix the bugs.").unwrap();
    fs::write(folder.join("prompts").join("x.md"), "Look around.").unwrap();
    let ws = Workspace::new("demo", tmp.path(), vec![Folder::new("Nomos", &folder)]);
    Fixture { _tmp: tmp, home, folder, ws }
}

fn example() -> Schedule {
    toml::from_str::<ScheduleFile>(DOC_EXAMPLE).unwrap().schedule.remove(0)
}

fn add(f: &Fixture, s: Schedule) {
    edit(&f.home, &f.ws, |all| {
        all.push(s);
        Ok(())
    })
    .unwrap();
}

fn one(f: &Fixture, id: &str) -> Schedule {
    load(&f.home, "demo").unwrap().into_iter().find(|s| s.id == id).unwrap()
}

fn change(f: &Fixture, id: &str, how: impl FnOnce(&mut Schedule)) -> Schedule {
    edit(&f.home, &f.ws, |all| {
        how(all.iter_mut().find(|s| s.id == id).unwrap());
        Ok(())
    })
    .unwrap();
    one(f, id)
}

fn approved(f: &Fixture, store: &MemoryStore) -> Schedule {
    add(f, example());
    approve(&f.home, store, &f.ws, "autofix-prod-bugs", 1_793_341_200_000).unwrap()
}

#[test]
fn the_doc_example_loads_and_saves_unchanged() {
    let s = example();
    assert_eq!(s.trigger, Trigger::Weekly { days: vec![Day::Mon], at: Clock::new(9, 0).unwrap() });
    assert_eq!(s.action.prompt, Some(Prompt::Command("/autofix-prod-bugs".into())));
    assert_eq!((s.action.script_timeout, s.caps.max_runtime, s.caps.max_tokens), (Span(600), Span(3600), 3_000_000));
    assert_eq!((s.permissions.profile, s.policy.overlap, s.policy.catch_up), (Profile::Edit, Overlap::Skip, CatchUp::Once));
    let file = ScheduleFile { version: 1, schedule: vec![s] };
    assert_eq!(toml::from_str::<ScheduleFile>(&toml::to_string(&file).unwrap()).unwrap(), file);
    let f = fixture();
    assert_eq!(s_valid(&f, &file.schedule[0]), Ok(()));
}

fn s_valid(f: &Fixture, s: &Schedule) -> Result<(), String> {
    s.validate(&f.ws)
}

#[test]
fn defaults_fill_what_a_short_table_leaves_out() {
    let s: ScheduleFile = toml::from_str(
        r#"[[schedule]]
id = "n"
folder = "Nomos"
trigger = { kind = "interval", every = "2h" }
action = { kind = "session", prompt = { text = "hi" } }"#,
    )
    .unwrap();
    let s = &s.schedule[0];
    assert_eq!((s.status, s.created_by, s.permissions.profile), (Status::Pending, CreatedBy::Fm, Profile::Read));
    assert_eq!(s.trigger, Trigger::Interval { at: Clock::default(), every: Span(7200) });
    assert_eq!((s.caps.clone(), s.policy.clone()), (Caps::default(), Policy::default()));
    assert!(s.action.worktree);
}

#[test]
fn clocks_and_spans_parse_their_forms_and_refuse_others() {
    assert_eq!("9:05".parse::<Clock>(), Ok(Clock::new(9, 5).unwrap()));
    assert_eq!(Clock::new(9, 5).unwrap().to_string(), "09:05");
    for bad in ["24:00", "09:60", "9", "09:5", "aa:bb", ""] {
        assert!(bad.parse::<Clock>().is_err(), "{bad}");
    }
    assert_eq!(["30s", "10m", "2h"].map(|s| s.parse::<Span>().unwrap()), [Span(30), Span(600), Span(7200)]);
    assert_eq!([Span(30), Span(90), Span(600), Span(7200), Span(5400)].map(|s| s.to_string()), ["30s", "90s", "10m", "2h", "90m"]);
    for bad in ["2", "h", "-2h", "2d", "1.5h", ""] {
        assert!(bad.parse::<Span>().is_err(), "{bad}");
    }
}

#[test]
fn validate_refuses_what_the_doc_forbids() {
    let f = fixture();
    let refused = |how: fn(&mut Schedule)| {
        let mut s = example();
        how(&mut s);
        s.validate(&f.ws).unwrap_err()
    };
    assert!(refused(|s| s.id = "Bad".into()).contains("lowercase"));
    assert!(refused(|s| s.id = "a".repeat(33)).contains("32"));
    assert!(refused(|s| s.folder = "Gaia".into()).contains("no folder"));
    assert!(refused(|s| s.trigger = Trigger::Interval { at: Clock::default(), every: Span(600) }).contains("15m"));
    assert!(refused(|s| s.trigger = Trigger::Weekly { days: vec![], at: Clock::default() }).contains("day"));
    assert!(refused(|s| s.action.script = Some("gate.py".into())).contains(".ps1"));
    assert!(refused(|s| s.action.script = Some("../gate.ps1".into())).contains("inside the folder"));
    assert!(refused(|s| s.action.script = None).contains("needs a script"));
    assert!(refused(|s| s.action.prompt = Some(Prompt::File("C:/x.md".into()))).contains("inside the folder"));
    assert!(refused(|s| s.action.prompt = Some(Prompt::Command("/../x".into()))).contains("slash command"));
    assert!(refused(|s| s.action.prompt = None).contains("needs a prompt"));
    assert!(refused(|s| s.permissions.secrets = vec!["lower".into()]).contains("secret"));
    assert!(refused(|s| s.caps.max_runtime = Span(9 * 3600)).contains("max_runtime"));
    assert!(refused(|s| s.caps.max_tokens = 20_000_001).contains("max_tokens"));
    assert!(refused(|s| s.action.script_timeout = Span(7200)).contains("script_timeout"));
}

#[test]
fn edit_saves_under_the_lock_and_a_refused_edit_writes_nothing() {
    let f = fixture();
    assert!(load(&f.home, "demo").unwrap().is_empty(), "no file: no schedules");
    add(&f, example());
    let saved = one(&f, "autofix-prod-bugs");
    assert_eq!(saved, example(), "created_at was set, so it stays");
    assert!(f.home.join("schedules").join("demo.lock").exists());

    let dup = edit(&f.home, &f.ws, |all| {
        all.push(example());
        Ok(())
    });
    assert!(matches!(dup, Err(CoreError::InvalidSchedule { why, .. }) if why.contains("taken")));
    let bad = edit(&f.home, &f.ws, |all| {
        all[0].folder = "Gaia".into();
        Ok(())
    });
    assert!(bad.as_ref().is_err_and(CoreError::is_user_error), "{bad:?}");
    assert_eq!(load(&f.home, "demo").unwrap(), [saved]);

    edit(&f.home, &f.ws, |all| {
        all.clear();
        Ok(())
    })
    .unwrap();
    assert!(load(&f.home, "demo").unwrap().is_empty(), "removing a schedule removes its table");
    assert!(matches!(file_path(&f.home, "../x"), Err(CoreError::InvalidName(_))));
}

#[test]
fn concurrent_writers_lose_no_edit() {
    let f = fixture();
    let mut shared = example();
    shared.id = "shared".into();
    add(&f, shared);
    let (threads, rounds) = (8, 10);
    std::thread::scope(|scope| {
        for t in 0..threads {
            let f = &f;
            scope.spawn(move || {
                for r in 0..rounds {
                    edit(&f.home, &f.ws, |all| {
                        all.iter_mut().find(|s| s.id == "shared").unwrap().description.push('x');
                        let mut own = example();
                        own.id = format!("t{t}-r{r}");
                        all.push(own);
                        Ok(())
                    })
                    .unwrap();
                }
            });
        }
    });
    let all = load(&f.home, "demo").unwrap();
    assert_eq!(all.iter().find(|s| s.id == "shared").unwrap().description.len(), threads * rounds);
    assert_eq!(all.len(), 1 + threads * rounds);
    let leftovers: Vec<_> = fs::read_dir(f.home.join("schedules")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
    assert!(leftovers.is_empty());
}

#[test]
fn approve_activates_and_verifies_with_its_key_only() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    assert_eq!(s.status, Status::Active);
    let a = s.approval.clone().unwrap();
    assert_eq!(a.approved_at, 1_793_341_200_000);
    assert_eq!(a.fingerprint.len(), 64);
    assert!(a.covered.starts_with(r#"{"v":1,"workspace":"demo","id":"autofix-prod-bugs","folder":"Nomos","trigger":"#), "{}", a.covered);
    assert!(a.covered.contains(r#""command":{"name":"/autofix-prod-bugs","path":".claude/commands/autofix-prod-bugs.md"}"#), "{}", a.covered);
    assert!(a.covered.contains(r#""files":{"scripts/gate.ps1":""#), "{}", a.covered);
    assert!(a.command_sha256.is_some());
    assert_eq!(one(&f, "autofix-prod-bugs"), s);
    assert_eq!(verify_approval(&store, &f.ws, &s).unwrap().fingerprint, a.fingerprint);
    assert_eq!(effective_status(&store, &f.ws, &s), Status::Active);

    let key = store.get(APPROVAL_KEY).unwrap().unwrap();
    assert_eq!(key.len(), 64, "32 bytes as hex");
    let again = approve(&f.home, &store, &f.ws, "autofix-prod-bugs", 2).unwrap();
    assert_eq!(store.get(APPROVAL_KEY).unwrap().unwrap(), key, "the key is created once");
    assert_eq!(again.approval.unwrap().mac, a.mac);

    let other = MemoryStore::default();
    other.set(APPROVAL_KEY, &"ab".repeat(32)).unwrap();
    assert_eq!(verify_approval(&other, &f.ws, &s), Err(Refusal::BadMac), "the wrong key");
    assert_eq!(effective_status(&other, &f.ws, &s), Status::Pending);
    assert_eq!(verify_approval(&MemoryStore::default(), &f.ws, &s), Err(Refusal::NoKey));
    let mut forged = s.clone();
    forged.approval.as_mut().unwrap().mac = "00".repeat(32);
    assert_eq!(verify_approval(&store, &f.ws, &forged), Err(Refusal::BadMac));
    let mut never = s.clone();
    never.approval = None;
    assert_eq!(verify_approval(&store, &f.ws, &never), Err(Refusal::NotApproved));
    assert!(matches!(approve(&f.home, &store, &f.ws, "nope", 0), Err(CoreError::NoSchedule { .. })));
}

#[test]
fn approval_fails_after_a_covered_change_in_the_file_or_its_script() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    fs::write(f.folder.join("scripts").join("gate.ps1"), "exit 0").unwrap();
    assert_eq!(verify_approval(&store, &f.ws, &s), Err(Refusal::Changed), "the script's content is covered");
    assert_eq!(effective_status(&store, &f.ws, &s), Status::Pending);
    fs::write(f.folder.join("scripts").join("gate.ps1"), "exit 10").unwrap();
    assert!(verify_approval(&store, &f.ws, &s).is_ok());

    let mut edited_behind = s.clone();
    edited_behind.caps.max_tokens = 20_000_000;
    assert_eq!(verify_approval(&store, &f.ws, &edited_behind), Err(Refusal::Changed), "a TOML edit behind the app's back");
    let mut renamed = s.clone();
    renamed.name = "Something else".into();
    renamed.description = "now described".into();
    assert!(verify_approval(&store, &f.ws, &renamed).is_ok(), "name and description are not covered");
}

#[test]
fn a_slash_command_is_covered_by_name_and_path_not_content() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    let command = f.folder.join(".claude").join("commands").join("autofix-prod-bugs.md");
    fs::write(&command, "Fix every bug, differently.").unwrap();
    let now = verify_approval(&store, &f.ws, &s).expect("the command's content is not covered");
    assert_ne!(now.command_sha256, s.approval.as_ref().unwrap().command_sha256, "but its change shows");

    fs::write(f.folder.join(".claude").join("commands").join("other.md"), "x").unwrap();
    let mut other = s.clone();
    other.action.prompt = Some(Prompt::Command("/other".into()));
    assert_eq!(verify_approval(&store, &f.ws, &other), Err(Refusal::Changed), "another command");
    fs::remove_file(&command).unwrap();
    assert_ne!(covered(&f.ws, &s).unwrap().command_sha256, now.command_sha256, "a vanished command shows too");
}

#[test]
fn the_prompt_files_content_is_covered() {
    let f = fixture();
    let store = MemoryStore::default();
    let mut s = example();
    s.action.prompt = Some(Prompt::File("prompts/x.md".into()));
    add(&f, s);
    let s = approve(&f.home, &store, &f.ws, "autofix-prod-bugs", 1).unwrap();
    assert!(s.approval.as_ref().unwrap().command_sha256.is_none());
    fs::write(f.folder.join("prompts").join("x.md"), "Delete everything.").unwrap();
    assert_eq!(verify_approval(&store, &f.ws, &s), Err(Refusal::Changed));
}

#[test]
fn a_covered_edit_makes_the_schedule_pending_and_a_name_or_description_edit_does_not() {
    let f = fixture();
    let store = MemoryStore::default();
    type Change = (&'static str, fn(&mut Schedule));
    let edits: [Change; 9] = [
        ("trigger", |s| s.trigger = Trigger::Daily { at: Clock::new(9, 0).unwrap() }),
        ("action", |s| s.action.kind = ActionKind::Session),
        ("script", |s| s.action.script = Some("scripts/other.ps1".into())),
        ("prompt source", |s| s.action.prompt = Some(Prompt::Text("do it".into()))),
        ("permissions", |s| s.permissions.allow_tools.push("Bash(rm:*)".into())),
        ("caps", |s| s.caps.max_runtime = Span(7200)),
        ("policy", |s| s.policy.wake = true),
        ("folder", |s| s.folder = "Gaia".into()),
        ("model", |s| s.action.model = "opus".into()),
    ];
    let mut ws = f.ws.clone();
    ws.folders.push(Folder::new("Gaia", &f.folder));
    let f = Fixture { ws, ..f };
    for (what, how) in edits {
        let _ = edit(&f.home, &f.ws, |all| {
            all.clear();
            Ok(())
        });
        let s = approved(&f, &store);
        let after = change(&f, &s.id, |s| {
            if what == "action" {
                s.action.script = None;
            }
            how(s)
        });
        assert_eq!(after.status, Status::Pending, "{what}");
        assert_eq!(after.approval, s.approval, "{what}: the approval stays for Approve to diff against");
        assert_eq!(effective_status(&store, &f.ws, &after), Status::Pending, "{what}");
    }
    let s = one(&f, "autofix-prod-bugs");
    let s = approve(&f.home, &store, &f.ws, &s.id, 3).unwrap();
    let after = change(&f, &s.id, |s| {
        s.name = "Renamed".into();
        s.description = "Described".into();
    });
    assert_eq!((after.status, after.approval), (Status::Active, s.approval));
}

#[test]
fn an_edit_never_writes_the_approval_nor_activates_by_itself() {
    let f = fixture();
    let store = MemoryStore::default();
    let mut s = example();
    s.status = Status::Active;
    s.approval = Some(Approval { fingerprint: "f".into(), mac: "m".into(), approved_at: 1, covered: "{}".into(), command_sha256: None });
    add(&f, s);
    let s = one(&f, "autofix-prod-bugs");
    assert_eq!((s.status, s.approval), (Status::Pending, None), "a new schedule brings no approval and no activation");

    let s = approve(&f.home, &store, &f.ws, &s.id, 1).unwrap();
    let paused = change(&f, &s.id, |s| s.status = Status::Paused);
    assert_eq!(paused.status, Status::Paused);
    let forged = change(&f, &s.id, |s| s.approval.as_mut().unwrap().fingerprint = "0".repeat(64));
    assert_eq!(forged.approval, s.approval, "the approval is the app's alone");
    let resumed = change(&f, &s.id, |s| s.status = Status::Active);
    assert_eq!(resumed.status, Status::Active, "resuming what is still approved");

    let pending = change(&f, &s.id, |s| s.caps.max_tokens = 1);
    assert_eq!(pending.status, Status::Pending);
    let forced = change(&f, &s.id, |s| s.status = Status::Active);
    assert_eq!(forced.status, Status::Pending, "only Approve activates a changed schedule");
    let back = change(&f, &s.id, |s| {
        s.caps.max_tokens = 3_000_000;
        s.status = Status::Active;
    });
    assert_eq!(back.status, Status::Pending, "a covered edit is pending even when it restores the approved value");
}

#[test]
fn mac_binds_the_workspace_the_id_and_the_fingerprint() {
    let key = [7u8; 32];
    let a = mac(&key, "demo", "x", "ff");
    assert_eq!(a.len(), 64);
    assert_ne!(a, mac(&key, "demo2", "x", "ff"));
    assert_ne!(a, mac(&key, "demo", "y", "ff"));
    assert_ne!(a, mac(&key, "demo", "x", "fe"));
    let approval = Approval { fingerprint: "ff".into(), mac: a, approved_at: 0, covered: String::new(), command_sha256: None };
    assert_eq!(verify(&approval, "demo", "x", "ff", &key), Ok(()));
    assert_eq!(verify(&approval, "demo", "y", "ff", &key), Err(Refusal::BadMac));
    assert_eq!(verify(&approval, "demo", "x", "fe", &key), Err(Refusal::Changed));
}

/// Europe/Berlin under the EU rule: CEST from the last Sunday of March to the last Sunday of October, both
/// at 01:00 UTC.
#[derive(Debug, Clone, Copy)]
struct Berlin;

fn last_sunday(year: i32, month: u32) -> NaiveDate {
    let mut d = NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap().pred_opt().unwrap();
    while d.weekday() != chrono::Weekday::Sun {
        d = d.pred_opt().unwrap();
    }
    d
}

fn berlin_offset(utc: &NaiveDateTime) -> FixedOffset {
    let start = last_sunday(utc.year(), 3).and_hms_opt(1, 0, 0).unwrap();
    let end = last_sunday(utc.year(), 10).and_hms_opt(1, 0, 0).unwrap();
    FixedOffset::east_opt(if (start..end).contains(utc) { 7200 } else { 3600 }).unwrap()
}

impl TimeZone for Berlin {
    type Offset = FixedOffset;

    fn from_offset(_: &FixedOffset) -> Self {
        Berlin
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
        self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        let fits: Vec<FixedOffset> = [7200, 3600]
            .into_iter()
            .map(|s| FixedOffset::east_opt(s).unwrap())
            .filter(|o| berlin_offset(&(*local - chrono::TimeDelta::seconds(o.local_minus_utc() as i64))) == *o)
            .collect();
        match fits[..] {
            [] => LocalResult::None,
            [o] => LocalResult::Single(o),
            [a, b] => LocalResult::Ambiguous(a, b),
            _ => unreachable!(),
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        berlin_offset(&utc.and_hms_opt(12, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
        berlin_offset(utc)
    }
}

fn berlin(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Berlin> {
    Berlin.from_local_datetime(&NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()).earliest().unwrap()
}

fn utc(t: &DateTime<Berlin>) -> String {
    t.with_timezone(&Utc).format("%Y-%m-%d %H:%M").to_string()
}

fn daily(h: u32, m: u32) -> Trigger {
    Trigger::Daily { at: Clock::new(h, m).unwrap() }
}

#[test]
fn the_test_zone_is_berlins() {
    assert_eq!(berlin(2026, 1, 15, 12, 0).offset().local_minus_utc(), 3600);
    assert_eq!(berlin(2026, 7, 15, 12, 0).offset().local_minus_utc(), 7200);
    assert!(Berlin.from_local_datetime(&NaiveDate::from_ymd_opt(2026, 3, 29).unwrap().and_hms_opt(2, 30, 0).unwrap()).single().is_none());
    assert!(matches!(
        Berlin.from_local_datetime(&NaiveDate::from_ymd_opt(2026, 10, 25).unwrap().and_hms_opt(2, 30, 0).unwrap()),
        LocalResult::Ambiguous(..)
    ));
}

#[test]
fn daily_and_weekly_fires_keep_the_wall_clock_across_dst() {
    let t = daily(9, 0);
    assert_eq!(utc(&next_fire(&t, &berlin(2026, 3, 28, 10, 0)).unwrap()), "2026-03-29 07:00", "CEST: 09:00 is 07:00 UTC");
    assert_eq!(utc(&next_fire(&t, &berlin(2026, 10, 24, 10, 0)).unwrap()), "2026-10-25 08:00", "CET again");
    assert_eq!(utc(&next_fire(&t, &berlin(2026, 3, 29, 8, 59)).unwrap()), "2026-03-29 07:00");
    assert_eq!(utc(&next_fire(&t, &berlin(2026, 3, 29, 9, 0)).unwrap()), "2026-03-30 07:00", "strictly after");

    let w = Trigger::Weekly { days: vec![Day::Mon, Day::Fri], at: Clock::new(9, 0).unwrap() };
    let fri = berlin(2026, 10, 30, 9, 0); // a Friday
    assert_eq!(utc(&next_fire(&w, &fri).unwrap()), "2026-11-02 08:00", "the next Monday");
    assert_eq!(utc(&previous_fire(&w, &fri).unwrap()), "2026-10-30 08:00", "at or before");
    assert_eq!(utc(&previous_fire(&w, &berlin(2026, 11, 2, 8, 0)).unwrap()), "2026-10-30 08:00");
    let sun = Trigger::Weekly { days: vec![Day::Sun], at: Clock::new(12, 0).unwrap() };
    assert_eq!(utc(&next_fire(&sun, &berlin(2026, 3, 22, 12, 0)).unwrap()), "2026-03-29 10:00", "a week on, across spring");
}

#[test]
fn an_at_in_the_spring_gap_fires_at_the_first_minute_after_it() {
    let t = daily(2, 30);
    let fire = next_fire(&t, &berlin(2026, 3, 28, 12, 0)).unwrap();
    assert_eq!(utc(&fire), "2026-03-29 01:00");
    assert_eq!(fire.naive_local().format("%H:%M").to_string(), "03:00");
    assert_eq!(utc(&next_fire(&t, &fire).unwrap()), "2026-03-30 00:30", "back to 02:30 CEST the day after");
}

#[test]
fn an_at_the_autumn_change_repeats_fires_once() {
    let t = daily(2, 30);
    let first = next_fire(&t, &berlin(2026, 10, 24, 12, 0)).unwrap();
    assert_eq!(utc(&first), "2026-10-25 00:30", "the first 02:30, still CEST");
    assert_eq!(utc(&next_fire(&t, &first).unwrap()), "2026-10-26 01:30", "not the second 02:30 an hour later");
}

#[test]
fn interval_fires_step_from_at_through_the_day_and_across_dst() {
    let t = Trigger::Interval { at: Clock::new(9, 0).unwrap(), every: Span(7 * 3600) };
    let mut at = berlin(2026, 6, 1, 8, 0);
    let mut seen = Vec::new();
    for _ in 0..5 {
        at = next_fire(&t, &at).unwrap();
        seen.push(at.naive_local().format("%d %H:%M").to_string());
    }
    assert_eq!(seen, ["01 09:00", "01 16:00", "01 23:00", "02 06:00", "02 09:00"], "the cycle starts again at `at`");

    let q = Trigger::Interval { at: Clock::new(0, 0).unwrap(), every: Span(15 * 60) };
    let mut at = berlin(2026, 3, 29, 1, 40);
    let mut spring = Vec::new();
    for _ in 0..4 {
        at = next_fire(&q, &at).unwrap();
        spring.push(utc(&at));
    }
    assert_eq!(spring, ["2026-03-29 00:45", "2026-03-29 01:00", "2026-03-29 01:15", "2026-03-29 01:30"], "the gap collapses to 03:00 once");

    let mut at = berlin(2026, 10, 25, 1, 40);
    let mut autumn = Vec::new();
    for _ in 0..6 {
        at = next_fire(&q, &at).unwrap();
        autumn.push(utc(&at));
    }
    assert_eq!(
        autumn,
        ["2026-10-24 23:45", "2026-10-25 00:00", "2026-10-25 00:15", "2026-10-25 00:30", "2026-10-25 00:45", "2026-10-25 02:00"],
        "the repeated hour fires once"
    );
    assert_eq!(utc(&previous_fire(&q, &berlin(2026, 6, 1, 10, 7)).unwrap()), "2026-06-01 08:00");
}

#[test]
fn next_fire_local_uses_this_machines_zone() {
    let now = crate::time::now_ms();
    let next = next_fire_local(&daily(9, 0), now).unwrap();
    assert!(next > now && next <= now + 25 * 3600 * 1000, "{now} {next}");
}

fn record(run: &str, ts: i64, event: RunEvent) -> RunRecord {
    RunRecord { ts, run: run.into(), schedule: "autofix-prod-bugs".into(), event }
}

#[test]
fn run_records_read_the_doc_lines_and_fold_by_run() {
    let lines = [
        r#"{"ts":1793595600000,"run":"r1","schedule":"autofix-prod-bugs","event":"started","trigger":"task","fingerprint":"9f2c","fm_pid":4242}"#,
        r#"{"ts":1793595605321,"run":"r1","schedule":"autofix-prod-bugs","event":"stage","stage":"script","code":10,"ms":5321}"#,
        r#"{"ts":1793595606000,"run":"r1","schedule":"autofix-prod-bugs","event":"stage","stage":"session","name":"fm-nomos-sched-autofix-prod-bugs-20261102-090000","id":"948719b4","session_id":"948719b4-291b","worktree":"P:\\x\\.claude\\worktrees\\fm-sched-a","workstate_session":"7f3c"}"#,
        r#"{"ts":1793597430000,"run":"r1","schedule":"autofix-prod-bugs","event":"ended","outcome":"done","why":"fixed 2 bugs, PR #41","tokens":{"input":81234,"output":9120,"cache_creation":40211,"cache_read":1200443},"ms":1830000}"#,
    ];
    let parsed: Vec<RunRecord> = lines.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
    for (line, r) in lines.iter().zip(&parsed) {
        assert_eq!(serde_json::to_string(r).unwrap(), *line, "round trip");
    }
    let RunEvent::Ended { tokens: Some(tokens), outcome, .. } = &parsed[3].event else { panic!() };
    assert_eq!((tokens.capped(), outcome.succeeded(), outcome.needs_attention()), (130_565, true, false));
    assert!(Outcome::Refused.needs_attention() && !Outcome::Refused.succeeded());

    let f = fixture();
    for r in &parsed {
        append_run(&f.home, "demo", r).unwrap();
    }
    append_run(&f.home, "demo", &record("r2", 1, RunEvent::Started { trigger: Fired::RunNow, fingerprint: "x".into(), fm_pid: 1 })).unwrap();
    let mut file = fs::read_to_string(runs_path(&f.home, "demo").unwrap()).unwrap();
    file.push_str("not json\n");
    fs::write(runs_path(&f.home, "demo").unwrap(), file).unwrap();
    assert_eq!(read_runs(&f.home, "demo").unwrap().len(), 5, "a broken line is skipped");
    let folded = runs(&f.home, "demo").unwrap();
    assert_eq!(folded.len(), 2);
    assert_eq!(folded[0].started, Some((1793595600000, Fired::Task)));
    assert_eq!(folded[0].stages.len(), 2);
    assert_eq!(folded[0].ended, Some((1793597430000, Outcome::Done, "fixed 2 bugs, PR #41".into())));
    assert_eq!(folded[0].tokens.map(|t| t.capped()), Some(81234 + 9120 + 40211));
    assert_eq!((folded[1].started, &folded[1].ended), (Some((1, Fired::RunNow)), &None));
}

#[test]
fn run_records_rotate_at_one_mib_and_read_across_both_files() {
    let f = fixture();
    let ended = |i: i64| record(&format!("run-{i:05}"), i, RunEvent::Ended { outcome: Outcome::Ok, why: "x".repeat(200), tokens: None, ms: 1 });
    let mut n = 0;
    let path = runs_path(&f.home, "demo").unwrap();
    while !crate::fs::rotated(&path).exists() {
        append_run(&f.home, "demo", &ended(n)).unwrap();
        n += 1;
    }
    for _ in 0..3 {
        append_run(&f.home, "demo", &ended(n)).unwrap();
        n += 1;
    }
    assert!(fs::metadata(crate::fs::rotated(&path)).unwrap().len() >= RUNS_CAP);
    let all = read_runs(&f.home, "demo").unwrap();
    assert_eq!(all.len() as i64, n, "nothing lost across the rotation");
    assert!(all.windows(2).all(|w| w[0].ts < w[1].ts), "oldest first");
}

fn git(dir: &Path, args: &[&str]) {
    let out = crate::process::git(dir, args).unwrap();
    assert!(out.success(), "git {args:?}: {}", out.stderr);
}

#[test]
fn prune_takes_old_rotated_runs_the_state_of_removed_schedules_and_spent_worktrees() {
    let f = fixture();
    add(&f, example());
    Registry::at(&f.home).save(&f.ws, false).unwrap();
    let runs = f.home.join("schedules").join("runs");
    fs::create_dir_all(&runs).unwrap();
    for name in ["demo.jsonl", "demo.jsonl.1"] {
        fs::write(runs.join(name), "{}\n").unwrap();
    }
    let state = f.home.join("schedules").join("state").join("demo");
    for id in ["autofix-prod-bugs", "removed"] {
        fs::create_dir_all(state.join(id)).unwrap();
        fs::write(state.join(id).join("seen.txt"), "1").unwrap();
    }
    let unreadable = f.home.join("schedules").join("state").join("broken");
    fs::create_dir_all(unreadable.join("x")).unwrap();
    fs::write(f.home.join("schedules").join("broken.toml"), "not toml [").unwrap();

    git(&f.folder, &["init", "-q"]);
    git(&f.folder, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"]);
    let wts = f.folder.join(".claude").join("worktrees");
    let wt = |n: &str| wts.join(format!("fm-sched-autofix-prod-bugs-{n}"));
    for n in ["clean", "dirty", "ahead"] {
        git(&f.folder, &["worktree", "add", "-q", "-b", &format!("sched/x/{n}"), wt(n).to_str().unwrap(), "HEAD"]);
    }
    fs::write(wt("dirty").join("wip.txt"), "unsaved").unwrap();
    git(&wt("ahead"), &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "work"]);
    assert_eq!(run_worktrees(&f.home).len(), 3);

    let now = crate::time::now_ms();
    assert_eq!(crate::prune::prune(&f.home, now, crate::prune::KEEP_MS), Vec::<String>::new());
    assert!(runs.join("demo.jsonl.1").exists() && wt("clean").exists(), "nothing is old yet");
    assert!(!state.join("removed").exists(), "the state of a removed schedule goes");
    assert!(state.join("autofix-prod-bugs").exists() && unreadable.join("x").exists(), "kept, and kept when unsure");

    let later = now + crate::prune::KEEP_MS + 24 * 3600 * 1000;
    assert_eq!(crate::prune::prune(&f.home, later, crate::prune::KEEP_MS), Vec::<String>::new());
    assert!(!runs.join("demo.jsonl.1").exists() && runs.join("demo.jsonl").exists(), "only the rotated file ages out");
    assert!(!wt("clean").exists(), "a clean worktree with nothing of its own goes");
    let branches = crate::process::git(&f.folder, &["branch", "--list", "sched/*"]).unwrap().stdout;
    assert!(!branches.contains("sched/x/clean") && branches.contains("sched/x/dirty") && branches.contains("sched/x/ahead"), "{branches}");
    assert!(wt("dirty").join("wip.txt").exists(), "uncommitted work stays");
    assert!(wt("ahead").exists(), "a commit only it has stays");
    assert_eq!(run_worktrees(&f.home).len(), 2, "listed for René");
}

mod registrar;

#[test]
fn a_runs_session_name_is_a_valid_name_and_profiles_map_onto_claudes_flags() {
    let stamp = run_stamp(&chrono::Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).unwrap());
    assert_eq!(stamp, "20261102-090000");
    let name = session_name("nomos", "autofix-prod-bugs", &stamp);
    assert_eq!(name, "fm-nomos-sched-autofix-prod-bugs-20261102-090000");
    crate::validate_name(&name).unwrap();
    assert_eq!(run_id("autofix-prod-bugs", &stamp), "autofix-prod-bugs-20261102-090000");

    assert_eq!("worktree-edit".parse::<Profile>(), Ok(Profile::Edit), "the earlier name still reads");
    assert_eq!(("edit".parse::<Profile>(), Profile::Edit.as_str()), (Ok(Profile::Edit), "edit"));
    assert_eq!(Profile::Edit.permission_mode(), "acceptEdits");
    assert!("bypassPermissions".parse::<Profile>().unwrap_err().contains("never a schedule's mode"));
    let p = Permissions { profile: Profile::Read, allow_tools: vec!["Read".into(), "Bash(ls:*)".into()], deny_tools: vec!["WebFetch".into()], ..Permissions::default() };
    assert_eq!(p.allowed(&["Bash(sm:*)".into()], None), ["Bash(sm:*)", "Read", "Glob", "Grep", "Bash(ls:*)"]);
    let edit = Permissions { profile: Profile::Edit, ..Permissions::default() };
    let branch = run_branch("fix", &stamp);
    assert_eq!(branch, "fm-sched/fix/20261102-090000");
    assert_eq!(
        edit.allowed(&[], None),
        ["Read", "Glob", "Grep", "Edit", "Write", "Bash(git status:*)", "Bash(git diff:*)", "Bash(git log:*)", "Bash(git add:*)", "Bash(git commit:*)"],
        "an edit run in the checkout pushes nothing"
    );
    let in_worktree = edit.allowed(&[], Some(&branch));
    assert_eq!(in_worktree.iter().filter(|r| r.contains("push")).collect::<Vec<_>>(), ["Bash(git push -u origin fm-sched/fix/20261102-090000)"], "a run worktree's own branch only");
    assert!(!Permissions { profile: Profile::Auto, ..Permissions::default() }.allowed(&[], Some(&branch)).iter().any(|r| r.contains("push")));
    assert_eq!(p.denied().len(), DEFAULT_DENY.len() + 1);
}

#[test]
fn a_stderr_tail_is_masked_before_it_is_cut() {
    let secrets = vec!["s3cr3t".to_string(), String::new()];
    assert_eq!(masked_tail("token=s3cr3t done", &secrets, 2048), "token=*** done");
    let long = format!("{}s3cr3t", "x".repeat(5000));
    assert_eq!(masked_tail(&long, &secrets, 10), "xxxxxxx***");
    assert_eq!(masked_tail("aé", &[], 1), "", "never a cut inside a char");
}

#[test]
fn the_live_and_state_dirs_and_secret_keys_are_per_workspace_and_schedule() {
    let home = Path::new(r"C:\h");
    assert_eq!(live_dir(home, "demo").unwrap(), home.join("schedules").join("live").join("demo"));
    assert_eq!(state_dir(home, "demo", "fix").unwrap(), home.join("schedules").join("state").join("demo").join("fix"));
    assert!(state_dir(home, "demo", "../x").is_err() && live_dir(home, "..").is_err());
    assert_eq!(fire_log(home), home.join("schedules").join("fire.log"));
    assert_eq!(secret_key("demo", "fix", "API_TOKEN"), "foreman/schedule/demo/fix/API_TOKEN");
}

#[test]
fn a_runs_session_name_gives_back_its_schedule_id() {
    assert_eq!(schedule_of_session("nomos", "fm-nomos-sched-autofix-prod-bugs-20261102-090000"), Some("autofix-prod-bugs"));
    assert_eq!(schedule_of_session("nomos", "fm-nomos-sched-fix-20261102-090000-2"), Some("fix"), "a second run in the same second");
    assert_eq!(schedule_of_session("nomos", &session_name("nomos", "a-1", "20261102-090000")), Some("a-1"));
    for other in ["fm-nomos-sched-fix", "fm-nomos-sched--20261102-090000", "fm-nomos-alpha", "fm-other-sched-fix-20261102-090000", "fm-nomos-sched-fix-2026110-090000", "nomos-sched-fix-20261102-090000"] {
        assert_eq!(schedule_of_session("nomos", other), None, "{other}");
    }
}

#[test]
fn the_diff_names_each_covered_field_that_changed_since_the_approval() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    let approval = s.approval.clone().unwrap();
    let now = |s: &Schedule| covered(&f.ws, s).unwrap().json;
    assert!(covered_diff(&approval.covered, &now(&s)).is_empty(), "nothing changed");
    let fields = covered_fields(&approval.covered);
    assert!(fields.contains(&("trigger.at".into(), "09:00".into())), "{fields:?}");
    assert!(fields.contains(&("trigger.days".into(), r#"["mon"]"#.into())), "an array is one value: {fields:?}");
    assert!(fields.iter().any(|(k, _)| k == "files.scripts/gate.ps1"), "the script's hash is covered: {fields:?}");

    let s = change(&f, "autofix-prod-bugs", |s| {
        s.trigger = Trigger::Weekly { days: vec![Day::Mon, Day::Fri], at: "10:30".parse().unwrap() };
        s.caps.max_tokens = 1_000_000;
    });
    fs::write(f.folder.join("scripts").join("gate.ps1"), "exit 0").unwrap();
    let diff = covered_diff(&approval.covered, &now(&s));
    let shown: Vec<(&str, Option<&str>, Option<&str>)> = diff.iter().map(|c| (c.field.as_str(), c.before.as_deref(), c.after.as_deref())).collect();
    assert_eq!(shown.iter().map(|c| c.0).collect::<Vec<_>>(), ["action.worktree", "caps.max_tokens", "files.scripts/gate.ps1", "trigger.at", "trigger.days"]);
    assert_eq!(shown[0], ("action.worktree", Some("true"), Some("false")));
    assert_eq!(shown[1], ("caps.max_tokens", Some("3000000"), Some("1000000")));
    assert_eq!(shown[3], ("trigger.at", Some("09:00"), Some("10:30")));
    assert_eq!(shown[4], ("trigger.days", Some(r#"["mon"]"#), Some(r#"["mon","fri"]"#)));

    let removed = covered_diff(r#"{"a":1,"b":{"c":2}}"#, r#"{"a":1}"#);
    assert_eq!(removed, [Change { field: "b.c".into(), before: Some("2".into()), after: None }]);
    assert_eq!(covered_diff(r#"{"a":null}"#, r#"{"a":"x"}"#)[0].before.as_deref(), Some("none"));
    assert!(covered_fields("not json").is_empty());
}


/// The fingerprint of the doc example as the release before `edit` computed it.
const GOLDEN_FINGERPRINT: &str = "52cd87cf1e7c3ce22ccc64f9ea094dfeecbf02c6aa9b3510351505d88ab0061e";

#[test]
fn a_schedule_approved_by_the_earlier_release_keeps_its_fingerprint_and_its_worktree() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    let a = s.approval.clone().unwrap();
    assert_eq!(a.fingerprint, GOLDEN_FINGERPRINT, "{}", a.covered);
    assert!(a.covered.contains(r#""profile":"worktree-edit""#) && a.covered.contains(r#""worktree":true"#), "{}", a.covered);

    let renamed = change(&f, &s.id, |s| s.name = "Renamed".into());
    assert_eq!((renamed.status, renamed.action.worktree), (Status::Active, true), "an uncovered edit keeps both");
    assert!(verify_approval(&store, &f.ws, &renamed).is_ok());
    let text = fs::read_to_string(file_path(&f.home, "demo").unwrap()).unwrap();
    assert!(text.contains(r#"profile = "edit""#) && text.contains("worktree = true"), "{text}");
    assert_eq!(covered(&f.ws, &one(&f, &s.id)).unwrap().fingerprint, GOLDEN_FINGERPRINT, "saved as edit, fingerprinted as before");
}

#[test]
fn a_covered_edit_of_a_worktree_schedule_drops_the_worktree_and_goes_pending() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    let after = change(&f, &s.id, |s| s.caps.max_tokens = 1_000_000);
    assert_eq!((after.status, after.action.worktree), (Status::Pending, false));
    assert_eq!(after.approval, s.approval);
    let fields: Vec<String> = covered_diff(&s.approval.unwrap().covered, &covered(&f.ws, &after).unwrap().json).into_iter().map(|c| c.field).collect();
    assert_eq!(fields, ["action.worktree", "caps.max_tokens"]);
}

#[test]
fn a_schedule_above_read_without_a_worktree_is_valid() {
    let f = fixture();
    for profile in [Profile::Edit, Profile::Auto] {
        let mut s = example();
        s.action.worktree = false;
        s.permissions.profile = profile;
        assert_eq!(s.validate(&f.ws), Ok(()), "{profile:?}");
    }
}

#[test]
fn the_preamble_says_where_the_run_works() {
    let checkout = preamble("r", "sm", Path::new(r"C:\r\Nomos"), None);
    assert!(checkout.contains("You work in the folder's checkout `C:/r/Nomos`") && checkout.contains("make one yourself"), "{checkout}");
    let wt = preamble("r", "sm", Path::new(r"C:\r\Nomos\.claude\worktrees\fm-sched-x-1"), Some("fm-sched/x/1"));
    assert!(wt.contains("your own git worktree `C:/r/Nomos/.claude/worktrees/fm-sched-x-1`") && wt.contains("on branch `fm-sched/x/1`") && !wt.contains("checkout"), "{wt}");
}

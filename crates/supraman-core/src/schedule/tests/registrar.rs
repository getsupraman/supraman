use std::path::Path;

use super::*;

const ID: &str = "autofix-prod-bugs";

fn golden(name: &str, xml: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/schedule/tests/golden").join(name);
    if std::env::var_os("SUPRAMAN_BLESS").is_some() {
        fs::write(&path, xml).unwrap();
    }
    let want = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}; SUPRAMAN_BLESS=1 writes it", path.display()));
    assert_eq!(xml, want.replace("\r\n", "\n"), "{name}");
}

fn spec<'a>(s: &'a Schedule, level: RunLevel) -> TaskSpec<'a> {
    TaskSpec {
        user: FakeRegistrar::USER,
        level,
        smw: Path::new(r"C:\Program Files\Supraman\smw.exe"),
        home: Path::new(r"C:\Users\rene\AppData\Roaming\Supraman\"),
        folder: Path::new(r"C:\Projects\Nomos"),
        key: "nomos-1a2b3c4d",
        schedule: s,
        fingerprint: "9f2c00000000000000000000000000000000000000000000000000000000beef",
        start: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
    }
}

#[test]
fn a_weekly_task_that_wakes_and_catches_up_matches_its_golden_file() {
    let mut s = example();
    s.trigger = Trigger::Weekly { days: vec![Day::Fri, Day::Mon], at: Clock::new(9, 0).unwrap() };
    s.policy = Policy { overlap: Overlap::Skip, catch_up: CatchUp::Once, wake: true };
    golden("weekly.xml", &task_xml(&spec(&s, RunLevel::LeastPrivilege)));
}

#[test]
fn a_daily_task_that_neither_wakes_nor_catches_up_matches_its_golden_file() {
    let mut s = example();
    s.trigger = daily(6, 30);
    s.policy = Policy { overlap: Overlap::Skip, catch_up: CatchUp::Skip, wake: false };
    s.caps.max_runtime = Span(2 * 3600);
    golden("daily.xml", &task_xml(&spec(&s, RunLevel::HighestAvailable)));
}

#[test]
fn an_interval_task_matches_its_golden_file() {
    let mut s = example();
    s.trigger = Trigger::Interval { at: Clock::new(7, 15).unwrap(), every: Span(90 * 60) };
    golden("interval.xml", &task_xml(&spec(&s, RunLevel::LeastPrivilege)));
}

#[test]
fn the_digest_ignores_the_start_date_and_follows_everything_else() {
    let s = example();
    let base = spec(&s, RunLevel::LeastPrivilege);
    let xml = task_xml(&base);
    assert_eq!(version_of(&xml), Some(definition_digest(&base).as_str()));
    let later = TaskSpec { start: NaiveDate::from_ymd_opt(2027, 3, 1).unwrap(), ..base.clone() };
    assert_eq!(definition_digest(&later), definition_digest(&base));
    assert_ne!(task_xml(&later), xml);
    let elevated = TaskSpec { level: RunLevel::HighestAvailable, ..base.clone() };
    assert_ne!(definition_digest(&elevated), definition_digest(&base));
    let other = TaskSpec { fingerprint: "00", ..base.clone() };
    assert_ne!(definition_digest(&other), definition_digest(&base));
}

#[test]
fn the_default_home_has_the_supraman_folder_and_any_other_home_its_own() {
    let default = Path::new(r"C:\Users\rene\AppData\Roaming\Supraman");
    assert_eq!(task_root_for(default, Some(default)), r"\Supraman");
    assert_eq!(task_root_for(Path::new(r"c:/users/rene/appdata/roaming/supraman/"), Some(default)), r"\Supraman", "compared by path_key");
    let dev = task_root_for(Path::new(r"D:\dev-home"), Some(default));
    assert!(dev.starts_with(r"\Supraman-") && dev.len() == r"\Supraman-".len() + 8, "{dev}");
    assert_eq!(dev, task_root_for(Path::new(r"D:\dev-home"), None));
    assert_ne!(dev, task_root_for(Path::new(r"D:\other-home"), Some(default)));
}

fn target<'a>(f: &'a Fixture, store: &'a MemoryStore, level: RunLevel) -> SyncTarget<'a> {
    SyncTarget { home: &f.home, store, level, smw: Path::new(r"C:\Supraman\smw.exe"), now_ms: 1_793_341_200_000 }
}

fn second(f: &Fixture, store: &MemoryStore, id: &str) {
    let mut s = example();
    s.id = id.into();
    s.trigger = daily(3, 0);
    add(f, s);
    approve(&f.home, store, &f.folder, id, 1).unwrap();
}

#[test]
fn sync_writes_only_its_own_folder_and_leaves_every_other_task_alone() {
    let f = fixture();
    let store = MemoryStore::default();
    approved(&f, &store);
    let root = task_root(&f.home);
    let mine = format!(r"{root}\{}", folder_key(&f.folder));
    let fake = FakeRegistrar::default();
    let planted = [
        (format!(r"{root}\other"), "autofix-prod-bugs"),
        (root.clone(), "demo"),
        (format!(r"{root}-x\demo"), "autofix-prod-bugs"),
        (r"\Microsoft\Windows\Defrag".to_string(), "ScheduledDefrag"),
    ];
    for (folder, name) in &planted {
        fake.plant(folder, name, "<Task>someone else's</Task>");
    }

    let done = sync(&fake, &target(&f, &store, RunLevel::LeastPrivilege), &f.folder, &load(&f.home, &f.folder).unwrap()).unwrap();
    assert_eq!(done.folder, mine);
    assert_eq!(done.registered, [ID]);
    change(&f, ID, |s| s.status = Status::Paused);
    let done = sync(&fake, &target(&f, &store, RunLevel::LeastPrivilege), &f.folder, &load(&f.home, &f.folder).unwrap()).unwrap();
    assert_eq!(done.removed, [ID]);
    assert!(done.folder_deleted && !fake.has_folder(&mine));

    for (folder, name) in &planted {
        assert_eq!(fake.xml(&format!(r"{folder}\{name}")).as_deref(), Some("<Task>someone else's</Task>"), r"{folder}\{name}");
    }
    assert!(fake.has_folder(&root), "the root still holds another folder's");
    let writes = fake.writes();
    assert!(writes.iter().all(|w| w.ends_with(&format!(r"{mine}\{ID}")) || w == &format!("rmdir {mine}")), "{writes:?}");
}

#[test]
fn sync_all_removes_the_tasks_of_folders_without_schedules_and_of_the_earlier_workspaces_only() {
    let f = fixture();
    let store = MemoryStore::default();
    approved(&f, &store);
    let root = task_root(&f.home);
    let fake = FakeRegistrar::default();
    let kept = [(r"\Microsoft\Windows\Defrag".to_string(), "ScheduledDefrag"), (format!(r"{root}-x\demo"), ID), (root.clone(), "loose")];
    for (folder, name) in &kept {
        fake.plant(folder, name, "<Task>someone else's</Task>");
    }
    for (folder, name) in [(format!(r"{root}\demo"), ID), (format!(r"{root}\demo"), "nightly"), (format!(r"{root}\gone-1a2b3c4d"), "x")] {
        fake.plant(&folder, name, "<Task>Supraman's</Task>");
    }
    let (all, errors) = load_all(&f.home);
    assert!(errors.is_empty(), "{errors:?}");
    let synced = sync_all(&fake, &target(&f, &store, RunLevel::LeastPrivilege), &all).unwrap();
    let mine = format!(r"{root}\{}", folder_key(&f.folder));
    assert_eq!((synced.len(), synced[0].folder.as_str(), synced[0].registered.as_slice()), (1, mine.as_str(), [ID.to_string()].as_slice()));
    let mut paths = fake.paths();
    paths.sort();
    let mut want: Vec<String> = kept.iter().map(|(folder, name)| format!(r"{folder}\{name}")).chain([format!(r"{mine}\{ID}")]).collect();
    want.sort();
    assert_eq!(paths, want);
    assert!(!fake.has_folder(&format!(r"{root}\demo")) && !fake.has_folder(&format!(r"{root}\gone-1a2b3c4d")));
}

#[test]
fn pending_paused_and_changed_schedules_have_no_task() {
    let f = fixture();
    let store = MemoryStore::default();
    add(&f, example());
    let mut paused = example();
    paused.id = "paused".into();
    paused.status = Status::Paused;
    add(&f, paused);
    second(&f, &store, "changed");
    fs::write(f.folder.join("scripts").join("gate.ps1"), "exit 0").unwrap();
    let fake = FakeRegistrar::default();
    let all = load(&f.home, &f.folder).unwrap();
    assert_eq!(all.iter().find(|s| s.id == "changed").unwrap().status, Status::Active, "active in the file, but its script changed");

    let done = sync(&fake, &target(&f, &store, RunLevel::LeastPrivilege), &f.folder, &all).unwrap();
    assert!(done.registered.is_empty() && fake.paths().is_empty(), "{:?}", fake.paths());
    assert!(fake.writes().iter().all(|w| w.starts_with("rmdir")), "{:?}", fake.writes());
}

#[test]
fn an_unchanged_task_stays_and_a_changed_one_is_registered_again() {
    let f = fixture();
    let store = MemoryStore::default();
    approved(&f, &store);
    second(&f, &store, "nightly");
    let fake = FakeRegistrar::default();
    let level = RunLevel::LeastPrivilege;
    let first = sync(&fake, &target(&f, &store, level), &f.folder, &load(&f.home, &f.folder).unwrap()).unwrap();
    assert_eq!(first.registered, [ID, "nightly"]);
    let writes = fake.writes().len();

    let again = sync(&fake, &target(&f, &store, level), &f.folder, &load(&f.home, &f.folder).unwrap()).unwrap();
    assert!(again.registered.is_empty() && again.updated.is_empty() && again.removed.is_empty());
    assert_eq!(fake.writes().len(), writes, "nothing written when nothing changed");

    change(&f, "nightly", |s| s.trigger = daily(4, 0));
    approve(&f.home, &store, &f.folder, "nightly", 2).unwrap();
    let moved = sync(&fake, &target(&f, &store, level), &f.folder, &load(&f.home, &f.folder).unwrap()).unwrap();
    assert_eq!(moved.updated, ["nightly"]);
    let path = format!(r"{}\nightly", moved.folder);
    assert!(fake.xml(&path).unwrap().contains("T04:00:00</StartBoundary>"));
}

#[test]
fn a_change_of_the_services_elevation_registers_every_task_again() {
    let f = fixture();
    let store = MemoryStore::default();
    approved(&f, &store);
    second(&f, &store, "nightly");
    let fake = FakeRegistrar::default();
    let all = load(&f.home, &f.folder).unwrap();
    sync(&fake, &target(&f, &store, RunLevel::LeastPrivilege), &f.folder, &all).unwrap();

    let up = sync(&fake, &target(&f, &store, RunLevel::HighestAvailable), &f.folder, &all).unwrap();
    assert_eq!(up.level, RunLevel::HighestAvailable);
    assert_eq!(up.updated, [ID, "nightly"]);
    for path in fake.paths() {
        assert!(fake.xml(&path).unwrap().contains("<RunLevel>HighestAvailable</RunLevel>"), "{path}");
    }
    let down = sync(&fake, &target(&f, &store, RunLevel::LeastPrivilege), &f.folder, &all).unwrap();
    assert_eq!(down.updated, [ID, "nightly"]);
    assert!(fake.xml(&format!(r"{}\{ID}", down.folder)).unwrap().contains("<RunLevel>LeastPrivilege</RunLevel>"));
}

#[test]
fn the_task_runs_smw_with_the_home_the_folder_key_the_id_and_the_approved_fingerprint() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    let fake = FakeRegistrar::default();
    let done = sync(&fake, &target(&f, &store, RunLevel::LeastPrivilege), &f.folder, &[s.clone()]).unwrap();
    let xml = fake.xml(&format!(r"{}\{ID}", done.folder)).unwrap();
    let fp = &s.approval.unwrap().fingerprint;
    let home = f.home.to_string_lossy();
    let key = folder_key(&f.folder);
    assert!(xml.contains(&format!("<Arguments>schedule fire --home &quot;{home}&quot; {key} {ID} --fingerprint {fp}</Arguments>")), "{xml}");
    assert!(xml.contains(r"<Command>C:\Supraman\smw.exe</Command>"));
    assert!(xml.contains(&format!("<UserId>{}</UserId>", FakeRegistrar::USER)));
}

#[test]
fn the_window_registers_smw_beside_its_own_exe() {
    let smw = smw_exe();
    assert_eq!(smw.file_name().and_then(|n| n.to_str()), Some("smw.exe"));
    assert_eq!(smw.parent(), std::env::current_exe().unwrap().parent());
}

#[test]
fn a_new_task_starts_on_its_first_fire_after_now() {
    let f = fixture();
    let store = MemoryStore::default();
    let s = approved(&f, &store);
    let fake = FakeRegistrar::default();
    let t = target(&f, &store, RunLevel::LeastPrivilege);
    let done = sync(&fake, &t, &f.folder, &[s.clone()]).unwrap();
    let first = next_fire_local(&s.trigger, t.now_ms).unwrap();
    let day = chrono::Local.timestamp_millis_opt(first).unwrap().format("%Y-%m-%d").to_string();
    let xml = fake.xml(&format!(r"{}\{ID}", done.folder)).unwrap();
    assert!(xml.contains(&format!("<StartBoundary>{day}T09:00:00</StartBoundary>")), "{xml}");
}

/// The real Task Scheduler: registers `\SupramanProbe-sched-3\probe`, reads it back with its settings,
/// removes it and deletes the folder. Never in the gate (C101); run it by hand with `--ignored`.
#[cfg(windows)]
#[test]
#[ignore = "registers a real Windows task under \\SupramanProbe-sched-3\\"]
fn a_real_task_registers_reads_back_and_goes_with_its_folder() {
    const FOLDER: &str = r"\SupramanProbe-sched-3";
    let tasks = crate::platform::tasks::WindowsRegistrar;
    let mut s = example();
    s.id = "probe".into();
    s.trigger = Trigger::Weekly { days: vec![Day::Sun], at: Clock::new(3, 33).unwrap() };
    s.policy.wake = true;
    let user = tasks.user().unwrap();
    let start = chrono::Local::now().date_naive() + chrono::Days::new(400);
    let spec = TaskSpec {
        user: &user,
        level: RunLevel::LeastPrivilege,
        smw: Path::new(r"C:\fmbuild\sched-3\no-such-smw.exe"),
        home: Path::new(r"C:\fmbuild\sched-3\probe-home"),
        folder: Path::new(r"C:\fmbuild\sched-3\probe"),
        key: "probe-00000000",
        schedule: &s,
        fingerprint: "00ff",
        start,
    };
    assert!(tasks.tasks(FOLDER).unwrap().is_empty(), "{FOLDER} is not empty before the probe");
    let removed = std::panic::catch_unwind(|| {
        tasks.register(FOLDER, "probe", &task_xml(&spec)).unwrap();
        let back = tasks.tasks(FOLDER).unwrap();
        assert_eq!(back.len(), 1);
        let (name, xml) = &back[0];
        println!("read back {FOLDER}\\{name}:\n{xml}");
        assert_eq!(name, "probe");
        assert_eq!(version_of(xml), Some(definition_digest(&spec).as_str()));
        assert!(!xml.contains("<RunLevel>"), "LeastPrivilege is the default, which Task Scheduler leaves out");
        for want in [
            "<LogonType>InteractiveToken</LogonType>",
            "<StartWhenAvailable>true</StartWhenAvailable>",
            "<WakeToRun>true</WakeToRun>",
            "<MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>",
            "<Sunday />",
            "T03:33:00</StartBoundary>",
            r"<Command>C:\fmbuild\sched-3\no-such-smw.exe</Command>",
        ] {
            assert!(xml.contains(want), "{want} missing");
        }
        let elevated = TaskSpec { level: RunLevel::HighestAvailable, ..spec.clone() };
        tasks.register(FOLDER, "probe", &task_xml(&elevated)).unwrap();
        let back = tasks.tasks(FOLDER).unwrap();
        let (_, xml) = &back[0];
        assert_eq!(back.len(), 1);
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"), "{xml}");
        assert_eq!(version_of(xml), Some(definition_digest(&elevated).as_str()));
        println!("registered again at HighestAvailable: read back <RunLevel>HighestAvailable</RunLevel>, version {}", definition_digest(&elevated));
    });
    for (name, _) in tasks.tasks(FOLDER).unwrap() {
        tasks.delete(FOLDER, &name).unwrap();
    }
    let gone = tasks.delete_empty_folder(FOLDER).unwrap();
    println!("tasks left in {FOLDER}: {}; folder deleted: {gone}", tasks.tasks(FOLDER).unwrap().len());
    assert!(gone, "{FOLDER} is still there");
    if let Err(e) = removed {
        std::panic::resume_unwind(e);
    }
}

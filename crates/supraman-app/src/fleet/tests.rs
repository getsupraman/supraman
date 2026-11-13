use super::*;
use supraman_claude::parse;

const NOW: i64 = 1_800_000_000_000;

fn group(name: &str, paths: &[&str]) -> Entry {
    Entry::Group { name: name.into(), folders: paths.iter().map(|p| FolderEntry::new(*p)).collect() }
}

fn top(path: &str) -> Entry {
    Entry::Folder(FolderEntry::new(path))
}

#[test]
fn shell_cwd_is_the_deepest_listed_folder_then_the_first_then_home() {
    let entries = Entries::new(vec![top("P:/Projects/A"), group("b", &[r"P:\Projects\B", r"P:\Projects\B\inner"])]);
    let home = Some(PathBuf::from(r"C:\Users\me"));
    let cwd = |active: Option<&str>, entries: &Entries| shell_cwd(active.map(Path::new), entries, &Shown::new(), home.clone());
    let p = |s: &str| Some(PathBuf::from(s));
    assert_eq!(cwd(Some(r"p:\projects\b\repo\src"), &entries), p(r"P:\Projects\B"), "another drive-letter case");
    assert_eq!(cwd(Some(r"P:\Projects\A\x"), &entries), p("P:/Projects/A"), "forward-slash path");
    assert_eq!(cwd(Some(r"P:\Projects\B\inner\r"), &entries), p(r"P:\Projects\B\inner"), "innermost folder");
    assert_eq!(cwd(Some(r"P:\Projects\Bee"), &entries), p("P:/Projects/A"), "a sibling with a common prefix is not inside");
    assert_eq!(cwd(None, &entries), p("P:/Projects/A"), "no active tab: the first folder");
    assert_eq!(cwd(Some(r"D:\elsewhere"), &Entries::default()), home, "no folder: home");
}

fn entries() -> Entries {
    Entries::new(vec![group("demo", &[r"C:\demo\Alpha", r"C:\demo\Beta"])])
}

fn sessions() -> Vec<Session> {
    let old = NOW - 3_600_000;
    parse(&format!(
        r#"[
  {{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"busy","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"working","status":"busy","pid":1}},
  {{"id":"aaaa0002","sessionId":"aaaa0002-x","name":"parked","cwd":"C:\\demo\\Alpha\\src","kind":"background","startedAt":{old},"state":"blocked","waitingFor":"permission prompt","pid":2}},
  {{"id":"aaaa0003","sessionId":"aaaa0003-x","name":"gone","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"blocked"}},
  {{"id":"aaaa0004","sessionId":"aaaa0004-x","name":"done","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"stopped"}},
  {{"sessionId":"bbbb0005-x","name":"term","cwd":"C:\\demo\\Beta","kind":"interactive","startedAt":{old},"status":"idle","pid":5}},
  {{"id":"aaaa0006","sessionId":"aaaa0006-x","name":"elsewhere","cwd":"C:\\other","kind":"background","startedAt":{old},"state":"working","status":"idle","pid":6}}
]"#
    ))
    .unwrap()
}

fn names(folder: &FolderNode) -> Vec<&str> {
    folder.sessions.iter().map(|r| r.session.name.as_str()).collect()
}

fn demo(tree: &Tree) -> &EntryNode {
    &tree.entries[0]
}

#[test]
fn a_polls_main_repos_table_places_worktree_sessions_without_touching_the_disk() {
    // C:\other has no .git, so only the table can say it is a worktree of Beta
    let known = HashMap::from([(PathBuf::from(r"C:\other"), Some(PathBuf::from("C:/demo/Beta")))]);
    let sup = TreeInputs { main_repos: Some(&known), ..TreeInputs::default() };
    let tree = build(&entries(), &sessions(), NOW, false, &sup);
    assert!(names(&demo(&tree).folders[1]).contains(&"elsewhere"));
    let tree = build(&entries(), &sessions(), NOW, false, &TreeInputs::default());
    assert!(!names(&demo(&tree).folders[1]).contains(&"elsewhere"));
}

#[test]
fn groups_folders_blocked_first_and_hides_stopped() {
    let tree = build(&entries(), &sessions(), NOW, false, &TreeInputs::default());
    let d = demo(&tree);
    assert!(d.group);
    let keys: Vec<_> = d.folders.iter().map(|r| (r.key.as_str(), r.name.as_str())).collect();
    assert_eq!(keys, [("demo/Alpha", "Alpha"), ("demo/Beta", "Beta")]);
    assert_eq!(names(&d.folders[0]), ["parked", "busy"]);
    assert_eq!(names(&d.folders[1]), ["term", "gone"]);
    assert_eq!(tree.hidden_stopped, 1);
    assert_eq!(d.session_count(), 4);
}

#[test]
fn a_folders_kind_and_branch_come_from_the_polls_table() {
    let disk = Disk { kinds: HashMap::from([(path_key(Path::new(r"C:\demo\Alpha")), OnDisk { kind: FolderKind::Git, branch: Some("main".into()) })]), ..Disk::default() };
    let tree = build(&entries(), &[], NOW, false, &TreeInputs { disk: Some(&disk), ..TreeInputs::default() });
    let (alpha, beta) = (&demo(&tree).folders[0], &demo(&tree).folders[1]);
    assert_eq!((alpha.kind, alpha.branch.as_deref()), (FolderKind::Git, Some("main")));
    assert_eq!((beta.kind, beta.branch.as_deref()), (FolderKind::Plain, None), "not read yet");
}

#[test]
fn show_stopped_keeps_them_last() {
    let tree = build(&entries(), &sessions(), NOW, true, &TreeInputs::default());
    assert_eq!(names(&demo(&tree).folders[1]), ["term", "gone", "done"]);
    assert_eq!(tree.hidden_stopped, 0);
}

#[test]
fn attach_only_for_live_background_sessions() {
    let tree = build(&entries(), &sessions(), NOW, true, &TreeInputs::default());
    let flags: Vec<(&str, bool)> = tree.rows().map(|r| (r.session.name.as_str(), r.attachable)).collect();
    assert_eq!(flags, [("parked", true), ("busy", true), ("term", false), ("gone", false), ("done", false)]);
}

#[test]
fn a_rows_one_open_action_follows_tab_then_attach_then_resume_and_refuses_the_rest() {
    let tree = build(&entries(), &sessions(), NOW, true, &TreeInputs::default());
    let row = |name: &str| tree.rows().find(|r| r.session.name == name).unwrap().clone();
    let (busy, term, done, gone) = (row("busy"), row("term"), row("done"), row("gone"));
    assert_eq!(Open::of(&busy, None, None, false), Open::Attach);
    assert_eq!(Open::of(&busy, Some(3), Some(1), false), Open::Tab(3), "open in a tab beats attach");
    assert_eq!(Open::of(&busy, Some(3), Some(3), false), Open::Shown);
    assert_eq!(Open::of(&done, None, None, true), Open::Resume);
    assert_eq!(Open::of(&gone, None, None, true), Open::Resume, "interrupted resumes");
    assert_eq!(Open::of(&done, None, None, false), Open::Nothing);
    assert_eq!(Open::of(&term, None, None, false), Open::Elsewhere, "live interactive outside a tab");
    assert_eq!(Open::of(&term, Some(2), None, false), Open::Tab(2));

    let mut foreign = busy.clone();
    foreign.foreign = true;
    assert_eq!(Open::of(&foreign, None, None, true), Open::Foreign);
    assert_eq!(Open::of(&foreign, Some(3), None, true), Open::Foreign, "nothing for another Supraman's, even in a tab");
    assert_eq!(Open::Foreign.hint(), "another Supraman session");
    let runs: Vec<bool> = [Open::Tab(1), Open::Attach, Open::Resume, Open::Shown, Open::Foreign, Open::Elsewhere, Open::Unlisted, Open::Nothing].map(Open::runs).into();
    assert_eq!(runs, [true, true, true, false, false, false, false, false], "only a tab, an attach or a resume opens");
}

#[test]
fn labels() {
    let tree = build(&entries(), &sessions(), NOW, true, &TreeInputs::default());
    let beta = &demo(&tree).folders[1];
    assert_eq!(State::of_row(&beta.sessions[1]).word(), "interrupted");
    let alpha = &demo(&tree).folders[0];
    assert_eq!(State::of_row(&alpha.sessions[0]).word(), "waiting");
    assert_eq!(alpha.sessions[0].health, Health::Blocked { waiting_for: Some("permission prompt".into()) });
    assert_eq!(short_id(&beta.sessions[0].session), "bbbb0005");
}

#[test]
fn supraman_session_waiting_on_a_peer_is_idle_a_parked_one_blocked() {
    let old = NOW - 3_600_000;
    let rows = parse(&format!(
        r#"[
  {{"id":"aaaa0011","sessionId":"aaaa0011-x","name":"fm-demo-alpha","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"blocked","pid":11}},
  {{"id":"aaaa0012","sessionId":"aaaa0012-x","name":"fm-demo-beta","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":12}},
  {{"id":"aaaa0013","sessionId":"aaaa0013-x","name":"mine","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"blocked","pid":13}}
]"#
    ))
    .unwrap();
    let own = ["demo".to_string()];
    let tree = build(&entries(), &rows, NOW, false, &TreeInputs { own_names: &own, ..TreeInputs::default() });
    let health: Vec<(&str, &Health)> = tree.rows().map(|r| (r.session.name.as_str(), &r.health)).collect();
    let parked = Health::Blocked { waiting_for: Some("permission prompt".into()) };
    let asks = Health::Blocked { waiting_for: None };
    assert_eq!(health, [("fm-demo-alpha", &Health::Idle), ("fm-demo-beta", &parked), ("mine", &asks)]);

    let mut hooks = HashMap::new();
    let mut prompt = event("Notification", Some("permission_prompt"), NOW);
    prompt.session_id = "aaaa0011-x".into();
    HookSignal::record(&mut hooks, prompt);
    let sup = TreeInputs { hooks: Some(&hooks), polled_at: NOW - 1_000, own_names: &own, ..TreeInputs::default() };
    let tree = build(&entries(), &rows, NOW, false, &sup);
    assert_eq!(demo(&tree).folders[0].sessions[0].health, parked, "the hook's permission prompt still blocks it");
}

#[test]
fn another_homes_fm_rows_are_foreign_shown_and_never_ranked_blocked() {
    let own = ["demo".to_string()];
    let foreign = |name: &str| is_foreign(name, &own, None);
    assert!(foreign("fm-selftest-abc123-park"), "a selftest's session");
    assert!(foreign("fm-other-sched-nightly-20261113-0900"), "another home's run");
    assert!(!foreign("fm-demo-sched-nightly-20261113-0900"), "named after a workspace of this home's schedules");
    let run = |name: &str| is_foreign(name, &own, Some("fm-selftest-abc123"));
    assert!(!run("fm-selftest-abc123") && !run("fm-selftest-abc123-park"), "this selftest run's own");
    assert!(run("fm-selftest-def456-park"), "another selftest run's");
    assert!(run("fm-selftest-abc1234"), "a longer hex is another run");
    assert!(!foreign("mine"), "the user's own session");

    let old = NOW - 3_600_000;
    let rows = parse(&format!(
        r#"[
  {{"id":"aaaa0021","sessionId":"aaaa0021-x","name":"fm-selftest-abc123-park","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":21}},
  {{"id":"aaaa0022","sessionId":"aaaa0022-x","name":"mine","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{},"state":"working","status":"busy","pid":22}},
  {{"id":"aaaa0023","sessionId":"aaaa0023-x","name":"fm-demo-alpha","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":23}}
]"#,
        NOW - 1_000
    ))
    .unwrap();
    let tree = build(&entries(), &rows, NOW, false, &TreeInputs { own_names: &own, ..TreeInputs::default() });
    let alpha: Vec<(&str, bool)> = demo(&tree).folders[0].sessions.iter().map(|r| (r.session.name.as_str(), r.foreign)).collect();
    assert_eq!(alpha, [("fm-demo-alpha", false), ("mine", false), ("fm-selftest-abc123-park", true)], "own blocked first, the foreign one as running");
    let park = &demo(&tree).folders[0].sessions[2];
    assert_eq!(park.health, Health::Blocked { waiting_for: Some("permission prompt".into()) }, "its health is still shown");
}

fn closed(id: &str, name: &str, cwd: &str, age_ms: i64) -> Session {
    supraman_claude::Transcript { session_id: id.into(), cwd: Some(cwd.into()), custom_title: Some(name.into()), modified_ms: NOW - age_ms, ..Default::default() }.to_session()
}

#[test]
fn a_continued_session_shows_as_one_row_its_newest() {
    const OLD: &str = "0a000000-0000-4000-8000-00000000000a";
    const NEW: &str = "0b000000-0000-4000-8000-00000000000b";
    const LOST: &str = "0c000000-0000-4000-8000-00000000000c";
    let continued = HashMap::from([(OLD.to_string(), NEW.to_string()), (LOST.to_string(), "0d000000-0000-4000-8000-00000000000d".to_string())]);
    let rows = vec![closed(OLD, "workman", r"C:\demo\alpha", 0), closed(LOST, "other", r"C:\demo\alpha", 0)];
    let live = Session { kind: Kind::Background, state: Some(supraman_claude::SessionState::Blocked), ..closed(NEW, "workman", r"C:\demo\alpha", 0) };
    let shown: Vec<String> = without_continued(rows.clone(), &continued, std::slice::from_ref(&live)).into_iter().map(|s| s.session_id).collect();
    assert_eq!(shown, [LOST], "the old row goes while its continuation is listed; a chain to no known session keeps its row");
    assert_eq!(without_continued(rows.clone(), &continued, &[]).len(), 2, "its continuation not listed: the row stays");
    let both = vec![rows[0].clone(), closed(NEW, "workman", r"C:\demo\alpha", 0)];
    assert_eq!(without_continued(both, &continued, &[]).iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(), [NEW], "both closed: the newest");
}

#[test]
fn closed_sessions_show_with_stopped_newest_first_capped_and_starred_ones_always() {
    const DAY: i64 = 24 * 3_600_000;
    let mut gone = vec![
        closed("cccc0001-x", "yesterday", r"C:\demo\Beta", DAY),
        closed("bbbb0005-x", "term again", r"C:\demo\Beta", DAY),
        closed("cccc0002-x", "old", r"C:\demo\Beta", 20 * DAY),
        closed("cccc0003-x", "fm-selftest-abc123", r"C:\demo\Beta", DAY),
    ];
    gone.extend((0..7).map(|i| closed(&format!("dddd000{i}-x"), &format!("alpha{i}"), r"C:\demo\Alpha", (i + 1) * 3_600_000)));
    let sup = |starred, expanded| TreeInputs { closed: &gone, starred, expanded, ..TreeInputs::default() };

    let tree = build(&entries(), &sessions(), NOW, true, &sup(None, None));
    let beta = &demo(&tree).folders[1];
    assert_eq!(names(beta), ["term", "gone", "done", "yesterday"], "listed by claude agents once; not past the window; no fm- transcript");
    assert!(beta.sessions[3].closed && !beta.sessions[0].closed);
    assert_eq!(beta.sessions[3].health, Health::Stopped);
    assert_eq!(beta.sessions[3].session.kind, Kind::Interactive);
    let alpha = &demo(&tree).folders[0];
    assert_eq!(names(alpha), ["parked", "busy", "alpha0", "alpha1", "alpha2", "alpha3", "alpha4"], "newest first, capped");
    assert_eq!(alpha.more, 2);

    let expanded = HashSet::from(["demo/Alpha".to_string()]);
    let tree = build(&entries(), &sessions(), NOW, true, &sup(None, Some(&expanded)));
    assert_eq!(demo(&tree).folders[0].sessions.len(), 9, "more… shows the rest");
    assert_eq!(demo(&tree).folders[0].more, 0);

    let hidden = build(&entries(), &sessions(), NOW, false, &sup(None, None));
    assert_eq!(names(&demo(&hidden).folders[1]), ["term", "gone"]);
    assert_eq!(hidden.hidden_stopped, 1 + 1 + 7, "done, yesterday and Alpha's seven");

    let starred = BTreeSet::from(["cccc0002-x".to_string(), "aaaa0004-x".to_string(), "dddd0006-x".to_string()]);
    let tree = build(&entries(), &sessions(), NOW, false, &sup(Some(&starred), None));
    assert_eq!(names(&demo(&tree).folders[1]), ["term", "gone", "done", "old"], "stopped hidden, yet the starred stopped and 20-day-old ones stay");
    assert!(demo(&tree).folders[1].sessions[2].starred);
    assert_eq!(names(&demo(&tree).folders[0]), ["parked", "busy", "alpha6"]);
    let tree = build(&entries(), &sessions(), NOW, true, &sup(Some(&starred), None));
    let alpha = &demo(&tree).folders[0];
    assert_eq!(names(alpha).len(), 2 + 1 + CLOSED_CAP, "a starred one is not counted in the cap");
    assert_eq!(alpha.more, 1);
}

#[test]
fn no_entries_is_empty() {
    let tree = build(&Entries::default(), &sessions(), NOW, false, &TreeInputs::default());
    assert!(tree.entries.is_empty());
}

fn event(event: &str, detail: Option<&str>, ts: i64) -> Event {
    let mut e = Event::from_payload(event, r#"{"session_id":"aaaa0001-x"}"#, ts).unwrap();
    match event {
        "SessionStart" => e.source = detail.map(Into::into),
        _ => e.notification_type = detail.map(Into::into),
    }
    e
}

fn rows(list: &[(&str, &str, &str)]) -> Vec<Session> {
    let rows: Vec<serde_json::Value> = list
        .iter()
        .map(|(id, name, cwd)| {
            serde_json::json!({"id": id, "sessionId": format!("{id}-x"), "name": name, "cwd": cwd, "kind": "background",
                "startedAt": NOW - 1_000, "state": "working", "status": "idle", "pid": 1})
        })
        .collect();
    parse(&serde_json::Value::Array(rows).to_string()).unwrap()
}

#[test]
fn a_scheduled_runs_session_sits_under_its_schedule_in_the_folder_of_its_key() {
    let (gaia, skills) = (supraman_core::schedule::folder_key(Path::new(r"P:\L\Gaia")), supraman_core::schedule::folder_key(Path::new(r"P:\L\Skills")));
    let names = [format!("fm-{gaia}-sched-nightly-20261102-090000"), format!("fm-{gaia}-sched-nightly-20261101-090000"), format!("fm-{skills}-sched-weekly-fix-20261102-090000-2")];
    let mut list = rows(&[
        ("eeee0001", names[0].as_str(), r"P:\L\Gaia\.claude\worktrees\fm-sched-nightly-20261102-090000"),
        ("eeee0002", names[1].as_str(), r"P:\L\Gaia"),
        ("eeee0003", names[2].as_str(), r"P:\L\Elsewhere"),
        ("eeee0004", "mine", r"P:\L\Gaia"),
        ("eeee0005", "fm-other-1a2b3c4d-sched-x-20261102-090000", r"P:\L\Gaia"),
    ]);
    list[1].state = Some(supraman_claude::SessionState::Stopped);
    list[1].pid = None;
    let entries = Entries::new(vec![group("larchsys", &[r"P:\L\Gaia", r"P:\L\Skills"])]);
    let tree = build(&entries, &list, NOW, false, &TreeInputs::default());
    let w = demo(&tree);
    let ids = |f: &FolderNode| f.scheduled.iter().map(|(id, r)| (id.clone(), r.iter().map(|r| r.session.id.clone()).collect::<Vec<_>>())).collect::<Vec<_>>();
    assert_eq!(ids(&w.folders[0]), [("nightly".to_string(), vec!["eeee0001".to_string()])], "the stopped run is hidden with stopped rows");
    assert_eq!(ids(&w.folders[1]), [("weekly-fix".to_string(), vec!["eeee0003".to_string()])], "by its key, wherever its cwd");
    assert_eq!(tree.hidden_stopped, 1);
    let plain: Vec<&str> = w.folders[0].sessions.iter().map(|s| s.session.id.as_str()).collect();
    assert_eq!(plain, ["eeee0004", "eeee0005"], "another home's run sits by its cwd");
    assert_eq!(w.session_count(), 4);
    let shown = build(&entries, &list, NOW, true, &TreeInputs::default());
    assert_eq!(demo(&shown).folders[0].scheduled["nightly"].len(), 2);
}

#[test]
fn hook_signal_labels_and_early_block() {
    let mut hooks = HashMap::new();
    HookSignal::record(&mut hooks, event("SessionStart", Some("startup"), NOW - 300_000));
    HookSignal::record(&mut hooks, event("SessionStart", Some("compact"), NOW - 120_000));
    HookSignal::record(&mut hooks, event("Stop", None, NOW - 5_000));
    let h = &hooks["aaaa0001-x"];
    assert_eq!(h.label(NOW), "turn ended 5 s ago · compacted 2 min ago");
    let polled_at = NOW - 1_000;
    assert_eq!(with_hook(Health::Running, Some(h), polled_at), Health::Running);

    HookSignal::record(&mut hooks, event("Notification", Some("permission_prompt"), NOW));
    let h = &hooks["aaaa0001-x"];
    assert_eq!(h.label(NOW + 61_000), "permission prompt 1 min ago · compacted 3 min ago");
    let blocked = Health::Blocked { waiting_for: Some("permission prompt".into()) };
    assert_eq!(with_hook(Health::Running, Some(h), polled_at), blocked, "newer than the poll");
    assert_eq!(with_hook(Health::Running, Some(h), NOW + 1), Health::Running, "the poll is newer: trust it");
    assert_eq!(with_hook(Health::Stopped, Some(h), polled_at), Health::Stopped);

    let sup = TreeInputs { hooks: Some(&hooks), polled_at, ..TreeInputs::default() };
    let tree = build(&entries(), &sessions(), NOW, false, &sup);
    let busy = tree.rows().find(|r| r.session.name == "busy").unwrap();
    assert_eq!(busy.health, blocked);
    assert!(busy.hook.is_some());
}

#[test]
fn live_sessions_show_the_sidebars_state() {
    let list = sessions();
    let activity_of = |hooks: &HashMap<String, HookSignal>, polled_at: i64| live_sessions(&list, NOW, hooks, polled_at).into_iter().find(|s| s.name == "busy").unwrap().activity;
    let health_of = |hooks: &HashMap<String, HookSignal>, polled_at: i64| {
        let sup = TreeInputs { hooks: Some(hooks), polled_at, ..TreeInputs::default() };
        let tree = build(&entries(), &list, NOW, false, &sup);
        tree.rows().find(|r| r.session.name == "busy").unwrap().health.clone()
    };
    let mut hooks = HashMap::new();
    HookSignal::record(&mut hooks, event("Notification", Some("permission_prompt"), NOW - 500));

    let blocked = Health::Blocked { waiting_for: Some("permission prompt".into()) };
    assert_eq!(activity_of(&hooks, NOW - 1_000), Activity::Waiting, "a prompt newer than the poll");
    assert_eq!(health_of(&hooks, NOW - 1_000), blocked);
    assert_eq!(activity_of(&hooks, NOW), Activity::Working, "the poll is newer: trust it");
    assert_eq!(health_of(&hooks, NOW), Health::Running);
}

fn folders(tree: &Tree) -> Vec<(String, Vec<String>)> {
    tree.folders().map(|(_, f)| (f.key.clone(), f.sessions.iter().map(|r| format!("{}{}", r.session.name, r.scope)).collect())).collect()
}

fn f(key: &str, sessions: &[&str]) -> (String, Vec<String>) {
    (key.to_string(), sessions.iter().map(|s| s.to_string()).collect())
}

#[test]
fn a_session_shows_under_the_deepest_listed_folder_never_its_parent() {
    let entries = Entries::new(vec![group("larchsys", &[r"P:\L"]), group("gaia", &[r"P:\L\gaia", r"P:\L\gaia\api"])]);
    let list = rows(&[("aaaa0101", "ui", r"P:\L\gaia\src"), ("aaaa0102", "api", r"P:\L\gaia\api\x"), ("aaaa0103", "loose", r"P:\L\Other")]);
    let tree = build(&entries, &list, NOW, false, &TreeInputs::default());
    assert_eq!(folders(&tree), [f("larchsys/L", &["loose"]), f("gaia/gaia", &["ui"]), f("gaia/api", &["api"])]);
}

#[test]
fn a_path_listed_twice_shows_its_sessions_in_both_rows_with_no_tag() {
    let entries = Entries::new(vec![top(r"P:\L\gaia"), group("larchsys", &[r"P:\L", r"P:\L\gaia"]), top(r"Q:\x\gaia")]);
    let list = rows(&[("aaaa0201", "ui", r"P:\L\gaia\src")]);
    let tree = build(&entries, &list, NOW, false, &TreeInputs::default());
    assert_eq!(folders(&tree), [f("gaia", &["ui"]), f("larchsys/L", &[]), f("larchsys/gaia", &["ui@larchsys/gaia"]), f("gaia-2", &[])], "the later row's keys are scoped; a second top-level gaia is gaia-2");
    assert_eq!((tree.entries[0].name.as_str(), tree.entries[0].group), ("gaia", false), "a top-level folder is its own entry");
    assert_eq!(tree.rows().count(), 2);
}

#[test]
fn a_linked_worktrees_session_shows_under_its_main_checkout() {
    let entries = Entries::new(vec![top(r"P:\L"), top(r"P:\L\AutoWork")]);
    let list = rows(&[("aaaa0301", "unit", r"P:\L\AutoWork.worktrees\unit")]);
    let known = HashMap::from([(PathBuf::from(r"P:\L\AutoWork.worktrees\unit"), Some(PathBuf::from(r"P:\L\AutoWork")))]);
    let tree = build(&entries, &list, NOW, false, &TreeInputs { main_repos: Some(&known), ..TreeInputs::default() });
    assert_eq!(folders(&tree), [f("L", &[]), f("AutoWork", &["unit"])]);
}

#[test]
fn a_shown_child_is_a_row_after_its_parent_and_takes_the_sessions_below_it() {
    let mut larch = FolderEntry::new(r"P:\L");
    larch.show_subfolders = true;
    let entries = Entries::new(vec![group("work", &[r"P:\W"]), Entry::Folder(larch), top(r"P:\L\gaia")]);
    let shown = Shown::from([(path_key(Path::new(r"P:\L")), vec![PathBuf::from(r"P:\L\gaia"), PathBuf::from(r"P:\L\zed")])]);
    let disk = Disk { shown, ..Disk::default() };
    let list = rows(&[("aaaa0401", "ui", r"P:\L\gaia\src"), ("aaaa0402", "top", r"P:\L\x.txt"), ("aaaa0403", "ed", r"P:\L\zed\src")]);
    let tree = build(&entries, &list, NOW, false, &TreeInputs { disk: Some(&disk), ..TreeInputs::default() });
    assert_eq!(folders(&tree), [f("work/W", &[]), f("L", &["top"]), f("L/gaia", &["ui"]), f("L/zed", &["ed"]), f("gaia", &["ui@gaia"])], "gaia/src under the shown gaia, also listed at the top level; zed/src under the shown zed alone");
    let l = &tree.entries[1].folders;
    assert_eq!((l[0].pos, l[0].shows_subfolders, l[0].parent.as_deref()), (Some(Pos::top(1)), true, None));
    assert_eq!((l[1].pos, l[1].parent.as_deref()), (None, Some("L")), "a shown child is no entry of folders.toml");
    assert_eq!(tree.entries[0].folders[0].pos, Some(Pos::member(0, 0)));
    let hidden = Entries::new(vec![top(r"P:\L")]);
    let tree = build(&hidden, &list, NOW, false, &TreeInputs { disk: Some(&disk), ..TreeInputs::default() });
    assert_eq!(folders(&tree), [f("L", &["ui", "top", "ed"])], "a parent that does not show them keeps them");
}

#[test]
fn a_session_in_a_shown_child_holds_the_child_as_its_folder_and_its_parents_entry() {
    let mut infra = FolderEntry::new(r"P:\ops\infra");
    infra.show_subfolders = true;
    let entries = Entries::new(vec![Entry::Group { name: "ops".into(), folders: vec![infra] }, top(r"P:\web")]);
    let shown = Shown::from([(path_key(Path::new(r"P:\ops\infra")), vec![PathBuf::from(r"P:\ops\infra\terraform")])]);
    let disk = Disk { shown: shown.clone(), ..Disk::default() };
    let tree = build(&entries, &[], NOW, false, &TreeInputs { disk: Some(&disk), ..TreeInputs::default() });
    let at = |cwd: &str| tree.holding(&entries, &shown, Path::new(cwd));
    let terraform = Some(("ops".to_string(), PathBuf::from(r"P:\ops\infra\terraform")));
    assert_eq!(at(r"P:\ops\infra\terraform\modules"), terraform, "the child, in the group its parent sits in");
    assert_eq!(at(r"P:\ops\infra\docs"), Some(("ops".to_string(), PathBuf::from(r"P:\ops\infra"))));
    assert_eq!(at(r"P:\web\src"), Some(("web".to_string(), PathBuf::from(r"P:\web"))));
    assert_eq!(at(r"P:\elsewhere"), None);
    assert_eq!(shell_cwd(Some(Path::new(r"P:\ops\infra\terraform\modules")), &entries, &shown, None).as_deref(), Some(Path::new(r"P:\ops\infra\terraform")), "a new shell starts in the child");

    let (entry, folder) = at(r"P:\ops\infra\terraform").unwrap();
    let ops = tree.entry(&entry).unwrap();
    let names = |chosen: Option<&Path>| crate::view::start::folders(ops, chosen).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    assert_eq!(names(Some(&folder)), ["infra", "terraform"], "the Start page offers the preselected child");
    assert_eq!(names(None), ["infra"], "and only the listed folders otherwise");
    let (group, name) = crate::tab_card::place(Path::new(r"P:\ops\infra\terraform\modules"), &entries, &shown);
    assert_eq!((group.as_deref(), name.as_deref()), (Some("ops"), Some("terraform")), "the tab's card names the child");
}

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Output, Stdio};

use supraman_core::{Event, EventReader, role_tag};

const SID: &str = "0a1b2c3d-0000-4000-8000-000000000002";

fn hook(home: Option<&Path>, role: Option<&str>, args: &[&str], stdin: &str) -> Output {
    hook_env(home, role, &[], args, stdin)
}

fn hook_env(home: Option<&Path>, role: Option<&str>, env: &[(&str, &str)], args: &[&str], stdin: &str) -> Output {
    let mut cmd = supraman_core::process::command(env!("CARGO_BIN_EXE_sm"));
    cmd.arg("hook").args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.env_remove("SUPRAMAN_HOME")
        .env_remove("FOREMAN_HOME")
        .env_remove("FOREMAN_ROLE")
        .env_remove("FOREMAN_WORKSPACE")
        .env_remove("FOREMAN_PLATFORM");
    if let Some(home) = home {
        cmd.env("SUPRAMAN_HOME", home);
    }
    if let Some(role) = role {
        cmd.env("FOREMAN_ROLE", role);
    }
    cmd.envs(env.iter().copied());
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn payload(event: &str, extra: &str) -> String {
    format!(
        r#"{{"session_id":"{SID}","hook_event_name":"{event}","cwd":"C:\\w","session_title":"aw-t","transcript_path":"t","prompt":"PRIVATE-PROMPT","last_assistant_message":"PRIVATE-REPLY"{extra}}}"#
    )
}

fn silent_success(out: &Output) {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}

#[test]
fn logs_each_event_and_injects_the_role_on_session_start() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join("roles")).unwrap();
    fs::write(home.path().join("roles").join("worker.md"), "You are the worker.\n").unwrap();
    let h = Some(home.path());

    let start = hook(h, Some("worker"), &["SessionStart"], &payload("SessionStart", r#","source":"compact""#));
    assert_eq!(start.status.code(), Some(0), "{start:?}");
    let tag = role_tag("worker", "You are the worker.");
    assert_eq!(String::from_utf8_lossy(&start.stdout), format!("{tag}\nYou are the worker.\n"));

    silent_success(&hook(h, Some("worker"), &["UserPromptSubmit"], &payload("UserPromptSubmit", "")));
    silent_success(&hook(h, Some("worker"), &["Notification"], &payload("Notification", r#","notification_type":"permission_prompt""#)));
    silent_success(&hook(h, Some("worker"), &["PreCompact"], &payload("PreCompact", r#","trigger":"auto""#)));
    silent_success(&hook(h, Some("worker"), &["Stop"], &payload("Stop", "")));

    let raw = fs::read_to_string(home.path().join("events").join(format!("{SID}.jsonl"))).unwrap();
    assert!(!raw.contains("PRIVATE"), "{raw}");
    let events: Vec<Event> = EventReader::new(home.path()).poll();
    let got: Vec<(&str, Option<&str>)> = events
        .iter()
        .map(|e| (e.event.as_str(), e.source.as_deref().or(e.notification_type.as_deref()).or(e.trigger.as_deref())))
        .collect();
    assert_eq!(
        got,
        [
            ("SessionStart", Some("compact")),
            ("UserPromptSubmit", None),
            ("Notification", Some("permission_prompt")),
            ("PreCompact", Some("auto")),
            ("Stop", None),
        ]
    );
    assert!(events.iter().all(|e| e.session_id == SID && e.name == "aw-t" && e.cwd == r"C:\w" && e.ts > 0));
}

#[test]
fn the_workspace_role_text_names_foreman_workspace_and_nothing_else() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join("roles")).unwrap();
    fs::write(home.path().join("roles").join("workspace.md"), "Controller for {workspace}.\n").unwrap();
    let h = Some(home.path());
    let tag = role_tag("workspace", "Controller for nomos.");
    let expected = format!("{tag}\nController for nomos.\n");
    let start = payload("SessionStart", r#","source":"resume""#);

    let current = hook_env(h, Some("workspace"), &[("FOREMAN_WORKSPACE", "nomos")], &["SessionStart"], &start);
    assert_eq!(String::from_utf8_lossy(&current.stdout), expected, "{current:?}");
    let old = hook_env(h, Some("platform"), &[("FOREMAN_PLATFORM", "nomos")], &["SessionStart"], &start);
    silent_success(&old);
    assert_eq!(String::from_utf8_lossy(&old.stdout), "", "{old:?}");
    let both = [("FOREMAN_WORKSPACE", "nomos"), ("FOREMAN_PLATFORM", "other")];
    let both = hook_env(h, Some("workspace"), &both, &["SessionStart"], &start);
    assert_eq!(String::from_utf8_lossy(&both.stdout), expected, "FOREMAN_PLATFORM is not read");
}

#[test]
fn a_session_of_an_older_build_with_only_foreman_home_still_logs_and_gets_its_role() {
    let (home, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fs::create_dir_all(home.path().join("roles")).unwrap();
    fs::write(home.path().join("roles").join("worker.md"), "You are the worker.
").unwrap();
    let legacy = [("FOREMAN_HOME", home.path().to_str().unwrap())];
    let start = hook_env(None, Some("worker"), &legacy, &["SessionStart"], &payload("SessionStart", ""));
    assert_eq!(String::from_utf8_lossy(&start.stdout), format!("{}
You are the worker.
", role_tag("worker", "You are the worker.")));
    assert_eq!(EventReader::new(home.path()).poll().len(), 1, "logged under FOREMAN_HOME");

    let both = [("FOREMAN_HOME", other.path().to_str().unwrap())];
    silent_success(&hook_env(Some(home.path()), None, &both, &["Stop"], &payload("Stop", "")));
    assert_eq!(EventReader::new(home.path()).poll().len(), 2, "SUPRAMAN_HOME wins");
    assert!(fs::read_dir(other.path()).unwrap().next().is_none(), "nothing under the losing FOREMAN_HOME");
}

#[test]
fn never_fails_and_stays_silent_on_bad_input() {
    let home = tempfile::tempdir().unwrap();
    let h = Some(home.path());
    // no home
    silent_success(&hook(None, Some("worker"), &["SessionStart"], &payload("SessionStart", "")));
    // no role file, no role name
    silent_success(&hook(h, Some("worker"), &["SessionStart"], &payload("SessionStart", "")));
    silent_success(&hook(h, None, &["SessionStart"], &payload("SessionStart", "")));
    // bad stdin, empty stdin, unsafe session id, no event name, extra args
    silent_success(&hook(h, None, &["Stop"], "{not json"));
    silent_success(&hook(h, None, &["Stop"], ""));
    silent_success(&hook(h, None, &["Stop"], r#"{"session_id":"..\\..\\evil"}"#));
    silent_success(&hook(h, None, &[], &payload("Stop", "")));
    silent_success(&hook(h, None, &["Stop", "extra"], &payload("Stop", "")));
    // the home a file: cannot write, still exit 0
    let file = home.path().join("not-a-dir");
    fs::write(&file, "x").unwrap();
    silent_success(&hook(Some(&file), None, &["Stop"], &payload("Stop", "")));
    assert!(!home.path().join("evil.jsonl").exists());
}

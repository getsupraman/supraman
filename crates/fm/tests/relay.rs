use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn fm(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(args)
        .env("FOREMAN_HOME", home)
        .env_remove("APPDATA")
        .env_remove("FOREMAN_ROLE")
        .output()
        .unwrap()
}

#[test]
fn send_is_refused_inside_a_foreman_session_before_touching_the_home() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(["send", "demo", "fm-demo-workspace", "hello"])
        .env("FOREMAN_HOME", home.path())
        .env_remove("APPDATA")
        .env("FOREMAN_ROLE", "workspace")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "fm send speaks as the human through the relay and is refused inside a Foreman session \
         (FOREMAN_ROLE=workspace); use SendMessage to reach a session\n"
    );
    assert!(out.stdout.is_empty(), "{out:?}");
    assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0, "nothing written under FOREMAN_HOME");
}

#[test]
fn send_with_an_empty_or_absent_role_drops_as_before() {
    let home = tempfile::tempdir().unwrap();
    let inbox = home.path().join("relay").join("demo").join("inbox");
    let out = Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(["send", "demo", "fm-demo-workspace", "one"])
        .env("FOREMAN_HOME", home.path())
        .env_remove("APPDATA")
        .env("FOREMAN_ROLE", "")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let out = fm(home.path(), &["send", "demo", "fm-demo-workspace", "two"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("dropped "), "{out:?}");
    let drops = fs::read_dir(&inbox).unwrap().filter(|e| e.as_ref().unwrap().path().is_file()).count();
    assert_eq!(drops, 2);
}

#[test]
fn inbox_prints_each_send_as_one_json_line_within_a_second() {
    let home = tempfile::tempdir().unwrap();
    let inbox = home.path().join("relay").join("demo").join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(["inbox", "--dir", inbox.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send((Instant::now(), line.unwrap())).is_err() {
                break;
            }
        }
    });

    let out = fm(home.path(), &["send", "demo", "fm-demo-workspace", "two\nlines"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let sent = Instant::now();
    let body_file = home.path().join("body.txt");
    fs::write(&body_file, "from a file").unwrap();
    let out = fm(home.path(), &["send", "demo", "fm-demo-x", "--file", body_file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let lines: Vec<(Instant, String)> = (0..2).map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap()).collect();
    let _ = child.kill();
    let _ = child.wait();
    assert!(lines[0].0 - sent < Duration::from_secs(1), "{:?}", lines[0].0 - sent);
    let v: Vec<serde_json::Value> = lines.iter().map(|(_, l)| serde_json::from_str(l).unwrap()).collect();
    assert_eq!((v[0]["to"].as_str(), v[0]["body"].as_str()), (Some("fm-demo-workspace"), Some("two\nlines")));
    assert_eq!((v[1]["to"].as_str(), v[1]["body"].as_str()), (Some("fm-demo-x"), Some("from a file")));
    assert_eq!(fs::read_dir(inbox.join("sent")).unwrap().count(), 2);
}

#[test]
fn messages_prints_the_outbox_newest_last() {
    let home = tempfile::tempdir().unwrap();
    let out = fm(home.path(), &["messages", "demo"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "no messages\n");
    let outbox = home.path().join("relay").join("demo").join("outbox");
    fs::create_dir_all(&outbox).unwrap();
    fs::write(outbox.join("ready.json"), "{}").unwrap();
    fs::write(outbox.join("msg-0001.json"), r#"{"from": "uds:x", "from_name": "fm-demo-workspace", "body": "a\nb"}"#).unwrap();
    let out = fm(home.path(), &["messages", "demo"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("  fm-demo-workspace  msg-0001.json\n    a\n    b\n"), "{text}");
    assert!(!text.contains("ready"), "{text}");
    assert_eq!(fm(home.path(), &["send", "demo", "x"]).status.code(), Some(1));
    assert_eq!(fm(home.path(), &["inbox"]).status.code(), Some(1));
}

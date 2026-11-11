#![cfg(windows)]

use std::process::Stdio;
use std::time::{Duration, Instant};

fn wait_for(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    done()
}

#[test]
#[ignore = "starts Supraman windows and a runas console; run elevated with --ignored"]
fn an_unelevated_second_launch_hands_off_to_an_elevated_foreman() {
    if !supraman_core::platform::process::is_elevated() {
        eprintln!("skipped: needs an elevated test process");
        return;
    }
    let exe = env!("CARGO_BIN_EXE_supraman");
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let mut first = supraman_core::process::command(exe)
        .env("SUPRAMAN_HOME", home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let log = home.path().join("logs").join(supraman_ui::LOG_FILE);
    let read_log = || std::fs::read_to_string(&log).unwrap_or_default();
    // the app logs its tick errors or nothing, so give it time to take the lock
    std::thread::sleep(Duration::from_secs(5));
    assert!(first.try_wait().unwrap().is_none(), "the elevated Supraman exited: {}", read_log());

    let out = scratch.path().join("second.txt");
    let script = scratch.path().join("second.cmd");
    std::fs::write(
        &script,
        format!(
            "@set SUPRAMAN_HOME={}\r\n@\"{exe}\" > \"{out}\" 2>&1\r\n@echo exit %errorlevel% >> \"{out}\"\r\n",
            home.path().display(),
            out = out.display()
        ),
    )
    .unwrap();
    let ran = supraman_core::process::command("runas")
        .arg("/trustlevel:0x20000")
        .arg(format!("cmd /c \"{}\"", script.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let second = || std::fs::read_to_string(&out).unwrap_or_default();
    let exited = ran.success() && wait_for(Duration::from_secs(30), || second().contains("exit "));
    let asked = wait_for(Duration::from_secs(5), || read_log().contains("a second launch asked for the window"));
    let still_first = first.try_wait().unwrap().is_none();
    let _ = first.kill();
    let _ = first.wait();
    assert!(exited, "the unelevated second launch did not exit within 30 s (a second Supraman?): {:?}", second());
    assert!(second().contains("Supraman is already running") && second().contains("exit 0"), "{}", second());
    assert!(asked, "the elevated Supraman was not asked for its window: {}", read_log());
    assert!(still_first);
}

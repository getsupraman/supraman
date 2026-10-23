//! `fm inbox` (the relay's watcher, run under its `Monitor`), `fm send` and `fm messages`. The files
//! and their shape are `foreman_core::relay`'s.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use chrono::DateTime;
use foreman_core::{InboxWatcher, Registry, read_outbox, relay_ready, send};

use crate::Failure;

/// Monitor turns each stdout line into an event; the brief allows at most 500 ms of latency here.
const POLL: Duration = Duration::from_millis(400);

/// Never returns: Monitor ends it (expiry, or the relay stopping). A pass that cannot read the dir
/// prints one `{"error"}` line per distinct error and keeps polling.
pub fn inbox(args: &[&str]) -> Result<(), Failure> {
    let dir = match args {
        ["--dir", dir] => *dir,
        _ => return Err(Failure::Usage("inbox needs --dir <inbox>".into())),
    };
    let mut watcher = InboxWatcher::new(dir);
    let mut last_error = None;
    loop {
        let lines = match watcher.scan() {
            Ok(lines) => {
                last_error = None;
                lines
            }
            Err(e) => {
                let line = serde_json::json!({ "error": format!("cannot read {dir}: {e}") }).to_string();
                if last_error.as_ref() == Some(&line) {
                    Vec::new()
                } else {
                    last_error = Some(line.clone());
                    vec![line]
                }
            }
        };
        if !lines.is_empty() {
            let mut out = std::io::stdout().lock();
            for line in lines {
                // stdout gone means Monitor is gone: nothing left to do.
                if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                    return Ok(());
                }
            }
        }
        std::thread::sleep(POLL);
    }
}

/// The relay delivers a drop as `fm-<w>-relay`, which the workspace-controller skill reads as the human;
/// every Foreman-launched session carries `FOREMAN_ROLE` (`foreman_supervisor` `session_settings`), the
/// human's shell does not. Runs before the registry is opened.
pub fn refuse_inside_foreman() -> Result<(), Failure> {
    match std::env::var_os("FOREMAN_ROLE") {
        Some(role) if !role.is_empty() => Err(Failure::Refused(format!(
            "fm send speaks as the human through the relay and is refused inside a Foreman session (FOREMAN_ROLE={}); use SendMessage to reach a session",
            role.to_string_lossy()
        ))),
        _ => Ok(()),
    }
}

/// `fm send <workspace> <to> <body>` or `fm send <workspace> <to> --file <path>`.
pub fn send_cmd(home: &Path, args: &[&str]) -> Result<(), Failure> {
    let (workspace, to, body) = match args {
        [w, to, "--file", path] => {
            let body =
                std::fs::read_to_string(path).map_err(|e| Failure::Usage(format!("--file {path}: {e}")))?;
            (*w, *to, body)
        }
        [w, to, body] if !body.starts_with("--") => (*w, *to, (*body).to_string()),
        _ => return Err(Failure::Usage("send needs <workspace> <to> <body> or <workspace> <to> --file <path>".into())),
    };
    let path = send(home, workspace, to, &body)?;
    println!("dropped {}", path.display());
    if relay_ready(home, workspace).is_none() {
        eprintln!("notice: the relay of {workspace} has not armed its watcher yet; it sends this once it has");
    }
    Ok(())
}

/// `fm messages <workspace>`: every outbox message, newest last.
pub fn messages(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let [workspace] = args else {
        return Err(Failure::Usage("messages needs <workspace>".into()));
    };
    let msgs = read_outbox(registry.home(), workspace)?;
    if msgs.is_empty() {
        println!("no messages");
    }
    for m in msgs {
        let at = DateTime::from_timestamp_millis(m.at)
            .map_or_else(|| m.at.to_string(), |t| t.format("%Y-%m-%d %H:%M:%SZ").to_string());
        let from = if m.from_name.is_empty() { "?" } else { &m.from_name };
        match &m.error {
            Some(e) => println!("{at}  {}  (unreadable: {e})", m.file),
            None => println!("{at}  {from}  {}", m.file),
        }
        for line in m.body.lines() {
            println!("    {line}");
        }
    }
    Ok(())
}

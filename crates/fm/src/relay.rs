//! `fm inbox` (the relay's watcher, run under its `Monitor`), `fm send`, `fm tell` and `fm messages`. The files
//! and their shape are `foreman_core::relay`'s.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use chrono::DateTime;
use foreman_claude::{Route, Sender};
use foreman_core::{InboxWatcher, Registry, read_outbox, relay_ready, tell};
use foreman_supervisor::{DesiredState, RELAY_ROLE, REPO_ROLE, WORKSPACE_ROLE, relay_session_name, repo_session_name, workspace_session_name};

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

/// `fm send` speaks as the human (direct, or through the relay as `fm-<w>-relay`), which the
/// workspace-controller skill reads as the human's voice; every Foreman-launched session carries `FOREMAN_ROLE` (`foreman_supervisor` `session_settings`), the
/// human's shell does not. Runs before the registry is opened.
pub fn refuse_inside_foreman() -> Result<(), Failure> {
    match std::env::var_os("FOREMAN_ROLE") {
        Some(role) if !role.is_empty() => Err(Failure::Refused(format!(
            "fm send speaks as the human through the relay and is refused inside a Foreman session (FOREMAN_ROLE={}); use SendMessage to reach a session, fm tell to reach the human",
            role.to_string_lossy()
        ))),
        _ => Ok(()),
    }
}

/// How long `fm send` waits for the receiver's burst budget before it goes through the relay.
const MAX_PACING_WAIT: Duration = Duration::from_secs(5);

fn body_arg(args: &[&str]) -> Option<Result<String, Failure>> {
    match args {
        ["--file", path] => Some(std::fs::read_to_string(path).map_err(|e| Failure::Usage(format!("--file {path}: {e}")))),
        [body] if !body.starts_with("--") => Some(Ok((*body).to_string())),
        _ => None,
    }
}

/// `fm send <workspace> <to> <body>` or `fm send <workspace> <to> --file <path>`: into the session's pipe,
/// else through the relay (`foreman_claude::human`); prints which.
pub fn send_cmd(home: &Path, args: &[&str]) -> Result<(), Failure> {
    let usage = || Failure::Usage("send needs <workspace> <to> <body> or <workspace> <to> --file <path>".into());
    let [workspace, to, rest @ ..] = args else { return Err(usage()) };
    let body = body_arg(rest).ok_or_else(usage)??;
    let sender = Sender { max_wait: MAX_PACING_WAIT, ..Sender::default() };
    let mode = DesiredState::launch_mode(home, workspace, to);
    let sent = sender.send(home, workspace, to, mode.as_deref(), &body).map_err(|e| Failure::Refused(format!("error: {e}")))?;
    match &sent.route {
        Route::Direct => println!("{} to {to} (written into its pipe; no receipt comes back)", sent.label()),
        Route::Relay { reason, drop } => {
            println!("dropped {} (via relay: {reason})", drop.display());
            if relay_ready(home, workspace).is_none() {
                eprintln!("notice: the relay of {workspace} has not armed its watcher yet; it sends this once it has");
            }
        }
    }
    Ok(())
}

/// Who runs `fm tell`: `FOREMAN_SESSION`, else the name Foreman gives the session's role (a session
/// launched before m5/4 has no `FOREMAN_SESSION`).
fn session_name(workspace: &str) -> Option<String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let Some(name) = var("FOREMAN_SESSION") {
        return Some(name);
    }
    match var("FOREMAN_ROLE")?.as_str() {
        WORKSPACE_ROLE => Some(workspace_session_name(workspace)),
        RELAY_ROLE => Some(relay_session_name(workspace)),
        REPO_ROLE => repo_session_name(workspace, &var("FOREMAN_REPO")?),
        _ => None,
    }
}

/// `fm tell <body>` or `fm tell --file <path>`, run by a Foreman session to reach the human: an outbox
/// message from it in its workspace (`FOREMAN_HOME`, `FOREMAN_WORKSPACE` from the session's env).
pub fn tell_cmd(args: &[&str]) -> Result<(), Failure> {
    let body = body_arg(args).ok_or_else(|| Failure::Usage("tell needs <body> or --file <path>".into()))??;
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (Some(home), Some(workspace)) = (var("FOREMAN_HOME"), var("FOREMAN_WORKSPACE")) else {
        return Err(Failure::Refused(
            "fm tell runs inside a Foreman session (FOREMAN_HOME and FOREMAN_WORKSPACE are not set); the human reads Foreman's Messages".into(),
        ));
    };
    let from = session_name(&workspace)
        .ok_or_else(|| Failure::Refused("fm tell cannot tell which session this is: FOREMAN_SESSION is not set".into()))?;
    let path = tell(Path::new(&home), &workspace, &from, &body)?;
    println!("told the human ({workspace}): {}", path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default());
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

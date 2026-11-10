//! `sm inbox` (the relay's watcher, run under its `Monitor`), `sm send`, `sm tell` and `sm messages`. The files
//! and their shape are `supraman_core::relay`'s.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use chrono::DateTime;
use supraman_claude::{Route, Sender};
use supraman_core::home::SessionEnv;
use supraman_core::{InboxWatcher, Registry, read_outbox, relay_ready, tell};
use supraman_supervisor::DesiredState;

use crate::Failure;

/// Monitor turns each stdout line into an event; at most 500 ms of latency.
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

/// `sm send` speaks as the human; every Supraman-launched session carries `SUPRAMAN_ROLE` (`supraman_supervisor::LaunchConfig::launch_spec`).
pub fn refuse_inside_supraman() -> Result<(), Failure> {
    match SessionEnv::role_os() {
        Some(role) if !role.is_empty() => Err(Failure::Refused(format!(
            "sm send speaks as the human through the relay and is refused inside a Supraman session (SUPRAMAN_ROLE={}); use SendMessage to reach a session, sm tell to reach the human",
            role.to_string_lossy()
        ))),
        _ => Ok(()),
    }
}

/// How long `sm send` waits for the receiver's burst budget before it goes through the relay.
const MAX_PACING_WAIT: Duration = Duration::from_secs(5);

fn body_arg(args: &[&str]) -> Option<Result<String, Failure>> {
    match args {
        ["--file", path] => Some(std::fs::read_to_string(path).map_err(|e| Failure::Usage(format!("--file {path}: {e}")))),
        [body] if !body.starts_with("--") => Some(Ok((*body).to_string())),
        _ => None,
    }
}

/// Into the session's pipe, else through the relay (`supraman_claude::human`); prints which.
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
                eprintln!(
                    "notice: the relay of {workspace} is not ready; Supraman starts it for this message (while the workspace is started), and it sends it once armed"
                );
            }
        }
    }
    Ok(())
}

/// Run by a Supraman session to reach the human: an outbox message in its workspace (its home, `SUPRAMAN_WORKSPACE`).
pub fn tell_cmd(args: &[&str]) -> Result<(), Failure> {
    let body = body_arg(args).ok_or_else(|| Failure::Usage("tell needs <body> or --file <path>".into()))??;
    let env = SessionEnv::from_env();
    let set = |v: Option<String>| v.filter(|v| !v.is_empty());
    let (Some(home), Some(workspace)) = (supraman_core::env_home(), set(env.workspace)) else {
        return Err(Failure::Refused(
            "sm tell runs inside a Supraman session (SUPRAMAN_HOME and SUPRAMAN_WORKSPACE are not set); the human reads Supraman's Messages".into(),
        ));
    };
    let from = set(env.session)
        .ok_or_else(|| Failure::Refused("sm tell cannot tell which session this is: SUPRAMAN_SESSION is not set".into()))?;
    let path = tell(&home, &workspace, &from, &body)?;
    println!("told the human ({workspace}): {}", path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default());
    Ok(())
}

/// Newest last.
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

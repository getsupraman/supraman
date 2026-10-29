//! `fm work overview|board`: the workspace's WorkState project (ref `<workspace>`) read through the
//! read API v1, printed compact. Read only (C21); the bearer is an access token of the app's sign-in
//! (refreshed and rotated here under the shared lock) and never printed — every error is one line without it.

use std::fmt::Write as _;
use std::path::Path;

use foreman_workstate::{Auth, AuthStatus, Board, BoardRow, Client, ItemRef, Overview, Rollup};

use crate::Failure;

pub fn work(home: &Path, args: &[&str]) -> Result<(), Failure> {
    match args {
        ["overview", workspace] => {
            let client = client(home)?;
            let project = project(&client, workspace)?;
            let overview = client.overview().map_err(refused)?;
            print!("{}", render_overview(&overview, &project)?);
            Ok(())
        }
        ["board", workspace, run] => {
            let client = client(home)?;
            let project = project(&client, workspace)?;
            let board = board(&client, &project, workspace, run)?;
            print!("{}", render_board(&board));
            Ok(())
        }
        _ => Err(Failure::Usage(format!("unrecognised arguments: work {}", args.join(" ")))),
    }
}

fn refused(e: impl std::fmt::Display) -> Failure {
    Failure::Refused(format!("error: {e}"))
}

fn client(home: &Path) -> Result<Client, Failure> {
    let auth = Auth::open(home);
    let origin = auth.configured_origin();
    auth.access_token(&origin).map_err(refused)?;
    if let AuthStatus::SignedIn { unsaved: Some(why), .. } = auth.status(&origin) {
        eprintln!("warning: {why}");
    }
    Ok(Client::with_source(origin.as_str(), auth.bearer_for(&origin)))
}

/// The one project whose ref is the workspace's name.
fn project(client: &Client, workspace: &str) -> Result<ItemRef, Failure> {
    let mut found: Vec<ItemRef> = client.resolve(workspace).map_err(refused)?.matches.into_iter().filter(|m| m.kind == "project").collect();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(refused(format!("WorkState has no project '{workspace}'"))),
        n => Err(refused(format!("'{workspace}' names {n} WorkState projects"))),
    }
}

/// The run `run` of `project`: a ref can name runs of other projects too, so each candidate's board
/// says whose it is.
fn board(client: &Client, project: &ItemRef, workspace: &str, run: &str) -> Result<Board, Failure> {
    let runs = client.resolve(run).map_err(refused)?.matches.into_iter().filter(|m| m.kind == "run");
    for candidate in runs {
        let board = client.board(&candidate.id).map_err(refused)?;
        if board.project.as_ref().is_some_and(|p| p.id == project.id) {
            return Ok(board);
        }
    }
    Err(refused(format!("WorkState project '{workspace}' has no run '{run}'")))
}

/// `10/13 done, 1 in progress, 1 blocked` — the denominator leaves out cancelled and superseded.
fn rollup_text(r: &Rollup) -> String {
    let mut out = format!("{}/{} done", r.done, r.counted());
    for (n, what) in [(r.in_progress, "in progress"), (r.review, "review"), (r.blocked, "blocked"), (r.open, "open")] {
        if n > 0 {
            let _ = write!(out, ", {n} {what}");
        }
    }
    out
}

fn render_overview(o: &Overview, project: &ItemRef) -> Result<String, Failure> {
    let p = o
        .projects
        .iter()
        .find(|p| p.item.id == project.id)
        .ok_or_else(|| refused(format!("WorkState's overview has no project '{}'", project.ref_)))?;
    let mut out = String::new();
    let _ = writeln!(out, "{}  {}  {}  (seq {}, as of {})", p.item.ref_, p.item.title, rollup_text(&p.rollup), o.seq, o.as_of);
    for r in &p.runs {
        let _ = write!(out, "  {}  {}  {}  [{}]", r.item.qualified_ref, r.item.title, rollup_text(&r.rollup), r.item.status.as_str());
        if let Some(b) = &r.branch {
            let _ = write!(out, "  branch {b}");
        }
        if r.active_claims > 0 {
            let _ = write!(out, "  claims {}", r.active_claims);
        }
        if r.inbox_count > 0 {
            let _ = write!(out, "  inbox {}", r.inbox_count);
        }
        if let Some(v) = &r.last_verification {
            let _ = write!(out, "  last verify {}", if v.passed { "pass" } else { "FAIL" });
        }
        out.push('\n');
    }
    let ours: Vec<_> = o.working_now.iter().filter(|c| p.runs.iter().any(|r| c.run.as_ref().is_some_and(|run| run.id == r.item.id))).collect();
    if !ours.is_empty() {
        out.push_str("working now\n");
        for c in ours {
            let expired = if c.expired { "  EXPIRED" } else { "" };
            let _ = writeln!(out, "  {}  {} ({})  until {}{expired}", c.item.qualified_ref, c.holder(), c.role, c.expires_at);
        }
    }
    let h = &o.honesty;
    let _ = writeln!(
        out,
        "inbox {}  done-unverified {}  forced {}  ungated {}  follow-ups open {}",
        o.inbox_count, h.done_unverified, h.forced, h.ungated, h.followups_open
    );
    Ok(out)
}

fn render_board(b: &Board) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}  {}  ({})  {}  (seq {})", b.run.qualified_ref, b.run.title, b.project.as_ref().map_or("—", |p| p.ref_.as_str()), rollup_text(&b.rollup), b.seq);
    if let Some(branch) = &b.branch {
        let _ = writeln!(out, "branch {branch}");
    }
    if b.rows.is_empty() {
        out.push_str("  no items\n");
    }
    for row in &b.rows {
        out.push_str(&board_row(row));
    }
    out
}

fn board_row(r: &BoardRow) -> String {
    let indent = "  ".repeat(r.depth.max(1) as usize);
    let seq = format!("{indent}{}", r.seq);
    let criteria = if r.criteria.total == 0 { "-".to_string() } else { format!("{}/{}", r.criteria.checked, r.criteria.total) };
    let claim = match &r.claim {
        Some(c) if c.expired => format!("{} (expired)", c.holder()),
        Some(c) => c.holder().to_string(),
        None => "-".into(),
    };
    let verified = match &r.verification {
        Some(v) if v.passed => "pass",
        Some(_) => "FAIL",
        None => "-",
    };
    let mut line = format!("{seq:<10} {:<11} {criteria:<5} {claim:<16} {verified:<4}  {}", r.item.status.as_str(), r.item.title);
    if let Some(rollup) = &r.rollup {
        let _ = write!(line, "  ({})", rollup_text(rollup));
    }
    if !r.blocked_by.is_empty() {
        let by: Vec<&str> = r.blocked_by.iter().map(|i| i.qualified_ref.as_str()).collect();
        let _ = write!(line, "  blocked by {}", by.join(", "));
    }
    if !r.flags.is_empty() {
        let _ = write!(line, "  [{}]", r.flags.join(" "));
    }
    line.push('\n');
    line
}

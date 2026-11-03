use std::path::{Path, PathBuf};

use supraman_claude::{Health, Kind, Status};
use supraman_ui::details::Section;

use crate::fleet::{self, Open, SessionRow};
use supraman_core::time::{ago, date_words};

/// Who launched a session, as far as this window can tell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartedBy {
    Supervisor,
    /// A run of the schedule with this id.
    Schedule(String),
    Hand,
    Outside,
}

/// What a session's page reads from disk; refreshed with the poll, never while painting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnDisk {
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub transcript: Option<PathBuf>,
    pub transcript_ms: Option<i64>,
}

impl OnDisk {
    /// `known`: the transcript found before, looked up again only once it is gone.
    pub fn read(cwd: &Path, session_id: &str, claude_dir: Option<&Path>, known: Option<&Path>) -> Self {
        let transcript = known
            .filter(|p| p.is_file())
            .map(Path::to_path_buf)
            .or_else(|| claude_dir.and_then(|d| supraman_claude::find_transcript(d, session_id, Some(cwd))));
        let transcript_ms = transcript.as_deref().and_then(|p| std::fs::metadata(p).ok()).map(|m| supraman_core::time::modified_ms(&m));
        Self { branch: supraman_core::branch_at(cwd), worktree: worktree(cwd), transcript, transcript_ms }
    }
}

pub struct SessionFacts<'a> {
    pub row: &'a SessionRow,
    pub workspace: &'a str,
    /// The workspace's folder holding its cwd.
    pub folder: Option<(&'a str, &'a Path)>,
    pub started_by: StartedBy,
    /// `None` while the workspace's Work tab is not open, which is where claims are read.
    pub claimed: Option<Option<String>>,
    /// The Workman sign-in of the [+] tab running it.
    pub sign_in: Option<String>,
    pub open: Open,
    /// Why its menu has no Adopt ([`fleet::Adoption::No`]).
    pub adopt: Option<String>,
    pub disk: &'a OnDisk,
    /// In the last poll's tree; else the page shows the session as it was last seen.
    pub listed: bool,
}

pub fn session_title(row: &SessionRow) -> String {
    if row.session.name.is_empty() { format!("Session {}", fleet::short_id(&row.session)) } else { row.session.name.clone() }
}

/// The pill's words, and whether the session asks for someone.
pub fn session_pill(row: &SessionRow, listed: bool) -> (&'static str, bool) {
    match &row.health {
        _ if !listed => ("Not listed", false),
        _ if row.closed => ("Closed", false),
        Health::Blocked { .. } => ("Blocked", true),
        Health::Orphaned { .. } => ("Interrupted", true),
        Health::Running => ("Running", false),
        Health::Idle => ("Idle", false),
        Health::Stopped => ("Stopped", false),
        Health::Unknown => ("Unknown", false),
    }
}

fn state_words(row: &SessionRow) -> String {
    if row.closed {
        return "Closed: its process ended; the transcript stays and it can be resumed".to_string();
    }
    match &row.health {
        Health::Running => "Running: working on a turn".to_string(),
        Health::Blocked { waiting_for: Some(w) } => format!("Blocked: waits for {w}"),
        Health::Blocked { waiting_for: None } => "Blocked: waits for an answer".to_string(),
        Health::Idle => "Idle: waits for a prompt".to_string(),
        Health::Stopped => "Stopped: no process runs it".to_string(),
        Health::Orphaned { last_state } => format!("Interrupted: the process ended while the session was {}", last_state.as_str()),
        Health::Unknown => "Unknown".to_string(),
    }
}

fn reported(row: &SessionRow) -> Option<String> {
    let s = &row.session;
    let status = s.status.as_ref().map(|st| match st {
        Status::Busy => "busy",
        Status::Waiting => "waiting",
        Status::Idle => "idle",
        Status::Other(o) => o.as_str(),
    });
    let parts: Vec<String> = [s.state.as_ref().map(|st| format!("state {}", st.as_str())), status.map(|st| format!("status {st}"))].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn when(ms: i64, now_ms: i64) -> String {
    format!("{} ({})", ago(now_ms - ms), date_words(ms))
}

pub fn session(f: &SessionFacts, now_ms: i64) -> Vec<Section> {
    let (row, s) = (f.row, &f.row.session);
    let mut session = Section::new("session", "Session");
    if !f.listed {
        session = session.text("The last poll no longer lists it; this is the session as it was last seen.");
    }
    session = session.row("Name", if s.name.is_empty() { "(unnamed)".to_string() } else { s.name.clone() }).row(
        "Kind",
        match &s.kind {
            Kind::Background => "Background: claude's daemon runs it".to_string(),
            Kind::Interactive => "Interactive: it runs in a terminal".to_string(),
            Kind::Other(o) => o.clone(),
        },
    );
    session = session.row("State", state_words(row));
    if let Some(r) = reported(row) {
        session = session.row("claude reports", r);
    }
    if !row.closed
        && let Some(at) = s.started_at
    {
        session = session.row("Started", when(at, now_ms));
    }
    match (&row.hook, f.disk.transcript_ms) {
        (Some(h), _) => session = session.row("Last activity", h.label(now_ms)),
        (None, Some(ms)) => session = session.row("Last activity", format!("transcript written {}", when(ms, now_ms))),
        (None, None) => {}
    }
    session = match &f.started_by {
        StartedBy::Supervisor => session.row("Started by", "Supraman's supervisor"),
        StartedBy::Schedule(id) => session.link("Started by", format!("a run of schedule {id}"), id.clone()),
        StartedBy::Hand => session.row("Started by", "a [+] tab in this window"),
        StartedBy::Outside => session.row("Started by", "outside Supraman"),
    };
    if let Some(h) = f.open.hint() {
        session = session.row("Open", h);
    }
    let id = if s.id.is_empty() { &s.session_id } else { &s.id };
    session = session.row("Id", id.clone());
    if !s.session_id.is_empty() && s.session_id != *id {
        session = session.row("UUID", s.session_id.clone());
    }
    if let Some(w) = &row.also_in {
        session = session.row("Also in", w.clone());
    }
    if let Some(why) = &f.adopt {
        session = session.row("Adopt", why.clone());
    }

    let mut place = Section::new("where", "Where").row("Workspace", f.workspace.to_string());
    place = match f.folder {
        Some((name, path)) => place.row("Folder", format!("{name} ({})", path.display())),
        None => place.row("Folder", format!("none of {}'s folders", f.workspace)),
    };
    place = place.row("Working directory", s.cwd.display().to_string());
    if let Some(b) = &f.disk.branch {
        place = place.row("Branch", b.clone());
    }
    if let Some(w) = &f.disk.worktree {
        place = place.row("Worktree", w.clone());
    }

    let mut workman = Section::new("workman", "Workman");
    workman = match &f.claimed {
        Some(Some(item)) => workman.row("Claimed item", item.clone()).row("Its session", s.name.clone()),
        Some(None) => workman.text("Claims no item on the open Work tab's board."),
        None => workman.text("Open the workspace's Work tab to see what it claims."),
    };
    if let Some(sign_in) = &f.sign_in {
        workman = workman.row("Sign-in", sign_in.clone());
    }

    let transcript = match &f.disk.transcript {
        Some(p) => Section::new("transcript", "Transcript").long(p.display().to_string()),
        None => Section::new("transcript", "Transcript").text("No transcript found under claude's projects folder."),
    };
    vec![session, place, workman, transcript]
}

#[derive(Clone, Debug, Default)]
pub struct WorkspaceFacts {
    pub name: String,
    pub home: Option<String>,
    /// Name, path, enabled.
    pub folders: Vec<(String, PathBuf, bool)>,
    /// Path, depth.
    pub scans: Vec<(PathBuf, usize)>,
    pub started: Option<bool>,
    pub sessions: usize,
    pub own: Option<SessionRow>,
    /// [`crate::supervise::RelayStatus::label`].
    pub relay: Option<String>,
    pub project: Option<String>,
}

pub fn workspace_pill(started: Option<bool>) -> &'static str {
    match started {
        Some(true) => "Started",
        Some(false) => "Stopped",
        None => "Never started",
    }
}

pub fn workspace(facts: &WorkspaceFacts, now_ms: i64) -> Vec<Section> {
    let mut ws = Section::new("workspace", "Workspace").row("Name", facts.name.clone()).row(
        "State",
        match facts.started {
            Some(true) => "Started: the supervisor keeps its wanted sessions running",
            Some(false) => "Stopped: the supervisor runs none of its sessions",
            None => "Never started",
        },
    );
    if let Some(h) = facts.home.as_ref().filter(|h| !h.is_empty()) {
        ws = ws.row("Home", h.clone());
    }
    ws = ws.row("Sessions", match facts.sessions {
        1 => "1 in the tree".to_string(),
        n => format!("{n} in the tree"),
    });
    ws = ws.row("Relay", facts.relay.clone().unwrap_or_else(|| "stopped".to_string()));
    if let Some(p) = &facts.project {
        ws = ws.row("Workman project", p.clone());
    }

    let mut folders = Section::new("folders", "Folders");
    if facts.folders.is_empty() {
        folders = folders.text("None yet: Edit folders… adds one.");
    }
    for (name, path, enabled) in &facts.folders {
        let disabled = if *enabled { "" } else { " (disabled)" };
        folders = folders.row(name.clone(), format!("{}{disabled}", path.display()));
    }

    let mut scans = Section::new("scans", "Scans");
    if facts.scans.is_empty() {
        scans = scans.text("None: Rescan repos adds nothing by itself.");
    }
    for (path, depth) in &facts.scans {
        scans = scans.row("Scan", format!("{}, git repos up to {depth} level(s) down", path.display()));
    }

    let mut own = Section::new("own", "Workspace session");
    match &facts.own {
        Some(row) => {
            own = own.row("Name", row.session.name.clone()).row("State", state_words(row)).row("Id", fleet::short_id(&row.session).to_string());
            if !row.session.session_id.is_empty() {
                own = own.row("UUID", row.session.session_id.clone());
            }
            if let Some(h) = &row.hook {
                own = own.row("Last activity", h.label(now_ms));
            }
        }
        None => own = own.text("Not wanted."),
    }
    vec![ws, folders, scans, own]
}

/// `<worktree folder> of <main checkout>` when `cwd` lies in a linked worktree.
fn worktree(cwd: &Path) -> Option<String> {
    let git = supraman_core::paths::git_at(cwd)?;
    if !git.is_file() {
        return None;
    }
    let top = git.parent()?;
    let main = supraman_core::paths::main_checkout(cwd)?;
    let name = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| top.display().to_string());
    Some(format!("{name} of {}", main.display()))
}

#[cfg(test)]
mod tests {
    use supraman_claude::{Health, parse};
    use supraman_ui::details::text;

    use super::*;
    use crate::fleet::{HookSignal, SessionRow};

    const NOW: i64 = 1_800_000_000_000;

    fn row(health: Health) -> SessionRow {
        let json = r#"[{"id":"77aa88bb","sessionId":"77aa88bb-x","name":"fm-demo-workspace","cwd":"C:\\nowhere\\demo","kind":"background","startedAt":1,"state":"working","status":"busy","pid":1}]"#;
        SessionRow::new(parse(json).unwrap().remove(0), health)
    }

    fn facts<'a>(row: &'a SessionRow, disk: &'a OnDisk) -> SessionFacts<'a> {
        SessionFacts {
            row,
            workspace: "demo",
            folder: Some(("Alpha", Path::new(r"C:\nowhere\demo"))),
            started_by: StartedBy::Supervisor,
            claimed: Some(Some("m5/59".into())),
            sign_in: None,
            open: Open::Attach,
            adopt: None,
            disk,
            listed: true,
        }
    }

    #[test]
    fn a_sessions_page_reads_its_facts_in_words_under_session_where_workman_and_transcript() {
        let mut r = row(Health::Running);
        r.session.started_at = Some(NOW - 120_000);
        r.also_in = Some("larchsys".into());
        let disk = OnDisk { branch: Some("main".into()), transcript: Some(PathBuf::from(r"C:\claude\projects\x\77aa88bb-x.jsonl")), transcript_ms: Some(NOW - 5_000), ..Default::default() };
        let page = text(&session_title(&r), &session(&facts(&r, &disk), NOW));
        let started = format!("Started  2 min ago ({})", date_words(NOW - 120_000));
        let written = format!("Last activity  transcript written 5 s ago ({})", date_words(NOW - 5_000));
        assert_eq!(
            page,
            [
                "fm-demo-workspace",
                "",
                "Session",
                "Name  fm-demo-workspace",
                "Kind  Background: claude's daemon runs it",
                "State  Running: working on a turn",
                "claude reports  state working, status busy",
                &started,
                &written,
                "Started by  Supraman's supervisor",
                "Open  Attach in a new tab",
                "Id  77aa88bb",
                "UUID  77aa88bb-x",
                "Also in  larchsys",
                "",
                "Where",
                "Workspace  demo",
                r"Folder  Alpha (C:\nowhere\demo)",
                r"Working directory  C:\nowhere\demo",
                "Branch  main",
                "",
                "Workman",
                "Claimed item  m5/59",
                "Its session  fm-demo-workspace",
                "",
                "Transcript",
                r"C:\claude\projects\x\77aa88bb-x.jsonl",
            ]
            .join("\n")
        );
    }

    #[test]
    fn a_hook_is_its_last_activity_and_a_schedules_run_links_its_schedule() {
        let mut r = row(Health::Blocked { waiting_for: Some("permission prompt".into()) });
        r.hook = Some(HookSignal { last: supraman_core::Event::from_payload("Stop", r#"{"session_id":"77aa88bb-x"}"#, NOW - 12_000).unwrap(), compacted_at: None });
        let disk = OnDisk::default();
        let mut f = facts(&r, &disk);
        f.started_by = StartedBy::Schedule("nightly".into());
        f.claimed = None;
        let sections = session(&f, NOW);
        let page = text("x", &sections);
        assert!(page.contains("\nState  Blocked: waits for permission prompt\n"), "{page}");
        assert!(page.contains("\nLast activity  turn ended 12 s ago\n"), "{page}");
        assert!(page.contains("\nOpen the workspace's Work tab to see what it claims."), "{page}");
        assert!(page.ends_with("\nNo transcript found under claude's projects folder."), "{page}");
        assert!(sections[0].lines.contains(&supraman_ui::details::Line::Link { label: "Started by".into(), value: "a run of schedule nightly".into(), target: "nightly".into() }));
        assert_eq!(session_pill(&r, true), ("Blocked", true));
    }

    #[test]
    fn a_closed_or_unlisted_session_says_so_and_an_interrupted_one_says_its_process_ended() {
        let disk = OnDisk::default();
        let mut r = row(Health::Stopped);
        r.closed = true;
        r.session.name.clear();
        let mut f = facts(&r, &disk);
        f.listed = false;
        f.started_by = StartedBy::Outside;
        let page = text(&session_title(&r), &session(&f, NOW));
        assert!(page.starts_with("Session 77aa88bb\n\nSession\nThe last poll no longer lists it"), "{page}");
        assert!(page.contains("\nName  (unnamed)\n") && page.contains("\nState  Closed: its process ended") && !page.contains("\nStarted  "), "{page}");
        assert!(page.contains("\nStarted by  outside Supraman\n"), "{page}");
        assert_eq!(session_pill(&r, false), ("Not listed", false));
        let gone = row(Health::Orphaned { last_state: supraman_claude::SessionState::Working });
        assert!(text("x", &session(&facts(&gone, &disk), NOW)).contains("\nState  Interrupted: the process ended while the session was working\n"));
    }

    #[test]
    fn a_workspaces_page_reads_its_home_folders_scans_and_its_session() {
        let facts = WorkspaceFacts {
            name: "demo".into(),
            home: Some(r"C:\nowhere\demo".into()),
            folders: vec![("Alpha".into(), PathBuf::from(r"C:\nowhere\demo\Alpha"), true), ("Beta".into(), PathBuf::from(r"C:\nowhere\demo\Beta"), false)],
            scans: vec![(PathBuf::from(r"C:\nowhere\demo"), 2)],
            started: Some(true),
            sessions: 3,
            own: Some(row(Health::Idle)),
            relay: Some("relay ready".into()),
            project: Some("autowork".into()),
        };
        assert_eq!(
            text("demo", &workspace(&facts, NOW)),
            [
                "demo",
                "",
                "Workspace",
                "Name  demo",
                "State  Started: the supervisor keeps its wanted sessions running",
                r"Home  C:\nowhere\demo",
                "Sessions  3 in the tree",
                "Relay  relay ready",
                "Workman project  autowork",
                "",
                "Folders",
                r"Alpha  C:\nowhere\demo\Alpha",
                r"Beta  C:\nowhere\demo\Beta (disabled)",
                "",
                "Scans",
                r"Scan  C:\nowhere\demo, git repos up to 2 level(s) down",
                "",
                "Workspace session",
                "Name  fm-demo-workspace",
                "State  Idle: waits for a prompt",
                "Id  77aa88bb",
                "UUID  77aa88bb-x",
            ]
            .join("\n")
        );
        let bare = WorkspaceFacts { name: "gaia".into(), ..Default::default() };
        let page = text("gaia", &workspace(&bare, NOW));
        assert!(page.contains("\nState  Never started\n") && page.contains("\nNone yet: Edit folders… adds one.\n") && page.ends_with("\nWorkspace session\nNot wanted."), "{page}");
        assert_eq!(workspace_pill(None), "Never started");
    }
}

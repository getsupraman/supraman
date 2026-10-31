use std::path::Path;

use foreman_ui::hover_card::HoverCard;

use crate::fleet::{self, Open, SessionRow};

#[derive(Clone, Debug, Default)]
pub struct WorkspaceFacts {
    pub name: String,
    pub home: Option<String>,
    pub folders: Vec<String>,
    pub started: Option<bool>,
    pub own: Option<SessionRow>,
    /// [`crate::supervise::RelayStatus::label`].
    pub relay: Option<String>,
    pub project: Option<String>,
}

pub fn workspace(facts: &WorkspaceFacts, now_ms: i64) -> HoverCard {
    let mut rows = Vec::new();
    rows.extend(facts.home.clone().filter(|h| !h.is_empty()).map(|h| ("home", h)));
    rows.push(("folders", if facts.folders.is_empty() { "none".to_string() } else { facts.folders.join(", ") }));
    let state = match facts.started {
        Some(true) => "started",
        Some(false) => "stopped",
        None => "never started",
    };
    rows.push(("state", state.to_string()));
    match &facts.own {
        Some(own) => {
            rows.push(("session", format!("{} · {} · {}", own.session.name, fleet::health_label(&own.health), fleet::short_id(&own.session))));
            rows.extend(Some(own.session.session_id.clone()).filter(|u| !u.is_empty()).map(|u| ("uuid", u)));
            rows.extend(waiting_for(own).map(|w| ("waiting", w)));
            rows.extend(own.hook.as_ref().map(|h| ("hook", h.label(now_ms))));
        }
        None => rows.push(("session", "not wanted".to_string())),
    }
    rows.push(("relay", facts.relay.clone().unwrap_or_else(|| "stopped".to_string())));
    rows.extend(facts.project.clone().map(|p| ("Workman", p)));
    HoverCard { title: facts.name.clone(), rows }
}

/// `adopt`: why its menu has no Adopt ([`fleet::Adoption::No`]).
pub fn session(row: &SessionRow, claimed: Option<String>, open: Open, adopt: Option<String>, now_ms: i64) -> HoverCard {
    let s = &row.session;
    let title = if s.name.is_empty() { "(unnamed)".to_string() } else { s.name.clone() };
    let state = if row.closed { "closed".to_string() } else { fleet::health_label(&row.health) };
    let mut rows = vec![("state", state)];
    if let foreman_claude::Health::Orphaned { last_state } = &row.health {
        rows.push(("", format!("the process ended while the session was {}", last_state.as_str())));
    }
    rows.extend(waiting_for(row).map(|w| ("waiting", w)));
    rows.extend(open.hint().map(|h| ("open", h.to_string())));
    let id = if s.id.is_empty() { &s.session_id } else { &s.id };
    rows.push(("id", format!("{} · {id}", s.kind.as_str())));
    if !s.session_id.is_empty() && s.session_id != *id {
        rows.push(("uuid", s.session_id.clone()));
    }
    rows.push(("cwd", s.cwd.display().to_string()));
    rows.extend(worktree(&s.cwd).map(|w| ("worktree", w)));
    rows.extend(foreman_core::branch_at(&s.cwd).map(|b| ("branch", b)));
    rows.extend(row.hook.as_ref().map(|h| ("hook", h.label(now_ms))));
    rows.extend(claimed.map(|c| ("claimed", c)));
    rows.extend(row.also_in.clone().map(|w| ("also in", w)));
    rows.extend(adopt.map(|why| ("adopt", why)));
    HoverCard { title, rows }
}

fn waiting_for(row: &SessionRow) -> Option<String> {
    match &row.health {
        foreman_claude::Health::Blocked { waiting_for: Some(w) } => Some(w.clone()),
        _ => None,
    }
}

/// `<worktree folder> of <main checkout>` when `cwd` lies in a linked worktree.
fn worktree(cwd: &Path) -> Option<String> {
    let top = cwd.ancestors().find(|d| d.join(".git").exists())?;
    if !top.join(".git").is_file() {
        return None;
    }
    let main = foreman_core::paths::main_checkout(cwd)?;
    let name = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| top.display().to_string());
    Some(format!("{name} of {}", main.display()))
}

#[cfg(test)]
mod tests {
    use foreman_claude::{Health, parse};

    use super::*;
    use crate::fleet::{HookSignal, SessionRow};

    const NOW: i64 = 1_800_000_000_000;

    fn row(health: Health) -> SessionRow {
        let json = r#"[{"id":"77aa88bb","sessionId":"77aa88bb-x","name":"fm-demo-workspace","cwd":"C:\\nowhere\\demo","kind":"background","startedAt":1,"state":"working","status":"busy","pid":1}]"#;
        SessionRow::new(parse(json).unwrap().remove(0), health)
    }

    #[test]
    fn the_workspace_details_name_home_folders_state_its_session_relay_and_project() {
        let mut own = row(Health::Blocked { waiting_for: Some("permission prompt".into()) });
        own.hook = Some(HookSignal {
            last: foreman_core::Event::from_payload("Stop", r#"{"session_id":"77aa88bb-x"}"#, NOW - 12_000).unwrap(),
            compacted_at: None,
        });
        let facts = WorkspaceFacts {
            name: "demo".into(),
            home: Some(r"C:\nowhere\demo".into()),
            folders: vec!["Alpha".into(), "Beta".into()],
            started: Some(true),
            own: Some(own),
            relay: Some("relay ready".into()),
            project: Some("autowork".into()),
        };
        assert_eq!(
            workspace(&facts, NOW).text(),
            "demo\nhome  C:\\nowhere\\demo\nfolders  Alpha, Beta\nstate  started\nsession  fm-demo-workspace · blocked · 77aa88bb\nuuid  77aa88bb-x\n\
             waiting  permission prompt\nhook  turn ended 12 s ago\nrelay  relay ready\nWorkman  autowork"
        );
        let bare = WorkspaceFacts { name: "gaia".into(), ..Default::default() };
        assert_eq!(workspace(&bare, NOW).text(), "gaia\nfolders  none\nstate  never started\nsession  not wanted\nrelay  stopped");
    }

    #[test]
    fn the_session_details_name_state_id_uuid_cwd_hook_claim_and_the_owner() {
        let mut r = row(Health::Running);
        r.also_in = Some("larchsys".into());
        assert_eq!(
            session(&r, Some("m5/59".into()), Open::Attach, None, NOW).text(),
            "fm-demo-workspace\nstate  running\nopen  Attach in a new tab\nid  background · 77aa88bb\nuuid  77aa88bb-x\ncwd  C:\\nowhere\\demo\nclaimed  m5/59\nalso in  larchsys"
        );
        let why = "none: fm-demo-alpha exists already; a folder has one supervised session".to_string();
        assert!(session(&r, None, Open::Attach, Some(why.clone()), NOW).text().ends_with(&format!("
also in  larchsys
adopt  {why}")));
        r.closed = true;
        r.session.name.clear();
        assert!(session(&r, None, Open::Nothing, None, NOW).text().starts_with("(unnamed)\nstate  closed\nid  "));
    }

    #[test]
    fn an_interrupted_session_says_its_process_ended_and_another_foremans_says_whose() {
        let gone = row(Health::Orphaned { last_state: foreman_claude::SessionState::Working });
        let text = session(&gone, None, Open::Supervised, None, NOW).text();
        assert!(
            text.starts_with(
                "fm-demo-workspace\nstate  interrupted\n  the process ended while the session was working\n\
                 open  the supervisor runs it while it is wanted: Start the workspace from its menu\n"
            ),
            "{text}"
        );
        let mut foreign = row(Health::Running);
        foreign.foreign = true;
        assert!(session(&foreign, None, Open::Foreign, None, NOW).text().contains("\nopen  another Supraman session\n"));
    }
}

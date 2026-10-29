use std::path::Path;

use foreman_ui::hover_card::HoverCard;

use crate::fleet::{self, SessionRow};

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

pub fn workspace_card(facts: &WorkspaceFacts, now_ms: i64) -> HoverCard {
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
            rows.extend(waiting_for(own).map(|w| ("waiting", w)));
            rows.extend(own.hook.as_ref().map(|h| ("hook", h.label(now_ms))));
        }
        None => rows.push(("session", "not wanted".to_string())),
    }
    rows.push(("relay", facts.relay.clone().unwrap_or_else(|| "stopped".to_string())));
    rows.extend(facts.project.clone().map(|p| ("WorkState", p)));
    HoverCard { title: facts.name.clone(), rows }
}

pub fn session_card(row: &SessionRow, claimed: Option<String>, now_ms: i64) -> HoverCard {
    let s = &row.session;
    let title = if s.name.is_empty() { "(unnamed)".to_string() } else { s.name.clone() };
    let state = if row.closed { "closed".to_string() } else { fleet::health_label(&row.health) };
    let mut rows = vec![("state", state)];
    rows.extend(waiting_for(row).map(|w| ("waiting", w)));
    let id = if s.id.is_empty() { &s.session_id } else { &s.id };
    rows.push(("id", format!("{} · {id}", s.kind.as_str())));
    rows.push(("cwd", s.cwd.display().to_string()));
    rows.extend(worktree(&s.cwd).map(|w| ("worktree", w)));
    rows.extend(foreman_core::branch_at(&s.cwd).map(|b| ("branch", b)));
    rows.extend(row.hook.as_ref().map(|h| ("hook", h.label(now_ms))));
    rows.extend(claimed.map(|c| ("claimed", c)));
    rows.extend(row.also_in.clone().map(|w| ("also in", w)));
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
    fn the_workspace_card_names_home_folders_state_its_session_relay_and_project() {
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
            workspace_card(&facts, NOW).text(),
            "demo\nhome  C:\\nowhere\\demo\nfolders  Alpha, Beta\nstate  started\nsession  fm-demo-workspace · blocked · 77aa88bb\n\
             waiting  permission prompt\nhook  turn ended 12 s ago\nrelay  relay ready\nWorkState  autowork"
        );
        let bare = WorkspaceFacts { name: "gaia".into(), ..Default::default() };
        assert_eq!(workspace_card(&bare, NOW).text(), "gaia\nfolders  none\nstate  never started\nsession  not wanted\nrelay  stopped");
    }

    #[test]
    fn the_session_card_names_state_id_cwd_hook_claim_and_the_owner() {
        let mut r = row(Health::Running);
        r.also_in = Some("larchsys".into());
        assert_eq!(
            session_card(&r, Some("m5/59".into()), NOW).text(),
            "fm-demo-workspace\nstate  running\nid  background · 77aa88bb\ncwd  C:\\nowhere\\demo\nclaimed  m5/59\nalso in  larchsys"
        );
        r.closed = true;
        r.session.name.clear();
        assert!(session_card(&r, None, NOW).text().starts_with("(unnamed)\nstate  closed\n"));
    }
}

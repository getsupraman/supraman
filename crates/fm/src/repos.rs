//! The desired state and the session shape are `foreman_supervisor`'s; the supervisor in the app acts on the file
//! at its next tick.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use foreman_claude::{Session, list_sessions, now_ms};
use foreman_core::{HOOK_EXE, Registry, Repo, Workspace, find_plugin_dir};
use foreman_supervisor::{
    DesiredState, REPO_ROLE, Wanted, fill_paths, render_repo_role, repo_session, repo_session_name,
};
use serde_json::json;

use crate::Failure;

pub fn want(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, repo, prompt_file) = match args {
        [w, repo] => (*w, *repo, None),
        [w, repo, "--prompt-file", path] => (*w, *repo, Some(*path)),
        _ => return Err(Failure::Usage("want needs <workspace> <repo> [--prompt-file <path>]".into())),
    };
    let workspace = registry.load(w)?;
    let repo = find_repo(&workspace, repo)?;
    let mut prompt = render_repo_role(&repo_template(registry.home())?, w, &repo.name);
    if let Some(path) = prompt_file {
        let task = std::fs::read_to_string(path).map_err(|e| Failure::Usage(format!("--prompt-file {path}: {e}")))?;
        prompt = format!("{prompt}\n\nFirst task:\n{}", task.trim());
    }
    let session = repo_session(w, &repo.name, &repo.path, prompt)
        .ok_or_else(|| Failure::Usage(format!("repo {} would take a name of the workspace's own sessions", repo.name)))?;
    let (desired, done) = DesiredState::update(registry.home(), w, |d| d.want(session.clone()))?;
    let what = match done {
        Wanted::Added => "added",
        Wanted::Updated => "updated (a live session keeps its launch prompt)",
        Wanted::Unchanged => "already wanted",
    };
    println!("{}: {what}; repo {}, cwd {}", session.name, repo.name, repo.path.display());
    if desired.started {
        println!("the supervisor launches it at its next tick while Foreman runs");
    } else {
        println!("notice: workspace {w} is not started; it launches on Start");
    }
    Ok(())
}

/// Retired as well, so the supervisor stops a live one.
pub fn unwant(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let [w, repo] = args else {
        return Err(Failure::Usage("unwant needs <workspace> <repo>".into()));
    };
    let workspace = registry.load(w)?;
    // a repo gone from the registry can still be unwanted by its name
    let name = match find_repo(&workspace, repo) {
        Ok(r) => r.name.clone(),
        Err(_) => (*repo).to_string(),
    };
    let session = repo_session_name(w, &name)
        .ok_or_else(|| Failure::Usage(format!("repo {name} would take a name of the workspace's own sessions")))?;
    let (_, was) = DesiredState::update(registry.home(), w, |d| d.unwant(&session))?;
    if was {
        println!("{session}: no longer wanted; the supervisor stops it at its next tick (stop, never rm)");
    } else {
        println!("{session}: was not wanted; a live one is stopped all the same");
    }
    Ok(())
}

/// By name, then ignoring case, then by slug; else the known names.
fn find_repo<'a>(workspace: &'a Workspace, name: &str) -> Result<&'a Repo, Failure> {
    let slug = foreman_core::slugify(name);
    workspace
        .repos
        .iter()
        .find(|r| r.name == name)
        .or_else(|| workspace.repos.iter().find(|r| r.name.eq_ignore_ascii_case(name)))
        .or_else(|| workspace.repos.iter().find(|r| foreman_core::slugify(&r.name) == slug))
        .ok_or_else(|| {
            let known: Vec<&str> = workspace.repos.iter().map(|r| r.name.as_str()).collect();
            Failure::Usage(format!(
                "workspace {} has no repo '{name}'; known: {}",
                workspace.name,
                if known.is_empty() { "none".into() } else { known.join(", ") }
            ))
        })
}

/// `<home>/roles/repo.md` as the app wrote it (`{fm}`/`{home}` filled), else the template of the plugin
/// this `fm.exe` belongs to (`<plugin>/bin/fm.exe`, or `<repo>/target/<profile>/fm.exe`).
fn repo_template(home: &Path) -> Result<String, Failure> {
    let file = format!("{REPO_ROLE}.md");
    if let Ok(text) = std::fs::read_to_string(home.join("roles").join(&file)) {
        return Ok(text);
    }
    let exe = std::env::current_exe().map_err(|e| Failure::Usage(format!("cannot locate fm.exe: {e}")))?;
    let plugin = find_plugin_dir(&exe)
        .map_err(|e| Failure::Usage(format!("no {}, and {e}: start Foreman once", home.join("roles").join(&file).display())))?;
    let template = std::fs::read_to_string(plugin.join("roles").join(&file))
        .map_err(|e| Failure::Usage(format!("{}: {e}", plugin.join("roles").join(&file).display())))?;
    Ok(fill_paths(&template, &plugin.join("bin").join(HOOK_EXE), home))
}

/// One session of the roster: a row of `claude agents`, a wanted session without one, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The wanted session's role; `retired` for one `fm unwant` retired; `-` for a row nothing wants.
    pub role: String,
    pub repo: Option<String>,
    /// [`foreman_claude::Health`], or `not launched` for a wanted session without a row.
    pub health: String,
    pub id: Option<String>,
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    pub wanted: bool,
}

pub fn roster_cmd(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, as_json) = match args {
        [w] => (*w, false),
        [w, "--json"] | ["--json", w] => (*w, true),
        _ => return Err(Failure::Usage("roster needs <workspace> [--json]".into())),
    };
    registry.load(w)?;
    let others: Vec<String> = registry.list()?.into_iter().map(|p| p.name).filter(|n| n != w).collect();
    let desired = DesiredState::load(registry.home(), w)?.unwrap_or_else(|| DesiredState::new(w));
    crate::check_claude()?;
    let entries = roster(&desired, &list_sessions()?, &others, now_ms());
    print!("{}", if as_json { render_json(&desired, &entries) } else { render(&entries) });
    Ok(())
}

/// Rows named `fm-<w>-…` (not those of a registered workspace whose name extends `<w>-`, unless `<w>`
/// wants them), in desired order first, then the rest by name; then wanted sessions without a row.
pub fn roster(desired: &DesiredState, rows: &[Session], others: &[String], now: i64) -> Vec<Entry> {
    let w = &desired.workspace;
    let prefix = format!("fm-{w}-");
    let wanted = |name: &str| desired.sessions.iter().find(|s| s.name == name);
    let retired = |name: &str| desired.retired.iter().any(|n| n == name);
    let other_prefixes: Vec<String> =
        others.iter().filter(|o| o.starts_with(&format!("{w}-"))).map(|o| format!("fm-{o}-")).collect();
    let mut ours: Vec<&Session> = rows
        .iter()
        .filter(|s| s.name.starts_with(&prefix))
        .filter(|s| wanted(&s.name).is_some() || retired(&s.name) || !other_prefixes.iter().any(|p| s.name.starts_with(p)))
        .collect();
    let order = |name: &str| desired.sessions.iter().position(|s| s.name == name).unwrap_or(usize::MAX);
    ours.sort_by(|a, b| order(&a.name).cmp(&order(&b.name)).then_with(|| a.name.cmp(&b.name)).then_with(|| a.id.cmp(&b.id)));
    let mut out: Vec<Entry> = ours
        .into_iter()
        .map(|s| {
            let w = wanted(&s.name);
            Entry {
                name: s.name.clone(),
                role: match w {
                    Some(w) => w.role.clone(),
                    None if retired(&s.name) => "retired".into(),
                    None => "-".into(),
                },
                repo: w.and_then(|w| w.repo.clone()),
                health: s.supervised_health(now).to_string(),
                id: Some(s.id.clone()),
                session_id: Some(s.session_id.clone()),
                cwd: s.cwd.clone(),
                wanted: w.is_some(),
            }
        })
        .collect();
    for w in &desired.sessions {
        if !rows.iter().any(|s| s.name == w.name) {
            out.push(Entry {
                name: w.name.clone(),
                role: w.role.clone(),
                repo: w.repo.clone(),
                health: "not launched".into(),
                id: None,
                session_id: None,
                cwd: w.cwd.clone(),
                wanted: true,
            });
        }
    }
    out
}

/// One line per session: name, role, health, id, cwd, tab-separated.
pub fn render(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "no sessions\n".into();
    }
    let mut out = String::new();
    for e in entries {
        let _ = writeln!(out, "{}\t{}\t{}\t{}\t{}", e.name, e.role, e.health, e.id.as_deref().unwrap_or("-"), e.cwd.display());
    }
    out
}

pub fn render_json(desired: &DesiredState, entries: &[Entry]) -> String {
    let sessions: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            json!({
                "name": e.name,
                "role": e.role,
                "repo": e.repo,
                "health": e.health,
                "id": e.id,
                "sessionId": e.session_id,
                "cwd": e.cwd.display().to_string(),
                "wanted": e.wanted,
            })
        })
        .collect();
    let mut text = json!({ "workspace": desired.workspace, "started": desired.started, "sessions": sessions }).to_string();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use foreman_claude::parse;
    use foreman_supervisor::{relay_session, workspace_session_name};

    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn rows() -> Vec<Session> {
        let old = NOW - 600_000;
        parse(&format!(
            r#"[
  {{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"fm-demo-alpha","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"working","status":"busy","pid":1}},
  {{"id":"aaaa0002","sessionId":"aaaa0002-x","name":"fm-demo-workspace","cwd":"C:\\demo","kind":"background","startedAt":{old},"state":"working","status":"idle","pid":2}},
  {{"id":"aaaa0003","sessionId":"aaaa0003-x","name":"fm-demo-beta","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"stopped"}},
  {{"id":"aaaa0004","sessionId":"aaaa0004-x","name":"fm-demo-x-workspace","cwd":"C:\\demo-x","kind":"background","startedAt":{old},"state":"stopped"}},
  {{"id":"aaaa0005","sessionId":"aaaa0005-x","name":"fm-other-workspace","cwd":"C:\\o","kind":"background","startedAt":{old},"state":"stopped"}},
  {{"id":"aaaa0006","sessionId":"aaaa0006-x","name":"fm-demo-stray","cwd":"C:\\demo","kind":"background","startedAt":{old},"state":"stopped"}}
]"#
        ))
        .unwrap()
    }

    fn desired() -> DesiredState {
        let mut d = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        d.started = true;
        d.sessions.insert(0, relay_session("demo", r"C:\demo", r"C:\fm\relay\demo", "r"));
        d.want(repo_session("demo", "Alpha", r"C:\demo\Alpha", "p").unwrap());
        d.unwant("fm-demo-beta");
        d
    }

    #[test]
    fn roster_lists_the_workspaces_rows_and_wanted_sessions_without_one() {
        let entries = roster(&desired(), &rows(), &["demo-x".into(), "other".into()], NOW);
        let text = render(&entries);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "fm-demo-workspace\tworkspace\tidle\taaaa0002\tC:\\demo",
                "fm-demo-alpha\trepo\trunning\taaaa0001\tC:\\demo\\Alpha",
                "fm-demo-beta\tretired\tstopped\taaaa0003\tC:\\demo\\Beta",
                "fm-demo-stray\t-\tstopped\taaaa0006\tC:\\demo",
                "fm-demo-relay\trelay\tnot launched\t-\tC:\\demo",
            ],
            "{text}"
        );
        assert_eq!(render(&[]), "no sessions\n");
        assert_eq!(workspace_session_name("demo"), "fm-demo-workspace");
    }

    #[test]
    fn roster_json_has_the_same_sessions() {
        let d = desired();
        let entries = roster(&d, &rows(), &["demo-x".into()], NOW);
        let v: serde_json::Value = serde_json::from_str(&render_json(&d, &entries)).unwrap();
        assert_eq!(v["workspace"], "demo");
        assert_eq!(v["started"], true);
        let alpha = &v["sessions"][1];
        assert_eq!(alpha["name"], "fm-demo-alpha");
        assert_eq!(alpha["repo"], "Alpha");
        assert_eq!(alpha["health"], "running");
        assert_eq!(alpha["sessionId"], "aaaa0001-x");
        assert_eq!(alpha["wanted"], true);
        assert_eq!(v["sessions"][4]["id"], serde_json::Value::Null);
        assert_eq!(v["sessions"].as_array().unwrap().len(), 5);
    }
}

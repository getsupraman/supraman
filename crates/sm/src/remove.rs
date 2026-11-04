//! `sm workspace remove`: the app's Remove without the app, whose supervisor only reads registered workspaces.

use std::path::Path;
use std::time::{Duration, Instant};

use supraman_claude::{Session, now_ms};
use supraman_supervisor::{DesiredState, is_live, remove_workspace};

use crate::run::Agents;

const STOPPED_WITHIN: Duration = Duration::from_secs(20);

/// Stops the workspace (`started = false`) and each live session it wants or retired (stop, never rm), waits until
/// none runs, then [`remove_workspace`]. Returns the names stopped. No token is revoked.
pub fn remove(home: &Path, workspace: &str, claude: &dyn Agents) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    if DesiredState::load(home, workspace).map_err(|e| e.to_string())?.is_some() {
        let (desired, ()) = DesiredState::update(home, workspace, |d| d.started = false).map_err(|e| e.to_string())?;
        names = desired.sessions.iter().map(|s| s.name.clone()).chain(desired.retired).collect();
    }
    let mut stopped = Vec::new();
    if !names.is_empty() {
        for s in live(claude, &names)? {
            claude.stop(&s.id).map_err(|e| format!("cannot stop {} ({}): {e}", s.name, s.id))?;
            stopped.push(s.name);
        }
        let deadline = Instant::now() + STOPPED_WITHIN;
        while let Some(s) = live(claude, &names)?.into_iter().next() {
            if Instant::now() >= deadline {
                return Err(format!("{} ({}) still runs; workspace {workspace} stays registered", s.name, s.id));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    remove_workspace(home, workspace).map_err(|e| e.to_string())?;
    Ok(stopped)
}

fn live(claude: &dyn Agents, names: &[String]) -> Result<Vec<Session>, String> {
    let now = now_ms();
    let rows = claude.list().map_err(|e| e.to_string())?;
    Ok(rows.into_iter().filter(|s| names.contains(&s.name) && is_live(s, now)).collect())
}

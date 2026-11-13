//! Token-less configs under `<home>/mcp` (a [+] tab signed out, `Keeper::launch_unminted`) once their session is gone.

use std::collections::{BTreeMap, BTreeSet};

use supraman_claude::Session;

use super::{Keeper, MintedSessions, record_files, remove_mcp_config};
use crate::Millis;

/// Each token-less config's name missing from the listings, with when the first listing in a row that lacked it was read.
#[derive(Debug, Default)]
pub struct Unlisted(BTreeMap<String, Millis>);

impl Keeper {
    /// Deletes each config with no record or kept token that two non-empty listings in a row lack ([`super::Absent`]), but `held`'s or one written since; what failed.
    pub fn sweep_unminted(&self, unlisted: &mut Unlisted, observed: &[Session], held: &[String], now: Millis) -> Vec<String> {
        if observed.is_empty() {
            unlisted.0.clear();
            return Vec::new();
        }
        let Some(recorded) = recorded_names(&self.home) else {
            return Vec::new();
        };
        let mut missing = BTreeMap::new();
        let mut failed = Vec::new();
        for name in supraman_claude::mcp_config_names(&self.mcp_dir) {
            if recorded.contains(&name) || held.contains(&name) || observed.iter().any(|s| s.name == name) || self.kept(&name).is_some() {
                continue;
            }
            let written = supraman_claude::mcp_config_path(&self.mcp_dir, &name).ok().and_then(|p| std::fs::metadata(p).ok()).map(|m| supraman_core::time::modified_ms(&m));
            match unlisted.0.get(&name) {
                Some(&since) if written.is_some_and(|w| w < since) => match remove_mcp_config(&self.mcp_dir, &name) {
                    Ok(()) => log::info!("{name}: no session of that name is listed, so its token-less Workman config is deleted"),
                    Err(e) => failed.push(e),
                },
                _ => drop(missing.insert(name, now)),
            }
        }
        unlisted.0 = missing;
        failed
    }

    /// `name`'s config while it has no record and no kept token, which stay [`super::revoke`]'s; what failed.
    pub fn forget_unminted(&self, name: &str) -> Option<String> {
        if self.kept(name).is_some() || recorded_names(&self.home).is_none_or(|r| r.contains(name)) {
            return None;
        }
        remove_mcp_config(&self.mcp_dir, name).err()
    }
}

/// Every name of the home's record files; `None` when one cannot be read, so nothing is deleted on a guess.
fn recorded_names(home: &std::path::Path) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for path in record_files(home) {
        match MintedSessions::load(path.clone()) {
            Ok(records) => names.extend(records.names().cloned()),
            Err(e) => {
                log::warn!("Cannot read {} ({e}): no token-less Workman config is deleted", path.display());
                return None;
            }
        }
    }
    Some(names)
}

#[cfg(test)]
mod tests;

//! What a home kept for the workspace controllers and the relay, which Supraman no longer runs.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::mint::{Bearer, Keeper, MintedSessions, Minter, files_ending, revoke};

/// The role files `<home>/roles` held for them; the hand's stays.
const ROLES: [&str; 3] = ["workspace.md", "relay.md", "repo.md"];

/// Revokes each session a `<home>/supervisor/<workspace>.sessions.json` records (its config and kept token go with it),
/// then deletes those records, the wanted-session files beside them, the role files and the relay's folder. The claude
/// sessions themselves are left as they are. What failed; nothing is retried.
pub fn end_controllers(keeper: &Keeper, minter: &dyn Minter, bearer: impl FnOnce() -> Bearer) -> Vec<String> {
    let home = keeper.home().to_path_buf();
    let supervisor = home.join("supervisor");
    let records = files_ending(&supervisor, ".sessions.json");
    let mut failed = Vec::new();
    let bearer = (!records.is_empty()).then(bearer);
    for path in &records {
        match MintedSessions::load(path.clone()) {
            Ok(minted) => {
                let bearer = bearer.as_ref().expect("read once records exist");
                for name in minted.names() {
                    if let Some(m) = minted.get(name) {
                        log::info!("{name}: a workspace controller's or relay's record, so its Workman token is revoked");
                        failed.extend(revoke(keeper, minter, bearer, name, m).notices);
                    }
                }
            }
            Err(e) => failed.push(format!("Cannot read {} ({e}); it is deleted, its tokens lapse unused", path.display())),
        }
    }
    let wanted: Vec<PathBuf> = files_ending(&supervisor, ".toml").into_iter().flat_map(|toml| [toml.with_extension("lock"), toml]).collect();
    let wanted = wanted.into_iter().chain(files_ending(&supervisor, ".sessions.json.damaged"));
    let roles = ROLES.iter().map(|f| home.join("roles").join(f));
    for file in records.into_iter().chain(wanted).chain(roles) {
        failed.extend(delete(&file));
    }
    failed.extend(delete_dir(&home.join("relay")));
    failed
}

fn delete(file: &Path) -> Option<String> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != ErrorKind::NotFound => Some(format!("Cannot delete {}: {e}", file.display())),
        _ => None,
    }
}

fn delete_dir(dir: &Path) -> Option<String> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != ErrorKind::NotFound => Some(format!("Cannot delete {}: {e}", dir.display())),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

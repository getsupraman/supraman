use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::store::schedules_dir;
use crate::{Result, validate_name};

/// Where `fire.log` rotates.
pub const FIRE_LOG_CAP: u64 = 1024 * 1024;
/// How much of a script's stderr a run record keeps.
pub const STDERR_TAIL: usize = 2048;
/// What a secret's value reads as in a recorded stderr tail.
pub const MASK: &str = "***";

/// The run in flight of one schedule at `schedules/live/<key>/<id>.json`; never a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Live {
    pub run: String,
    pub fm_pid: u32,
    /// `process::started(fm_pid)`: a pid reused by another process does not match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fm_started: Option<u64>,
    pub started_at: i64,
    pub stage: String,
    /// The run's claude session, once launched: what recovery stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<LiveSession>,
}

/// A run's launched session as its live file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSession {
    pub name: String,
    pub id: String,
    /// The minted Workman session id, with the origin its revoke goes to; never the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workstate_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// What `sm schedule report` hands the run's owner at `schedules/live/<key>/<id>.report`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reported {
    pub run: String,
    pub outcome: super::Outcome,
    #[serde(default)]
    pub why: String,
}

/// `schedules/live/<key>/`: each schedule's live file, overlap lock, `.queued` and `.restart`.
pub fn live_dir(home: &Path, key: &str) -> Result<PathBuf> {
    validate_name(key)?;
    Ok(schedules_dir(home).join("live").join(key))
}

/// `schedules/state/<key>/<id>/`, the script's own dir, kept between runs.
pub fn state_dir(home: &Path, key: &str, id: &str) -> Result<PathBuf> {
    validate_name(key)?;
    validate_name(id)?;
    Ok(schedules_dir(home).join("state").join(key).join(id))
}

/// `schedules/fire.log`: fire's diagnostics, appended with `append_capped` at [`FIRE_LOG_CAP`].
pub fn fire_log(home: &Path) -> PathBuf {
    schedules_dir(home).join("fire.log")
}

/// The `SecretStore` key of secret `name` of schedule `id` in the folder of [`super::folder_key`] `key`.
pub fn secret_key(key: &str, id: &str, name: &str) -> String {
    format!("supraman/schedule/{key}/{id}/{name}")
}

/// `text` with each non-empty secret replaced by [`MASK`], then its last `max` bytes (cut on a char boundary).
pub fn masked_tail(text: &str, secrets: &[String], max: usize) -> String {
    let mut masked = text.to_string();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        masked = masked.replace(s.as_str(), MASK);
    }
    let mut from = masked.len().saturating_sub(max);
    while !masked.is_char_boundary(from) {
        from += 1;
    }
    masked[from..].to_string()
}

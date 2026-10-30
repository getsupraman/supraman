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

/// The run in flight of one schedule at `schedules/live/<ws>/<id>.json`; never a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Live {
    pub run: String,
    pub fm_pid: u32,
    /// `process::started(fm_pid)`: a pid reused by another process does not match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fm_started: Option<u64>,
    pub started_at: i64,
    pub stage: String,
}

/// `schedules/live/<ws>/`: each schedule's live file, overlap lock, `.queued` and `.restart`.
pub fn live_dir(home: &Path, workspace: &str) -> Result<PathBuf> {
    validate_name(workspace)?;
    Ok(schedules_dir(home).join("live").join(workspace))
}

/// `schedules/state/<ws>/<id>/`, the script's own dir, kept between runs.
pub fn state_dir(home: &Path, workspace: &str, id: &str) -> Result<PathBuf> {
    validate_name(workspace)?;
    validate_name(id)?;
    Ok(schedules_dir(home).join("state").join(workspace).join(id))
}

/// `schedules/fire.log`: fire's diagnostics, appended with `append_capped` at [`FIRE_LOG_CAP`].
pub fn fire_log(home: &Path) -> PathBuf {
    schedules_dir(home).join("fire.log")
}

/// The `SecretStore` key of a schedule's secret `name`.
pub fn secret_key(workspace: &str, id: &str, name: &str) -> String {
    format!("foreman/schedule/{workspace}/{id}/{name}")
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

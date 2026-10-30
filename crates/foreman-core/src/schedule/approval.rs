use std::collections::BTreeMap;
use std::fmt;
use std::io::ErrorKind;
use std::path::Path;

use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::store::{edit_raw, schedules_dir};
use super::{Action, ActionKind, Approval, Caps, Permissions, Policy, Prompt, Schedule, Span, Status, Trigger};
use crate::secret::SecretStore;
use crate::{CoreError, Result, Workspace};

/// The `SecretStore` key of the approval key: 32 random bytes as hex.
pub const APPROVAL_KEY: &str = "foreman/schedule-approval/v1";
const MAC_DOMAIN: &str = "foreman-schedule-v1";
const MISSING: &str = "missing";

/// The fingerprinted JSON; field order is the canonical order of docs/research/scheduler.md.
#[derive(Serialize)]
struct CoveredJson<'a> {
    v: u32,
    workspace: &'a str,
    id: &'a str,
    folder: &'a str,
    trigger: &'a Trigger,
    action: CoveredAction<'a>,
    permissions: &'a Permissions,
    caps: &'a Caps,
    policy: &'a Policy,
    files: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct CoveredAction<'a> {
    kind: ActionKind,
    prompt: Option<CoveredPrompt<'a>>,
    script: Option<&'a str>,
    script_args: &'a [String],
    script_timeout: Span,
    worktree: bool,
    model: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum CoveredPrompt<'a> {
    Command { name: &'a str, path: Option<String> },
    File(&'a str),
    Text(&'a str),
}

/// What a schedule's approval covers, computed now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Covered {
    pub json: String,
    /// Hex sha256 of `json`.
    pub fingerprint: String,
    /// The resolved slash command file's hash; not covered.
    pub command_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotApproved,
    /// The covered fields or files differ from the approved ones.
    Changed,
    BadMac,
    /// Nothing was ever approved in this home.
    NoKey,
    Key(String),
    Unreadable(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotApproved => f.write_str("the schedule was never approved"),
            Self::Changed => f.write_str("the schedule changed since its approval"),
            Self::BadMac => f.write_str("the approval does not verify"),
            Self::NoKey => f.write_str("no approval key: nothing was approved in this Foreman home"),
            Self::Key(e) => write!(f, "cannot read the approval key: {e}"),
            Self::Unreadable(e) => write!(f, "cannot compute the fingerprint: {e}"),
        }
    }
}

/// The covered fields alone, without the files: what an edit compares.
pub(super) fn covered_toml(workspace: &str, s: &Schedule) -> String {
    serde_json::to_string(&covered_json(workspace, s, None)).expect("plain data serializes")
}

fn covered_json<'a>(workspace: &'a str, s: &'a Schedule, command_path: Option<String>) -> CoveredJson<'a> {
    let a: &Action = &s.action;
    CoveredJson {
        v: 1,
        workspace,
        id: &s.id,
        folder: &s.folder,
        trigger: &s.trigger,
        action: CoveredAction {
            kind: a.kind,
            prompt: a.prompt.as_ref().map(|p| match p {
                Prompt::Command(name) => CoveredPrompt::Command { name, path: command_path.clone() },
                Prompt::File(file) => CoveredPrompt::File(file),
                Prompt::Text(text) => CoveredPrompt::Text(text),
            }),
            script: a.script.as_deref(),
            script_args: &a.script_args,
            script_timeout: a.script_timeout,
            worktree: a.worktree,
            model: &a.model,
        },
        permissions: &s.permissions,
        caps: &s.caps,
        policy: &s.policy,
        files: BTreeMap::new(),
    }
}

/// `s`'s covered JSON and fingerprint, hashing its script and prompt file in `workspace`'s folder.
pub fn covered(workspace: &Workspace, s: &Schedule) -> Result<Covered> {
    let folder = workspace.folder(&s.folder).ok_or_else(|| CoreError::NoFolder {
        workspace: workspace.name.clone(),
        folder: s.folder.clone(),
        known: workspace.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "),
    })?;
    let mut files = BTreeMap::new();
    let mut hash = |rel: &str| -> Result<()> {
        files.insert(rel.to_string(), sha256_file(&folder.path.join(rel))?.unwrap_or_else(|| MISSING.into()));
        Ok(())
    };
    if let Some(script) = &s.action.script {
        hash(script)?;
    }
    if let Some(Prompt::File(file)) = &s.action.prompt {
        hash(file)?;
    }
    let (command_path, command_sha256) = match &s.action.prompt {
        Some(Prompt::Command(name)) => {
            let (shown, file) = command_file(&folder.path, name);
            (Some(shown), sha256_file(&file)?)
        }
        _ => (None, None),
    };
    let mut doc = covered_json(&workspace.name, s, command_path);
    doc.files = files;
    let json = serde_json::to_string(&doc).expect("plain data serializes");
    Ok(Covered { fingerprint: hex(&Sha256::digest(json.as_bytes())), json, command_sha256 })
}

/// `/a:b` → the folder's `.claude/commands/a/b.md` (shown relative), else the user's `commands/a/b.md` when
/// it exists (shown absolute), else the folder's path as not found.
fn command_file(folder: &Path, name: &str) -> (String, std::path::PathBuf) {
    let rel = format!(".claude/commands/{}.md", name.trim_start_matches('/').replace(':', "/"));
    let project = folder.join(&rel);
    if !project.is_file()
        && let Some(user) = crate::paths::claude_dir().map(|d| d.join(&rel[".claude/".len()..])).filter(|p| p.is_file())
    {
        return (crate::paths::normalize(&user), user);
    }
    (rel, project)
}

fn sha256_file(path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(hex(&Sha256::digest(&bytes)))),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(path, e)),
    }
}

fn hmac_of(key: &[u8], workspace: &str, id: &str, fingerprint: &str) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    m.update(format!("{MAC_DOMAIN}\n{workspace}\n{id}\n{fingerprint}").as_bytes());
    m
}

/// HMAC-SHA256 of the approved fingerprint, as hex.
pub fn mac(key: &[u8], workspace: &str, id: &str, fingerprint: &str) -> String {
    hex(&hmac_of(key, workspace, id, fingerprint).finalize().into_bytes())
}

/// `approval` holds for `fingerprint_now` under `key`; the MAC compares in constant time.
pub fn verify(approval: &Approval, workspace: &str, id: &str, fingerprint_now: &str, key: &[u8]) -> std::result::Result<(), Refusal> {
    if approval.fingerprint != fingerprint_now {
        return Err(Refusal::Changed);
    }
    let tag = unhex(&approval.mac).ok_or(Refusal::BadMac)?;
    hmac_of(key, workspace, id, &approval.fingerprint).verify_slice(&tag).map_err(|_| Refusal::BadMac)
}

/// Checks 2 and 3 of a fire: the fingerprint now is the approved one and its MAC verifies under the key.
pub fn verify_approval(store: &dyn SecretStore, workspace: &Workspace, s: &Schedule) -> std::result::Result<Covered, Refusal> {
    let approval = s.approval.as_ref().ok_or(Refusal::NotApproved)?;
    let key = read_key(store)?.ok_or(Refusal::NoKey)?;
    let now = covered(workspace, s).map_err(|e| Refusal::Unreadable(e.to_string()))?;
    verify(approval, &workspace.name, &s.id, &now.fingerprint, &key)?;
    Ok(now)
}

/// `status`, except that an `active` schedule whose approval does not verify counts as `pending`.
pub fn effective_status(store: &dyn SecretStore, workspace: &Workspace, s: &Schedule) -> Status {
    match s.status {
        Status::Active if verify_approval(store, workspace, s).is_err() => Status::Pending,
        status => status,
    }
}

fn read_key(store: &dyn SecretStore) -> std::result::Result<Option<Vec<u8>>, Refusal> {
    match store.get(APPROVAL_KEY).map_err(Refusal::Key)? {
        None => Ok(None),
        Some(hexed) => unhex(&hexed).filter(|k| k.len() == 32).map(Some).ok_or_else(|| Refusal::Key("not 32 bytes of hex".into())),
    }
}

/// René's Approve in the Foreman window, and nothing else (C100): records the approval of schedule `id` as
/// it is now and makes it active. Creates the key on the first approval in this home.
pub fn approve(home: &Path, store: &dyn SecretStore, workspace: &Workspace, id: &str, now_ms: i64) -> Result<Schedule> {
    let key = {
        let lock = schedules_dir(home).join("approval.lock");
        let _held = crate::fs::lock_file(&lock).map_err(|e| CoreError::io(&lock, e))?;
        match read_key(store).map_err(|e| CoreError::Secret(e.to_string()))? {
            Some(key) => key,
            None => {
                let mut key = [0u8; 32];
                getrandom::fill(&mut key).map_err(|e| CoreError::Secret(format!("no random source: {e}")))?;
                store.set(APPROVAL_KEY, &hex(&key)).map_err(CoreError::Secret)?;
                key.to_vec()
            }
        }
    };
    edit_raw(home, workspace, |schedules| {
        let s = schedules
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| CoreError::NoSchedule { workspace: workspace.name.clone(), id: id.to_string() })?;
        s.validate(workspace).map_err(|why| CoreError::InvalidSchedule { id: s.id.clone(), why })?;
        let c = covered(workspace, s)?;
        s.approval = Some(Approval {
            mac: mac(&key, &workspace.name, id, &c.fingerprint),
            fingerprint: c.fingerprint,
            approved_at: now_ms,
            covered: c.json,
            command_sha256: c.command_sha256,
        });
        s.status = Status::Active;
        Ok(s.clone())
    })
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect()
}

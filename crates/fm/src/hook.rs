//! `fm hook <event>`: run by `plugins/foreman/hooks/hooks.json` inside every session Foreman launches.
//! Appends one [`Event`] line to `<FOREMAN_HOME>/events/<session_id>.jsonl`; on SessionStart also
//! prints the role text for `FOREMAN_ROLE` (`{workspace}` filled from `FOREMAN_WORKSPACE`, `{repo}` from
//! `FOREMAN_REPO`), which Claude
//! Code adds to the session's context.
//! Never fails: whatever goes wrong, nothing is printed and the exit code is 0 (see `main`).

use std::io::{Read, Write};
use std::path::PathBuf;

use foreman_core::{Event, ROTATE_AT, append_event, role_injection_for};

/// A UserPromptSubmit payload carries the whole prompt; a larger one is not parsed.
const MAX_PAYLOAD: u64 = 8 << 20;

pub fn run(event: &str) {
    let mut payload = String::new();
    let _ = std::io::stdin().lock().take(MAX_PAYLOAD).read_to_string(&mut payload);
    let Some(home) = std::env::var_os("FOREMAN_HOME").filter(|h| !h.is_empty()).map(PathBuf::from) else {
        return;
    };
    if let Some(e) = Event::from_payload(event, &payload, foreman_core::time::now_ms()) {
        let _ = append_event(&home, &e, ROTATE_AT);
    }
    let workspace = std::env::var("FOREMAN_WORKSPACE").ok();
    if event == "SessionStart"
        && let Some(role) = std::env::var("FOREMAN_ROLE").ok()
        && let Some(text) = role_injection_for(&home, &role, workspace.as_deref(), std::env::var("FOREMAN_REPO").ok().as_deref())
    {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

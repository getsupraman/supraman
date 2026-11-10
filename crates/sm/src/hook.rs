//! Run by `plugins/supraman/hooks/hooks.json`. Never fails: whatever goes wrong, nothing is printed and the exit code is 0.

use std::io::{Read, Write};
use supraman_core::home::SessionEnv;
use supraman_core::{Event, ROTATE_AT, append_event, role_injection_for};

/// A UserPromptSubmit payload carries the whole prompt; a larger one is not parsed.
const MAX_PAYLOAD: u64 = 8 << 20;

pub fn run(event: &str) {
    let mut payload = String::new();
    let _ = std::io::stdin().lock().take(MAX_PAYLOAD).read_to_string(&mut payload);
    let Some(home) = supraman_core::env_home() else {
        return;
    };
    if let Some(e) = Event::from_payload(event, &payload, supraman_core::time::now_ms()) {
        let _ = append_event(&home, &e, ROTATE_AT);
    }
    let env = SessionEnv::from_env();
    if event == "SessionStart"
        && let Some(role) = &env.role
        && let Some(text) = role_injection_for(&home, role, env.workspace.as_deref(), env.repo.as_deref())
    {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

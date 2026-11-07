//! Settings → Workman's server switch: server.json written, the old server left once a sign-in on the new one holds.

use std::path::PathBuf;

use gpui::Context;
use supraman_core::workman_server::MCP_NAME;
use supraman_workman::{Auth, AuthStatus};

use super::{FleetView, stale_tabs};
use crate::work_view::WorkHub;
use crate::workman_setup::WorkmanStatus;

#[derive(Debug, PartialEq, Eq)]
enum Switch {
    Same,
    SignInFirst { from: String },
    Save,
}

/// Decided from server.json and the sign-in as they are now, never from a status still being checked; blocking.
fn switch(auth: &Auth, origin: &str) -> Switch {
    let Ok(server) = auth.server() else {
        return Switch::Save;
    };
    let current = server.origin();
    if current == origin {
        return if server.mcp_name == MCP_NAME { Switch::Same } else { Switch::Save };
    }
    match auth.status(&current) {
        AuthStatus::SignedIn { .. } => Switch::SignInFirst { from: current },
        _ => Switch::Save,
    }
}

impl FleetView {
    pub fn set_workman_url(&mut self, text: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let home = self.home.clone().ok_or("No Supraman home: settings cannot be saved.")?;
        let origin = supraman_core::validate_workman_url(text)?;
        let decided = {
            let (auth, origin) = (crate::signin::auth(&home), origin.clone());
            cx.background_executor().spawn(async move { switch(&auth, &origin) })
        };
        cx.spawn(async move |this, cx| {
            let decided = decided.await;
            this.update(cx, |this, cx| match decided {
                Switch::Same => {}
                Switch::SignInFirst { from } => this.sign_in_to(origin, Some(from), cx),
                Switch::Save => this.save_workman_url(home, origin, None, cx),
            })
            .ok();
        })
        .detach();
        Ok(())
    }

    /// `origin` into server.json off the UI thread, then a recheck; `left` is the server signed out of once it is saved.
    pub(super) fn save_workman_url(&mut self, home: PathBuf, origin: String, left: Option<String>, cx: &mut Context<Self>) {
        let auth = crate::signin::auth(&home);
        let saved = cx.background_executor().spawn(async move { auth.save_server(&origin) });
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            this.update(cx, |this, cx| {
                this.workman = WorkmanStatus::Checking;
                match (saved, left) {
                    (Ok(()), Some(left)) => this.leave_servers(vec![left], cx),
                    (Ok(()), None) => {}
                    (Err(e), _) => this.action_error = Some(e),
                }
                this.recheck_workman();
                WorkHub::reconfigure(Some(home), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn sync_server(&mut self, origin: String, cx: &mut Context<Self>) {
        self.synced_server = Some(origin.clone());
        let Some(hands) = self.hands.clone() else {
            return;
        };
        let at = origin.clone();
        let task = cx.background_executor().spawn(async move {
            let mut hands = hands.lock().unwrap_or_else(|e| e.into_inner());
            (hands.elsewhere(&at), hands.take_left())
        });
        cx.spawn(async move |this, cx| {
            let (elsewhere, left) = task.await;
            this.update(cx, |this, cx| {
                let stale = stale_tabs(&this.tabs, &elsewhere);
                for tab in this.tabs.iter_mut().filter(|t| t.hand && !t.exited) {
                    tab.restart_to = stale.contains(&tab.id).then(|| origin.clone());
                }
                this.hand_notices.extend(crate::hand::restart_notice(stale.len(), &origin));
                this.leave_servers(elsewhere.into_iter().filter_map(|(_, o)| o).chain(left).collect(), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn leave_servers(&mut self, origins: Vec<String>, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone().filter(|_| !origins.is_empty()) else {
            return;
        };
        let auth = crate::signin::auth(&home);
        let current = self.workman.checked_url().map(str::to_string);
        let origins: Vec<String> = origins.into_iter().filter(|o| current.as_ref() != Some(o) && self.left_servers.insert(o.clone())).collect();
        if origins.is_empty() {
            return;
        }
        let revoke = cx.background_executor().spawn(async move { origins.iter().filter_map(|o| auth.sign_out(o).err()).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| {
            let failed = revoke.await;
            if !failed.is_empty() {
                this.update(cx, |this, cx| {
                    this.hand_notices.push(format!("The old server's grant was not revoked: {}", failed.join("; ")));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use supraman_core::workman_server::{self, ServerJson};
    use supraman_workman::fake::FakeWorkman;

    use super::*;
    use crate::test_auth;

    #[test]
    fn a_switch_reads_the_server_and_sign_in_as_saved_whatever_the_status_shows() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkman::start(vec![]);
        fake.enable_oauth();
        let auth = test_auth::signed_in(&home, fake.base());
        assert_eq!(switch(&auth, "https://other.example"), Switch::SignInFirst { from: fake.base().to_string() });
        assert_eq!(switch(&auth, fake.base()), Switch::Same);

        let path = test_auth::server_json(&home, fake.base());
        workman_server::save(&path, &ServerJson { mcp_name: "ws".into(), ..ServerJson::new(fake.base()) }).unwrap();
        assert_eq!(switch(&auth, fake.base()), Switch::Save, "the same server under another mcpName is written as workman");

        let unsigned = Auth::with_store(&dir.path().join("other"), Arc::new(supraman_workman::MemoryStore::default())).with_server_json(Some(path.clone()));
        assert_eq!(switch(&unsigned, "https://other.example"), Switch::Save, "not signed in: saved at once");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(switch(&auth, "https://other.example"), Switch::Save, "no server.json: saved at once");
    }
}

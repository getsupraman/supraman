//! The settings window's Workman page: the sign-in status, the server URL and the MCP server name.

use gpui::{AnyElement, App, ClickEvent, Context, Div, Hsla, SharedString, div, prelude::*, px};
use supraman_ui::settings_page::row;
use supraman_ui::{banner, button};
use supraman_workman::AuthStatus;
use supraman_workman::status::{RECHECK_EVERY, WorkmanStatus};
use theme::ActiveTheme as _;

use super::SettingsView;
use crate::workman_setup::ServerCheck;

impl SettingsView {
    /// server.json and claude's own `workman` entry, read off the UI thread.
    pub fn check_server(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        self.server_checks += 1;
        let check = self.server_checks;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { crate::workman_setup::check_server(&crate::signin::auth(&home), supraman_core::paths::claude_json().as_deref()) })
                .await;
            this.update(cx, |this, cx| {
                if this.server_checks == check {
                    this.server_check = Some(found);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    #[cfg(feature = "selftest")]
    pub fn server_check(&self) -> Option<&ServerCheck> {
        self.server_check.as_ref()
    }

    /// The selftest types into the URL field.
    #[cfg(feature = "selftest")]
    pub fn set_url_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.url_input.update(cx, |input, cx| input.set_text(text, cx));
    }

    pub fn apply_url(&mut self, cx: &mut Context<Self>) {
        let text = self.url_input.read(cx).text().to_string();
        let result = self.fleet.update(cx, |fleet, cx| fleet.set_workman_url(&text, cx)).unwrap_or(Err("the main window is closed".into()));
        match result {
            Ok(()) => {
                self.url_error = None;
                self.reload(cx);
                if let Ok(origin) = supraman_core::validate_workman_url(&text) {
                    self.url_input.update(cx, |input, cx| input.set_text(&origin, cx));
                }
            }
            Err(e) => self.url_error = Some(e),
        }
        cx.notify();
    }

    pub(super) fn render_workman(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let status = cx.theme().status().clone();
        let colors = cx.theme().colors().clone();
        let field = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().id("ws-url").w(px(280.)).child(self.url_input.clone()))
            .child(button::action(&self.hits, "settings-ws-url-apply", "Apply", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.apply_url(cx))));
        let below = div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .children(self.url_error.clone().map(|e| div().id("ws-url-error").text_xs().text_color(status.error).child(SharedString::from(e))))
            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(match (&self.workman, self.workman.checked_url()) {
                (_, Some(url)) => format!("Now {url}."),
                (WorkmanStatus::NotSetUp { .. }, None) => "None: run /workman:setup, or apply a URL here.".to_string(),
                (_, None) => "Checking…".to_string(),
            })));
        let warnings = self.server_check.iter().flat_map(|c| c.warnings.iter()).enumerate().map(|(ix, w)| {
            banner::tinted(status.warning).id(("ws-server-warning", ix)).mt_2().text_xs().child(SharedString::from(w.clone()))
        });
        let below = below.children(warnings);
        let mcp_name = match &self.server_check {
            None => "checking…".to_string(),
            Some(ServerCheck { mcp_name: Some(name), .. }) => name.clone(),
            Some(ServerCheck { mcp_name: None, .. }) => "none: no usable ~/.workman/server.json".to_string(),
        };
        vec![
            self.render_workman_status(cx),
            row(
                "Server URL",
                format!(
                    "The Workman server Supraman signs in to, as ~/.workman/server.json names it (http only for this \
                     machine). While signed in, a new server is used only once you have signed in there: Supraman then \
                     restarts its sessions with tokens from it, asks you to restart open claude tabs, and signs out of \
                     the old one."
                ),
                field.into_any_element(),
                Some(below.into_any_element()),
                cx,
            ),
            row(
                "MCP server name",
                "The name ~/.workman/server.json gives the Workman MCP server, which /workman:setup registers in                  claude's user config. Supraman's sessions are given it as workman; Apply writes workman.",
                div().id("ws-mcp-name").text_sm().child(SharedString::from(mcp_name)).into_any_element(),
                None,
                cx,
            ),
        ]
    }

    pub(super) fn render_workman_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = cx.theme().status().clone();
        let (signing_in, sign_in_error) = self
            .fleet
            .upgrade()
            .map(|f| {
                let f = f.read(cx);
                (f.workman_state.signing_in.clone(), f.workman_state.sign_in_error.clone())
            })
            .unwrap_or_default();
        let (text, color) = self.status_words(cx);
        let controls = self.sign_in_controls(signing_in.is_some(), cx);
        row(
            "Status",
            format!(
                "Supraman signs in to Workman in your browser (OAuth). The grant is kept in the Supraman home, \
                 encrypted for your Windows account; Supraman uses it to read Workman and to give each session it \
                 starts a token of its own. Sign out revokes the grant, and with it every session token minted \
                 under it. Checked every {} s and after every change here.",
                RECHECK_EVERY.as_secs()
            ),
            controls.into_any_element(),
            Some(
                div()
                    .id("ws-status")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pt_2()
                    .child(div().text_sm().text_color(color).child(SharedString::from(text)))
                    .children(signing_in.map(|to| {
                        div().text_xs().child(SharedString::from(format!("Waiting for the sign-in to {to} in your browser…")))
                    }))
                    .children(sign_in_error.map(|e| div().text_xs().text_color(status.error).child(SharedString::from(e))))
                    .children(
                        matches!(self.workman, WorkmanStatus::Unreachable { .. })
                            .then(|| self.workman.fix())
                            .flatten()
                            .map(|fix| div().text_xs().child(SharedString::from(fix))),
                    )
                    .into_any_element(),
            ),
            cx,
        )
    }

    fn status_words(&self, cx: &App) -> (String, Hsla) {
        let status = cx.theme().status();
        match (&self.workman, &self.auth) {
            (WorkmanStatus::Checking, _) => ("checking…".to_string(), cx.theme().colors().text_muted),
            (WorkmanStatus::Ready { url, .. }, Some(AuthStatus::SignedIn { unsaved: Some(why), .. })) => {
                (format!("Signed in on {url}, but {why}."), status.warning)
            }
            (WorkmanStatus::Ready { url, who, .. }, _) => (
                match who {
                    Some(who) => format!("Signed in on {url} as {who}."),
                    None => format!("Signed in on {url}."),
                },
                status.success,
            ),
            (WorkmanStatus::NotSignedIn { .. }, _) => ("Not signed in.".to_string(), status.warning),
            (WorkmanStatus::SignInAgain { why, .. }, _) => (format!("Sign in again: {why}."), status.warning),
            (WorkmanStatus::Unreachable { url, error }, Some(AuthStatus::SignedIn { .. })) => {
                (format!("Signed in on {url}, but it is unreachable ({error})."), status.error)
            }
            (ws, _) => (ws.problem().unwrap_or_default(), status.error),
        }
    }

    fn sign_in_controls(&self, signing_in: bool, cx: &mut Context<Self>) -> Div {
        let held = self.auth.as_ref().is_some_and(|a| !matches!(a, AuthStatus::NotSignedIn));
        let mut controls = div().flex().flex_col().items_end().gap_1();
        controls = match signing_in {
            true => controls.child(button::action(&self.hits, "settings-ws-cancel", "Cancel sign-in", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.fleet.update(cx, |fleet, _| fleet.cancel_sign_in()).ok();
            }))),
            false if self.auth.is_some() && (!held || matches!(self.workman, WorkmanStatus::SignInAgain { .. })) => {
                controls.child(button::action(&self.hits, "settings-ws-signin", "Sign in", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.fleet.update(cx, |fleet, cx| fleet.sign_in(cx)).ok();
                    cx.notify();
                })))
            }
            false => controls,
        };
        if held && !signing_in {
            controls = controls.child(button::action(&self.hits, "settings-ws-signout", "Sign out", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.fleet.update(cx, |fleet, cx| fleet.sign_out(cx)).ok();
                cx.notify();
            })));
        }
        controls.child(button::action(&self.hits, "settings-ws-check", "Check now", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.fleet.update(cx, |fleet, _| fleet.recheck_workman()).ok();
            this.check_server(cx);
        })))
    }

    #[cfg(feature = "selftest")]
    pub fn workman(&self) -> &WorkmanStatus {
        &self.workman
    }
}

//! The remote calls' prompts in the tab's band: a branch's upstream (a remote and the branch there), a remote's
//! URL, and ssh's prompt (a passphrase with Remember, a password, an unknown host key's yes or no). Hit keys:
//! `git-upstream-<remote>`, `git-upstream-ok`, `git-upstream-cancel`, `git-edit-remote-ok`,
//! `git-edit-remote-cancel`, `git-askpass-ok`, `git-askpass-remember`, `git-askpass-cancel`, each band
//! `<key>-band` and its field `<key>-field`.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Div, Entity, Subscription, Window, div, prelude::*, px};
use supraman_core::askpass::{Asked, Prompt, Reply};
use supraman_gitman::write::remote::PullMode;
use supraman_gitman::{Remote, mask_userinfo};
use supraman_ui::button;
use supraman_ui::confirm::band;
use supraman_ui::hit::hit_target;
use supraman_ui::segmented::{Segment, segmented};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::text_input::{InputEvent, TextInput};
use theme::ActiveTheme as _;

use super::RemoteActions;

/// What follows a chosen upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Then {
    Push,
    Pull(PullMode),
    Set,
}

pub(crate) struct UpstreamDialog {
    pub(crate) local: String,
    pub(crate) then: Then,
    /// The remote chosen, by name.
    pub(crate) remote: String,
    field: Option<(Entity<TextInput>, Subscription)>,
}

impl UpstreamDialog {
    /// `origin` first when there is one, else the first remote.
    pub(crate) fn new(local: String, then: Then, remotes: &[Remote]) -> UpstreamDialog {
        let remote = remotes.iter().find(|r| r.name == "origin").or(remotes.first()).map(|r| r.name.clone()).unwrap_or_default();
        UpstreamDialog { local, then, remote, field: None }
    }

    fn branch(&self, cx: &gpui::App) -> String {
        self.field.as_ref().map(|(f, _)| f.read(cx).text().trim().to_string()).unwrap_or_default()
    }

    /// `(local, remote, branch, then)` once the branch has a name and the remote is one of `remotes`.
    pub(crate) fn chosen(&self, remotes: &[Remote], cx: &gpui::App) -> Option<(String, String, String, Then)> {
        let branch = self.branch(cx);
        (!branch.is_empty() && remotes.iter().any(|r| r.name == self.remote)).then(|| (self.local.clone(), self.remote.clone(), branch, self.then))
    }

    #[cfg(test)]
    pub(crate) fn type_in(&self, branch: &str, cx: &mut gpui::App) {
        if let Some((f, _)) = &self.field {
            f.update(cx, |i, cx| i.set_text(branch, cx));
        }
    }
}

pub(crate) struct EditRemote {
    pub(crate) name: String,
    pub(crate) was: String,
    field: Option<(Entity<TextInput>, Subscription)>,
}

impl EditRemote {
    pub(crate) fn new(name: String, was: String) -> EditRemote {
        EditRemote { name, was, field: None }
    }

    pub(crate) fn url(&self, cx: &gpui::App) -> String {
        self.field.as_ref().map(|(f, _)| f.read(cx).text().trim().to_string()).unwrap_or_default()
    }
}

/// ssh's prompt waiting for the user's answer.
pub(crate) struct Prompting {
    asked: Option<Asked>,
    prompt: Prompt,
    remember: bool,
    field: Option<(Entity<TextInput>, Subscription)>,
}

impl Prompting {
    pub(crate) fn new(asked: Asked) -> Prompting {
        let prompt = asked.prompt.clone();
        Prompting { asked: Some(asked), prompt, remember: false, field: None }
    }

    /// No answer: ssh fails and says so.
    pub(crate) fn cancel(mut self) {
        if let Some(asked) = self.asked.take() {
            asked.answer(None);
        }
    }

    fn reply(&mut self, text: String) {
        if let Some(asked) = self.asked.take() {
            asked.answer(Some(Reply { text, remember: self.remember }));
        }
    }

    fn words(&self) -> String {
        match &self.prompt {
            Prompt::Passphrase { key: Some(key) } => format!("The passphrase of {key}"),
            Prompt::Passphrase { key: None } => "The ssh key's passphrase".into(),
            Prompt::Password { who } => format!("The ssh password of {who}"),
            Prompt::HostKey { text } => text.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn type_in(&self, text: &str, cx: &mut gpui::App) {
        if let Some((f, _)) = &self.field {
            f.update(cx, |i, cx| i.set_text(text, cx));
        }
    }
}

impl Drop for Prompting {
    fn drop(&mut self) {
        if let Some(asked) = self.asked.take() {
            asked.answer(None);
        }
    }
}

impl RemoteActions {
    /// The dialogs' fields, made once a window is at hand, each taking the keyboard.
    pub(crate) fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = |text: &str, placeholder: &str, masked: bool, on_confirm: fn(&mut RemoteActions, &mut Context<RemoteActions>), window: &mut Window, cx: &mut Context<RemoteActions>| {
            let input = cx.new(|cx| {
                let input = TextInput::new(placeholder.to_string(), None, window, cx);
                if masked { input.masked() } else { input }
            });
            input.update(cx, |i, cx| {
                i.set_text(text, cx);
                i.focus_all(window, cx);
            });
            let sub = cx.subscribe(&input, move |this: &mut RemoteActions, _, e: &InputEvent, cx| match e {
                InputEvent::Confirm => on_confirm(this, cx),
                InputEvent::Cancel => this.close_dialogs(cx),
                _ => cx.notify(),
            });
            (input, sub)
        };
        if let Some(d) = self.upstream.as_ref().filter(|d| d.field.is_none()) {
            let local = d.local.clone();
            let made = field(&local, "Branch on the remote", false, |this, cx| this.submit_upstream(cx), window, cx);
            self.upstream.as_mut().map(|d| d.field = Some(made));
        }
        if let Some(e) = self.editing.as_ref().filter(|e| e.field.is_none()) {
            let shown = mask_userinfo(&e.was);
            let made = field(&shown, "URL", false, |this, cx| this.submit_edit(cx), window, cx);
            self.editing.as_mut().map(|e| e.field = Some(made));
        }
        if self.prompting.as_ref().is_some_and(|p| p.field.is_none() && !matches!(p.prompt, Prompt::HostKey { .. })) {
            let made = field("", "Passphrase", true, |this, cx| this.answer_prompt(true, cx), window, cx);
            self.prompting.as_mut().map(|p| p.field = Some(made));
        }
    }

    fn close_dialogs(&mut self, cx: &mut Context<Self>) {
        self.upstream = None;
        self.editing = None;
        if let Some(p) = self.prompting.take() {
            p.cancel();
        }
        cx.notify();
    }

    /// The prompt's answer: OK sends the field's text (a host key's yes), Cancel nothing.
    pub(crate) fn answer_prompt(&mut self, ok: bool, cx: &mut Context<Self>) {
        let Some(mut p) = self.prompting.take() else { return };
        if ok {
            let text = match &p.prompt {
                Prompt::HostKey { .. } => "yes".to_string(),
                _ => p.field.as_ref().map(|(f, _)| f.read(cx).text().to_string()).unwrap_or_default(),
            };
            p.reply(text);
        } else if matches!(p.prompt, Prompt::HostKey { .. }) {
            p.reply("no".into());
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn prompting(&self) -> Option<&Prompting> {
        self.prompting.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn upstream_dialog(&self) -> Option<&UpstreamDialog> {
        self.upstream.as_ref()
    }

    /// The question, else the upstream, the URL or ssh's prompt, as the tab's band; `true` when a field of it
    /// holds the keyboard.
    pub(crate) fn band(&self, cx: &mut Context<Self>) -> Option<(Div, bool)> {
        if let Some(band) = self.writes.band_answer(&self.hits, "git-remote-ask", Self::answer_force, cx) {
            return Some((band, false));
        }
        self.render_upstream(cx).or_else(|| self.render_edit(cx)).or_else(|| self.render_prompt(cx))
    }

    fn cancel_button(&self, key: &str, words: &str, cx: &mut Context<Self>) -> AnyElement {
        button::tinted(&self.hits, key.to_string(), words.to_string(), None, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_dialogs(cx))).into_any_element()
    }

    fn field(&self, field: &Entity<TextInput>, key: &str, width: f32) -> AnyElement {
        div().relative().w(px(width)).child(field.clone()).child(hit_target(&self.hits, key.to_string())).into_any_element()
    }

    fn render_upstream(&self, cx: &mut Context<Self>) -> Option<(Div, bool)> {
        let d = self.upstream.as_ref()?;
        let (field, _) = d.field.as_ref()?;
        let segments: Vec<Segment> = self.remotes.iter().map(|r| Segment::new(r.name.clone()).label(r.name.clone())).collect();
        let chosen = self.remotes.iter().position(|r| r.name == d.remote).unwrap_or(0);
        let remotes = segmented(&self.hits, "git-upstream", &segments, chosen, |this: &mut RemoteActions, i, _, _| {
            if let (Some(d), Some(r)) = (this.upstream.as_mut(), this.remotes.get(i)) {
                d.remote = r.name.clone();
            }
        }, cx);
        let (ok_words, ask) = match d.then {
            Then::Push => ("Push", "Push {} with no upstream to"),
            Then::Pull(_) => ("Pull", "Pull {} with no upstream from"),
            Then::Set => ("Set upstream", "Track {} at"),
        };
        let ok = button::tinted(&self.hits, "git-upstream-ok", ok_words, Some(cx.theme().status().success), cx);
        let ok = if d.chosen(&self.remotes, cx).is_some() { ok.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_upstream(cx))) } else { button::dimmed(ok) };
        let items = vec![
            remotes.into_any_element(),
            div().text_color(cx.theme().colors().text_muted).child("/").into_any_element(),
            self.field(field, "git-upstream-field", FIELD),
            ok.into_any_element(),
            self.cancel_button("git-upstream-cancel", "Cancel", cx),
        ];
        Some((band(&self.hits, "git-upstream", ask.replace("{}", &d.local), items, cx), true))
    }

    fn render_edit(&self, cx: &mut Context<Self>) -> Option<(Div, bool)> {
        let e = self.editing.as_ref()?;
        let (field, _) = e.field.as_ref()?;
        let ok = button::tinted(&self.hits, "git-edit-remote-ok", "Save", Some(cx.theme().status().success), cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_edit(cx)));
        let items = vec![self.field(field, "git-edit-remote-field", 2. * FIELD), ok.into_any_element(), self.cancel_button("git-edit-remote-cancel", "Cancel", cx)];
        Some((band(&self.hits, "git-edit-remote", format!("The URL {} fetches from and pushes to", e.name), items, cx), true))
    }

    fn render_prompt(&self, cx: &mut Context<Self>) -> Option<(Div, bool)> {
        let p = self.prompting.as_ref()?;
        let host = matches!(p.prompt, Prompt::HostKey { .. });
        let ok = button::tinted(&self.hits, "git-askpass-ok", if host { "Yes, trust it" } else { "OK" }, Some(cx.theme().status().success), cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.answer_prompt(true, cx)));
        let cancel = button::tinted(&self.hits, "git-askpass-cancel", if host { "No" } else { "Cancel" }, None, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.answer_prompt(false, cx)));
        let remember = matches!(p.prompt, Prompt::Passphrase { key: Some(_) }).then(|| {
            let tick = button::checkbox(&self.hits, "git-askpass-remember", p.remember, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(p) = this.prompting.as_mut() {
                    p.remember = !p.remember;
                }
                cx.notify();
            }));
            div().flex().items_center().gap_2().text_xs().child(tick).child(div().id("git-askpass-remember-words").child("Remember").tooltip(Tooltip::text("Keep it in Supraman's secret store"))).into_any_element()
        });
        let mut items: Vec<AnyElement> = p.field.as_ref().map(|(field, _)| self.field(field, "git-askpass-field", FIELD)).into_iter().collect();
        items.extend(remember);
        items.extend([ok.into_any_element(), cancel.into_any_element()]);
        let message = if host { format!("ssh does not know this host. {}", p.words()) } else { p.words() };
        Some((band(&self.hits, "git-askpass", message, items, cx), p.field.is_some()))
    }
}

/// A band's text field's width, GitKraken's select's.
const FIELD: f32 = 160.;

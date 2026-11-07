//! The remote calls' dialogs over the tab: a branch's upstream (a remote and the branch there), a remote's URL,
//! and ssh's prompt (a passphrase with Remember, a password, an unknown host key's yes or no). Hit keys:
//! `git-upstream-ok`, `git-upstream-cancel`, `git-edit-remote-ok`, `git-edit-remote-cancel`,
//! `git-askpass-ok`, `git-askpass-remember`, `git-askpass-cancel`.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, Subscription, Window, div, prelude::*, px};
use supraman_core::askpass::{Asked, Prompt, Reply};
use supraman_gitman::write::remote::PullMode;
use supraman_gitman::{Remote, mask_userinfo};
use supraman_ui::button;
use supraman_ui::text_input::{InputEvent, TextInput};
use theme::ActiveTheme as _;

use super::RemoteActions;

/// What follows a chosen upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    Push,
    Pull(PullMode),
    Set,
}

pub struct UpstreamDialog {
    pub local: String,
    pub then: Then,
    /// The remote chosen, by name.
    pub remote: String,
    field: Option<(Entity<TextInput>, Subscription)>,
}

impl UpstreamDialog {
    /// `origin` first when there is one, else the first remote.
    pub fn new(local: String, then: Then, remotes: &[Remote]) -> UpstreamDialog {
        let remote = remotes.iter().find(|r| r.name == "origin").or(remotes.first()).map(|r| r.name.clone()).unwrap_or_default();
        UpstreamDialog { local, then, remote, field: None }
    }

    fn branch(&self, cx: &gpui::App) -> String {
        self.field.as_ref().map(|(f, _)| f.read(cx).text().trim().to_string()).unwrap_or_default()
    }

    /// `(local, remote, branch, then)` once the branch has a name and the remote is one of `remotes`.
    pub fn chosen(&self, remotes: &[Remote], cx: &gpui::App) -> Option<(String, String, String, Then)> {
        let branch = self.branch(cx);
        (!branch.is_empty() && remotes.iter().any(|r| r.name == self.remote)).then(|| (self.local.clone(), self.remote.clone(), branch, self.then))
    }

    #[cfg(test)]
    pub fn type_in(&self, branch: &str, cx: &mut gpui::App) {
        if let Some((f, _)) = &self.field {
            f.update(cx, |i, cx| i.set_text(branch, cx));
        }
    }
}

pub struct EditRemote {
    pub name: String,
    pub was: String,
    field: Option<(Entity<TextInput>, Subscription)>,
}

impl EditRemote {
    pub fn new(name: String, was: String) -> EditRemote {
        EditRemote { name, was, field: None }
    }

    pub fn url(&self, cx: &gpui::App) -> String {
        self.field.as_ref().map(|(f, _)| f.read(cx).text().trim().to_string()).unwrap_or_default()
    }
}

/// ssh's prompt waiting for the user's answer.
pub struct Prompting {
    asked: Option<Asked>,
    prompt: Prompt,
    remember: bool,
    field: Option<(Entity<TextInput>, Subscription)>,
}

impl Prompting {
    pub fn new(asked: Asked) -> Prompting {
        let prompt = asked.prompt.clone();
        Prompting { asked: Some(asked), prompt, remember: false, field: None }
    }

    /// No answer: ssh fails and says so.
    pub fn cancel(mut self) {
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
            Prompt::Passphrase { key: Some(key) } => format!("The passphrase of the ssh key {key}"),
            Prompt::Passphrase { key: None } => "The ssh key's passphrase".into(),
            Prompt::Password { who } => format!("The ssh password of {who}"),
            Prompt::HostKey { text } => text.clone(),
        }
    }

    #[cfg(test)]
    pub fn type_in(&self, text: &str, cx: &mut gpui::App) {
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

fn card(title: impl Into<String>, cx: &gpui::App) -> gpui::Div {
    let colors = cx.theme().colors();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .m_2()
        .p_2()
        .max_w(px(520.))
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.elevated_surface_background)
        .child(div().text_xs().text_color(colors.text_muted).child(title.into()))
}

impl RemoteActions {
    /// The dialogs' fields, made once a window is at hand, each taking the keyboard.
    pub(super) fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub fn answer_prompt(&mut self, ok: bool, cx: &mut Context<Self>) {
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
    pub fn prompting(&self) -> Option<&Prompting> {
        self.prompting.as_ref()
    }

    #[cfg(test)]
    pub fn upstream_dialog(&self) -> Option<&UpstreamDialog> {
        self.upstream.as_ref()
    }

    pub(super) fn render_dialogs(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        out.extend(self.render_upstream(cx));
        if let Some(e) = &self.editing
            && let Some((field, _)) = &e.field
        {
            let ok = button::bordered(&self.hits, "git-edit-remote-ok", "Save", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_edit(cx)));
            let cancel = button::bordered(&self.hits, "git-edit-remote-cancel", "Cancel", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_dialogs(cx)));
            out.push(card(format!("The URL {} fetches from and pushes to", e.name), cx).child(field.clone()).child(div().flex().justify_end().gap_2().child(ok).child(cancel)).into_any_element());
        }
        out.extend(self.render_prompt(cx));
        out
    }

    fn render_upstream(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let d = self.upstream.as_ref()?;
        let (field, _) = d.field.as_ref()?;
        let choices = self.remotes.iter().map(|r| {
            let name = r.name.clone();
            let key = format!("git-upstream-{name}");
            button::choice(gpui::ElementId::Name(key.clone().into()), format!("{name}  {}", mask_userinfo(&r.url)), r.name == d.remote, cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(d) = this.upstream.as_mut() {
                        d.remote = name.clone();
                    }
                    cx.notify();
                }))
        });
        let ok_words = match d.then {
            Then::Push => "Push",
            Then::Pull(_) => "Pull",
            Then::Set => "Set upstream",
        };
        let ok = button::bordered(&self.hits, "git-upstream-ok", ok_words, cx);
        let ok = if d.chosen(&self.remotes, cx).is_some() { ok.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_upstream(cx))) } else { button::dimmed(ok) };
        let cancel = button::bordered(&self.hits, "git-upstream-cancel", "Cancel", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_dialogs(cx)));
        let title = format!("{} has no upstream: choose the remote and its branch there", d.local);
        Some(card(title, cx).child(div().flex().flex_col().gap_1().children(choices)).child(field.clone()).child(div().flex().justify_end().gap_2().child(ok).child(cancel)).into_any_element())
    }

    fn render_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.prompting.as_ref()?;
        let host = matches!(p.prompt, Prompt::HostKey { .. });
        let ok = button::bordered(&self.hits, "git-askpass-ok", if host { "Yes, trust it" } else { "OK" }, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.answer_prompt(true, cx)));
        let cancel = button::bordered(&self.hits, "git-askpass-cancel", if host { "No" } else { "Cancel" }, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.answer_prompt(false, cx)));
        let remember = matches!(p.prompt, Prompt::Passphrase { key: Some(_) }).then(|| {
            let tick = button::checkbox(&self.hits, "git-askpass-remember", p.remember, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(p) = this.prompting.as_mut() {
                    p.remember = !p.remember;
                }
                cx.notify();
            }));
            div().flex().items_center().gap_2().child(tick).child("Remember it in Supraman's secret store")
        });
        let mut body = card(if host { "ssh does not know this host".to_string() } else { "ssh asks".to_string() }, cx).child(div().whitespace_normal().child(p.words()));
        if let Some((field, _)) = &p.field {
            body = body.child(field.clone());
        }
        Some(body.children(remember).child(div().flex().justify_end().gap_2().child(ok).child(cancel)).into_any_element())
    }
}

//! The remote calls' prompts in the tab's prompt: a branch's upstream and ssh's prompt.

use std::rc::Rc;

use gpui::{Focusable as _, AnyElement, AppContext as _, ClickEvent, Context, Entity, FocusHandle, SharedString, Subscription, Window, div, prelude::*};
use supraman_core::askpass::{Asked, Prompt, Reply};
use supraman_gitman::write::remote::PullMode;
use supraman_gitman::Remote;
use supraman_ui::button;
use supraman_ui::dialog::{Button as DialogButton, Panel};
use supraman_ui::focus_order::Press;
use supraman_ui::form;
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

/// ssh's prompt waiting for the user's answer.
pub(crate) struct Prompting {
    asked: Option<Asked>,
    prompt: Prompt,
    remember: bool,
    field: Option<(Entity<TextInput>, Subscription)>,
    /// Remember's tick box, for a key's passphrase.
    tick: Option<FocusHandle>,
}

impl Prompting {
    pub(crate) fn new(asked: Asked) -> Prompting {
        let prompt = asked.prompt.clone();
        Prompting { asked: Some(asked), prompt, remember: false, field: None, tick: None }
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
        self.ensure_add_fields(window, cx);
        let field = |text: &str, placeholder: &str, masked: bool, on_confirm: fn(&mut RemoteActions, &mut Context<RemoteActions>), window: &mut Window, cx: &mut Context<RemoteActions>| {
            let input = cx.new(|cx| {
                let input = TextInput::new(placeholder.to_string(), None, window, cx);
                if masked { input.masked() } else { input }
            });
            input.update(cx, |i, cx| {
                i.set_text(text, cx);
                i.select_all_text(cx);
            });
            supraman_ui::focus::request(&input.focus_handle(cx), window, cx);
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
        if self.prompting.as_ref().is_some_and(|p| p.field.is_none() && !matches!(p.prompt, Prompt::HostKey { .. })) {
            let made = field("", "Passphrase", true, |this, cx| this.answer_prompt(true, cx), window, cx);
            self.prompting.as_mut().map(|p| p.field = Some(made));
        }
        if let Some(p) = self.prompting.as_mut().filter(|p| p.tick.is_none() && matches!(p.prompt, Prompt::Passphrase { key: Some(_) })) {
            p.tick = Some(cx.focus_handle());
        }
    }

    fn close_dialogs(&mut self, cx: &mut Context<Self>) {
        self.upstream = None;
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

    /// The question, else the upstream or ssh's prompt, else Add or Edit Remote, in the tab's prompt.
    pub(crate) fn prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.ensure_fields(window, cx);
        self.writes.prompt_answer(&self.hits, "git-remote-ask", Self::answer_force, cx).or_else(|| self.render_upstream(cx)).or_else(|| self.render_prompt(cx)).or_else(|| self.render_add(cx))
    }

    /// The upstream to push to, pull from or track: a remote picked, then the branch on it.
    fn render_upstream(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
        let why = d.chosen(&self.remotes, cx).is_none().then_some(NEEDS_BRANCH);
        let ok = DialogButton::new("git-upstream-ok", ok_words, button::Tone::Neutral, |this: &mut Self, _, cx| this.submit_upstream(cx)).dimmed(why);
        let cancel = DialogButton::new("git-upstream-cancel", "Cancel", button::Tone::Neutral, |this: &mut Self, _, cx| this.close_dialogs(cx));
        let muted = supraman_ui::themes::secondary(cx.theme().colors());
        let line = div().flex().items_center().gap_2().child(remotes).child(div().text_color(muted).child("/")).child(form::input(&self.hits, "git-upstream-field", field));
        let text = div().relative().child(ask.replace("{}", &d.local)).child(hit_target(&self.hits, "git-upstream-text".into()));
        let panel = Panel {
            key: "git-upstream".into(),
            title: Some(ok_words.into()),
            body: div().flex().flex_col().gap_3().child(text).child(line).into_any_element(),
            fields: vec![field.focus_handle(cx)],
            buttons: vec![ok, cancel],
            first: None,
            default: Some("git-upstream-ok".into()),
            close: None,
            cancel: Rc::new(|this: &mut Self, _, cx| this.close_dialogs(cx)),
        };
        Some(self.writes.dialog().render(&self.hits, panel, cx))
    }

    /// ssh's prompt: its words, its field and Remember for a key's passphrase; an unknown host's No is the default.
    fn render_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.prompting.as_ref()?;
        let host = matches!(p.prompt, Prompt::HostKey { .. });
        let ok = DialogButton::new("git-askpass-ok", if host { "Yes, trust it" } else { "OK" }, button::Tone::Neutral, |this: &mut Self, _, cx| this.answer_prompt(true, cx));
        let cancel = DialogButton::new("git-askpass-cancel", if host { "No" } else { "Cancel" }, button::Tone::Neutral, |this: &mut Self, _, cx| this.answer_prompt(false, cx));
        let remember = p.tick.as_ref().map(|tick| {
            let square = button::focusable_checkbox(&self.hits, "git-askpass-remember", p.remember, tick, cx.listener(|this, _: &Press, _, cx| this.toggle_remember(cx)), cx);
            let words = div().id("git-askpass-remember-words").child("Remember").tooltip(Tooltip::text("Keep it in Supraman's secret store"));
            let ring = supraman_ui::focus::ring(tick, supraman_ui::tokens::radius::S);
            div().id("git-askpass-remember-row").relative().flex().items_center().gap_2().cursor_pointer().child(square).child(words).child(ring).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_remember(cx)))
        });
        let message = if host { format!("ssh does not know this host. {}", p.words()) } else { p.words() };
        let text = div().relative().child(message).child(hit_target(&self.hits, "git-askpass-text".into()));
        let body = div().flex().flex_col().gap_3().child(text).children(p.field.as_ref().map(|(f, _)| form::input(&self.hits, "git-askpass-field", f))).children(remember);
        let fields = p.field.as_ref().map(|(f, _)| f.focus_handle(cx)).into_iter().chain(p.tick.clone()).collect();
        let default = SharedString::from(if host { "git-askpass-cancel" } else { "git-askpass-ok" });
        let panel = Panel {
            key: "git-askpass".into(),
            title: Some(if host { "Unknown host" } else { "ssh" }.into()),
            body: body.into_any_element(),
            fields,
            buttons: vec![ok, cancel],
            first: host.then(|| default.clone()),
            default: Some(default),
            close: None,
            cancel: Rc::new(|this: &mut Self, _, cx| this.answer_prompt(false, cx)),
        };
        Some(self.writes.dialog().render(&self.hits, panel, cx))
    }

    fn toggle_remember(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.prompting.as_mut() {
            p.remember = !p.remember;
        }
        cx.notify();
    }
}

/// Why the upstream's OK cannot act: no branch typed on the remote.
const NEEDS_BRANCH: &str = "Type the branch on the remote";

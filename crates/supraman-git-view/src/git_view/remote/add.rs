//! Add Remote and Edit Remote beside the ref panel: Name, Pull URL and an optional Push URL;
//! hit keys `git-add-remote{,-close,-ok,-name,-pull,-push}`, Edit's `git-edit-remote…`.

use gpui::{AnyElement, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, Window};
use supraman_gitman::write::LockSet;
use supraman_gitman::write::remote;
use supraman_gitman::{Remote, mask_userinfo};
use supraman_ui::focus_order::FocusOrder;
use supraman_ui::form::{self, Answers, Form};
use supraman_ui::icon::Icon;
use supraman_ui::text_input::TextInput;

use super::{Doing, Done, RemoteActions};

pub(crate) const KEY: &str = "git-add-remote";
pub(crate) const EDIT_KEY: &str = "git-edit-remote";
pub(crate) const NEEDS_NAME: &str = "Type the remote's name";
pub(crate) const NEEDS_URL: &str = "Type the URL it pulls from";

const FIELDS: [(&str, &str, &str); 3] = [("name", "Name", "origin"), ("pull", "Pull URL", "https://github.com/owner/repo.git"), ("push", "Push URL (optional)", "The pull URL")];

const ANSWERS: Answers<RemoteActions> = Answers { ok: RemoteActions::submit_add, close: RemoteActions::close_add };

#[derive(Default)]
pub(crate) struct AddRemote {
    /// The remote Edit Remote changes; `None` adds one.
    editing: Option<Remote>,
    fields: Vec<(Entity<TextInput>, Subscription)>,
    ok: Option<FocusHandle>,
}

impl AddRemote {
    fn text(&self, at: usize, cx: &gpui::App) -> String {
        self.fields.get(at).map(|(f, _)| f.read(cx).text().trim().to_string()).unwrap_or_default()
    }

    fn key(&self) -> &'static str {
        if self.editing.is_some() { EDIT_KEY } else { KEY }
    }

    /// Name, Pull URL, Push URL, then OK.
    fn order(&self, cx: &gpui::App) -> FocusOrder {
        FocusOrder::new(self.fields.iter().map(|(f, _)| f.focus_handle(cx)).chain(self.ok.clone()))
    }

    #[cfg(test)]
    pub(crate) fn focused(&self, window: &Window, cx: &gpui::App) -> Option<usize> {
        self.order(cx).focused(window, cx)
    }

    /// `(name, pull, push)` as typed.
    pub(crate) fn typed(&self, cx: &gpui::App) -> (String, String, Option<String>) {
        (self.text(0, cx), self.text(1, cx), Some(self.text(2, cx)).filter(|p| !p.is_empty()))
    }

    /// Why OK cannot act on what is typed.
    pub(crate) fn why_not(&self, cx: &gpui::App) -> Option<&'static str> {
        let (name, pull, _) = self.typed(cx);
        if name.is_empty() {
            Some(NEEDS_NAME)
        } else if pull.is_empty() {
            Some(NEEDS_URL)
        } else {
            None
        }
    }

    /// The fields as Edit Remote first shows them: its name, URL and push URL, userinfo masked.
    fn shown(remote: Option<&Remote>) -> [String; 3] {
        remote.map_or_else(Default::default, |r| [r.name.clone(), mask_userinfo(&r.url), r.push_url.as_deref().map(mask_userinfo).unwrap_or_default()])
    }

    /// Edit Remote's changes: a new name, URL and push URL (`Some(None)` to drop it), each `None` when unchanged.
    #[allow(clippy::type_complexity)]
    pub(crate) fn changes(&self, cx: &gpui::App) -> Option<(String, Option<String>, Option<String>, Option<Option<String>>)> {
        let was = self.editing.as_ref()?;
        let [name, url, push] = Self::shown(Some(was));
        let (typed_name, typed_url, typed_push) = self.typed(cx);
        let renamed = Some(typed_name).filter(|n| *n != name);
        let url = Some(typed_url).filter(|u| *u != url);
        let push = (typed_push.clone().unwrap_or_default() != push).then_some(typed_push);
        Some((was.name.clone(), renamed, url, push))
    }

    #[cfg(test)]
    pub(crate) fn type_in(&self, at: usize, text: &str, cx: &mut gpui::App) {
        if let Some((f, _)) = self.fields.get(at) {
            f.update(cx, |i, cx| i.set_text(text, cx));
        }
    }

    #[cfg(test)]
    pub(crate) fn texts(&self, cx: &gpui::App) -> [String; 3] {
        [0, 1, 2].map(|at| self.text(at, cx))
    }
}

impl RemoteActions {
    pub(crate) fn open_add(&mut self, cx: &mut Context<Self>) {
        if self.adding.as_ref().is_none_or(|a| a.editing.is_some()) {
            self.adding = Some(AddRemote::default());
        }
        cx.notify();
    }

    /// Edit Remote: the form filled in with `name`'s URLs.
    pub(super) fn edit_remote(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(r) = self.remotes.iter().find(|r| r.name == name) else { return };
        self.adding = Some(AddRemote { editing: Some(r.clone()), ..Default::default() });
        cx.emit(super::RemoteEvent::Opened);
        cx.notify();
    }

    pub(crate) fn close_add(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn adding(&self) -> Option<&AddRemote> {
        self.adding.as_ref()
    }

    /// The form's fields, made once a window is at hand, Name taking the keyboard.
    pub(super) fn ensure_add_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adding) = self.adding.as_ref().filter(|a| a.fields.is_empty()) else { return };
        let shown = AddRemote::shown(adding.editing.as_ref());
        let fields: Vec<_> = FIELDS.iter().zip(&shown).map(|((_, _, placeholder), text)| form::field(placeholder, text, ANSWERS, |_, cx| cx.notify(), window, cx)).collect();
        if let Some((name, _)) = fields.first() {
            name.update(cx, |i, cx| i.focus(window, cx));
        }
        if let Some(a) = self.adding.as_mut() {
            a.fields = fields;
            a.ok = Some(cx.focus_handle());
        }
    }

    /// Adds the remote and fetches it, or saves Edit Remote's changes; the form closes once they are written.
    pub(crate) fn submit_add(&mut self, cx: &mut Context<Self>) {
        let Some(adding) = &self.adding else { return };
        if let Some(why) = adding.why_not(cx).map(str::to_string).or_else(|| self.busy(cx)) {
            return cx.emit(super::RemoteEvent::Why(why));
        }
        if adding.editing.is_some() {
            return self.submit_edit(cx);
        }
        let (name, pull, push) = adding.typed(cx);
        let prune = self.prune;
        self.request(Doing::AddRemote(name.clone()), LockSet::remote_config(), move |s, t| {
            remote::add_remote(s, &name, &pull, push.as_deref())?;
            Ok(Done::Added(remote::fetch(s, &name, prune, t).err().map(|e| e.to_string())))
        }, cx);
    }

    /// The name, then the URL and the push URL under it, each only when changed; a refused name writes nothing.
    fn submit_edit(&mut self, cx: &mut Context<Self>) {
        let Some((was, renamed, url, push)) = self.adding.as_ref().and_then(|a| a.changes(cx)) else { return };
        if renamed.is_none() && url.is_none() && push.is_none() {
            return self.close_add(cx);
        }
        let name = renamed.clone().unwrap_or_else(|| was.clone());
        let words = format!("Saved remote {name}");
        self.request(Doing::EditRemote(words), LockSet::remote_config(), move |s, _| {
            if let Some(new) = &renamed {
                remote::rename_remote(s, &was, new)?;
            }
            if let Some(url) = &url {
                remote::set_url(s, &name, url)?;
            }
            if let Some(push) = &push {
                remote::set_push_url(s, &name, push.as_deref())?;
            }
            Ok(Done::Nothing)
        }, cx);
    }

    /// The remote is added: the form closed, the tab refreshed, and why its fetch failed said.
    pub(super) fn added(&mut self, name: &str, fetched: Result<(), String>, cx: &mut Context<Self>) {
        match fetched {
            Ok(()) => cx.emit(super::RemoteEvent::Wrote(Some(format!("Added remote {name}")))),
            Err(why) => {
                cx.emit(super::RemoteEvent::Wrote(None));
                cx.emit(super::RemoteEvent::Why(format!("Added remote {name}, but its fetch failed: {why}")));
            }
        }
        cx.notify();
    }

    /// The form as the tab's form slot shows it, while it is open.
    pub(crate) fn render_add(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let adding = self.adding.as_ref()?;
        let key = adding.key();
        let (title, ok) = if adding.editing.is_some() { ("Edit Remote", "Save") } else { ("Add Remote", "Add Remote") };
        let fields = FIELDS.iter().zip(&adding.fields).map(|((field, label, _), (input, _))| (SharedString::from(*label), form::input(&self.hits, format!("{key}-{field}"), input))).collect();
        let why = adding.why_not(cx).map(str::to_string).or_else(|| self.busy(cx));
        let form = Form { key: key.into(), icon: Icon::Remote, title: title.into(), ok: ok.into(), why: why.map(Into::into), running: false, answers: ANSWERS };
        Some(form.render(&self.hits, fields, adding.order(cx), adding.ok.as_ref(), cx))
    }
}

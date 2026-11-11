//! Add Remote in the commit panel's place: Name, Pull URL and an optional Push URL; hit keys `git-add-remote{,-close,-ok,-name,-pull,-push}`.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, Subscription, Window, div, prelude::*, px};
use supraman_gitman::write::remote;
use supraman_gitman::write::LockSet;
use supraman_ui::button;
use supraman_ui::focus_order::{Dismiss, FocusOrder, Press};
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::text_input::{InputEvent, TextInput};
use supraman_ui::themes::secondary;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::{Doing, Done, RemoteActions};

pub(crate) const KEY: &str = "git-add-remote";
pub(crate) const NEEDS_NAME: &str = "Type the remote's name";
pub(crate) const NEEDS_URL: &str = "Type the URL it pulls from";

const FIELDS: [(&str, &str, &str); 3] = [("name", "Name", "origin"), ("pull", "Pull URL", "https://github.com/owner/repo.git"), ("push", "Push URL (optional)", "The pull URL")];

#[derive(Default)]
pub(crate) struct AddRemote {
    fields: Vec<(Entity<TextInput>, Subscription)>,
    ok: Option<FocusHandle>,
}

impl AddRemote {
    fn text(&self, at: usize, cx: &gpui::App) -> String {
        self.fields.get(at).map(|(f, _)| f.read(cx).text().trim().to_string()).unwrap_or_default()
    }

    /// Name, Pull URL, Push URL, then Add Remote.
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

    /// Why Add Remote cannot act on what is typed.
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

    #[cfg(test)]
    pub(crate) fn type_in(&self, at: usize, text: &str, cx: &mut gpui::App) {
        if let Some((f, _)) = self.fields.get(at) {
            f.update(cx, |i, cx| i.set_text(text, cx));
        }
    }
}

impl RemoteActions {
    pub(crate) fn open_add(&mut self, cx: &mut Context<Self>) {
        if self.adding.is_none() {
            self.adding = Some(AddRemote::default());
        }
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
        if self.adding.as_ref().is_none_or(|a| !a.fields.is_empty()) {
            return;
        }
        let fields = FIELDS
            .iter()
            .enumerate()
            .map(|(at, (_, _, placeholder))| {
                let input = cx.new(|cx| TextInput::new(placeholder.to_string(), None, window, cx));
                if at == 0 {
                    input.update(cx, |i, cx| i.focus(window, cx));
                }
                let sub = cx.subscribe(&input, |this: &mut RemoteActions, _, e: &InputEvent, cx| match e {
                    InputEvent::Confirm => this.submit_add(cx),
                    InputEvent::Cancel => this.close_add(cx),
                    _ => cx.notify(),
                });
                (input, sub)
            })
            .collect();
        if let Some(a) = self.adding.as_mut() {
            a.fields = fields;
            a.ok = Some(cx.focus_handle());
        }
    }

    /// Adds the remote and fetches it; the form closes once the remote is added.
    pub(crate) fn submit_add(&mut self, cx: &mut Context<Self>) {
        let Some(adding) = &self.adding else { return };
        if let Some(why) = adding.why_not(cx).map(str::to_string).or_else(|| self.busy(cx)) {
            return cx.emit(super::RemoteEvent::Why(why));
        }
        let (name, pull, push) = adding.typed(cx);
        let prune = self.prune;
        self.request(Doing::AddRemote(name.clone()), LockSet::remote_config(), move |s, t| {
            remote::add_remote(s, &name, &pull, push.as_deref())?;
            Ok(Done::Added(remote::fetch(s, &name, prune, t).err().map(|e| e.to_string())))
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

    /// The form as the commit panel's place shows it, while it is open.
    pub(crate) fn render_add(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let adding = self.adding.as_ref()?;
        let colors = cx.theme().colors();
        let (muted, border) = (secondary(colors), colors.border);
        let close = button::icon(&self.hits, format!("{KEY}-close"), Icon::Close.svg(14., muted), px(22.), cx)
            .tooltip(Tooltip::text("Close (Esc)"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_add(cx)));
        let title = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(Icon::Remote.svg(16., muted))
            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Add Remote"))
            .child(close);
        let fields = FIELDS.iter().enumerate().map(|(at, (key, label, _))| {
            let field = adding.fields.get(at).map(|(f, _)| f.clone());
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(muted).child(*label))
                .child(div().relative().children(field).child(hit_target(&self.hits, format!("{KEY}-{key}"))))
        });
        let why = adding.why_not(cx).map(str::to_string).or_else(|| self.busy(cx));
        let ok = button::tinted(&self.hits, format!("{KEY}-ok"), "Add Remote", Some(cx.theme().status().success), cx);
        let ok = match why {
            Some(why) => button::dimmed(ok).tooltip(Tooltip::text(why)),
            None => ok.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_add(cx))),
        };
        let ok = match &adding.ok {
            Some(focus) => button::focusable(ok, focus, cx.listener(|this, _: &Press, _, cx| this.submit_add(cx)), cx),
            None => ok,
        };
        let body = div().flex().flex_col().gap_3().p_3().children(fields).child(div().flex().child(ok));
        let form = adding.order(cx).on(div()).on_action(cx.listener(|this, _: &Dismiss, _, cx| this.close_add(cx))).relative().size_full().flex().flex_col().bg(colors.panel_background).child(title).child(body).child(hit_target(&self.hits, KEY.to_string()));
        Some(form.into_any_element())
    }
}

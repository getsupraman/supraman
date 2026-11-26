//! The name of a new or renamed branch or tag, or an annotated tag's message: the tab's prompt, or a field in the row's cell.

use gpui::{Focusable as _, AnyElement, AppContext as _, ClickEvent, Context, Entity, FocusHandle, SharedString, Subscription, Window, div, prelude::*};
use supraman_gitman::write::branch::name_problem;
use supraman_ui::button;
use supraman_ui::dialog::{Button as DialogButton, Panel};
use supraman_ui::focus_order::Press;
use supraman_ui::form;
use supraman_ui::hit::hit_target;
use supraman_ui::text_input::{InputEvent, TextInput};
use theme::ActiveTheme as _;

use super::BranchActions;

/// What a name is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NameAsk {
    Branch { at: String },
    Tag { at: String },
    Annotated { at: String },
    /// The light tag `name` made again annotated: its message only.
    Annotate { name: String, at: String },
    Rename { from: String },
}

impl NameAsk {
    fn title(&self) -> String {
        match self {
            NameAsk::Branch { .. } => "New branch".into(),
            NameAsk::Tag { .. } => "New tag".into(),
            NameAsk::Annotated { .. } => "New annotated tag".into(),
            NameAsk::Annotate { name, .. } => format!("Annotate {name}"),
            NameAsk::Rename { from } => format!("Rename {from}"),
        }
    }

    fn has_name(&self) -> bool {
        !matches!(self, NameAsk::Annotate { .. })
    }

    fn has_message(&self) -> bool {
        matches!(self, NameAsk::Annotated { .. } | NameAsk::Annotate { .. })
    }
}

/// The fields once a window made them.
struct Fields {
    name: Entity<TextInput>,
    message: Option<Entity<TextInput>>,
    /// Check out after creating.
    tick: FocusHandle,
    _events: Vec<Subscription>,
}

pub(crate) struct NameDialog {
    pub(crate) ask: NameAsk,
    /// The row's commit when the field stands in the graph.
    pub(crate) inline: Option<String>,
    pub(crate) checkout: bool,
    fields: Option<Fields>,
}

/// What a submitted dialog asks for.
pub(crate) struct Named {
    pub(crate) ask: NameAsk,
    pub(crate) name: String,
    pub(crate) message: String,
    pub(crate) checkout: bool,
}

impl NameDialog {
    pub(crate) fn new(ask: NameAsk, inline: Option<String>) -> NameDialog {
        NameDialog { ask, inline, checkout: true, fields: None }
    }

    pub(crate) fn name(&self, cx: &gpui::App) -> String {
        self.fields.as_ref().map(|f| f.name.read(cx).text().trim().to_string()).unwrap_or_default()
    }

    fn message(&self, cx: &gpui::App) -> String {
        self.fields.as_ref().and_then(|f| f.message.as_ref()).map(|m| m.read(cx).text().trim().to_string()).unwrap_or_default()
    }

    /// Why it cannot be submitted yet.
    pub(crate) fn problem(&self, cx: &gpui::App) -> Option<&'static str> {
        if self.ask.has_name()
            && let Some(problem) = name_problem(&self.name(cx))
        {
            return Some(problem);
        }
        (self.ask.has_message() && self.message(cx).is_empty()).then_some("Type the tag's message")
    }

    pub(crate) fn named(&self, cx: &gpui::App) -> Named {
        let name = match &self.ask {
            NameAsk::Annotate { name, .. } => name.clone(),
            _ => self.name(cx),
        };
        Named { ask: self.ask.clone(), name, message: self.message(cx), checkout: self.inline.is_some() || self.checkout }
    }

    pub(crate) fn input(&self) -> Option<Entity<TextInput>> {
        self.fields.as_ref().map(|f| f.name.clone())
    }

    #[cfg(test)]
    pub(crate) fn type_in(&self, name: &str, message: &str, cx: &mut gpui::App) {
        if let Some(fields) = &self.fields {
            fields.name.update(cx, |i, cx| i.set_text(name, cx));
            if let Some(m) = &fields.message {
                m.update(cx, |i, cx| i.set_text(message, cx));
            }
        }
    }
}

impl BranchActions {
    /// The dialog's fields, made once a window is at hand, the first taking the keyboard.
    pub(crate) fn ensure_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_add_worktree(window, cx);
        let Some(dialog) = self.naming.as_mut().filter(|d| d.fields.is_none()) else { return };
        let given = match &dialog.ask {
            NameAsk::Rename { from } => from.clone(),
            _ => String::new(),
        };
        let name = cx.new(|cx| TextInput::new("Name", None, window, cx));
        name.update(cx, |i, cx| i.set_text(&given, cx));
        let message = dialog.ask.has_message().then(|| cx.new(|cx| TextInput::new("Message", None, window, cx)));
        let first = if dialog.ask.has_name() { name.clone() } else { message.clone().unwrap_or(name.clone()) };
        first.update(cx, |i, cx| i.select_all_text(cx));
        supraman_ui::focus::request(&first.focus_handle(cx), window, cx);
        let listen = |input: &Entity<TextInput>, cx: &mut Context<Self>| {
            cx.subscribe(input, |this: &mut BranchActions, _, e: &InputEvent, cx| match e {
                InputEvent::Confirm => this.submit_name(cx),
                InputEvent::Cancel => this.cancel_name(cx),
                _ => cx.notify(),
            })
        };
        let mut events = vec![listen(&name, cx)];
        events.extend(message.as_ref().map(|m| listen(m, cx)));
        dialog.fields = Some(Fields { name, message, tick: cx.focus_handle(), _events: events });
    }

    pub(crate) fn cancel_name(&mut self, cx: &mut Context<Self>) {
        self.naming = None;
        cx.notify();
    }

    /// The field a row of `sha` shows in place of its pills.
    pub(crate) fn inline_field(&self, sha: &str) -> Option<Entity<TextInput>> {
        self.naming.as_ref().filter(|d| d.inline.as_deref() == Some(sha))?.input()
    }

    fn toggle_checkout(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.naming {
            d.checkout = !d.checkout;
        }
        cx.notify();
    }

    /// The name asked in the tab's prompt: its fields, why it cannot be made yet, Check out after creating, then Create
    /// (Rename) and Cancel; hit keys `git-name-field`, `git-name-message`, `git-name-checkout`, `git-name-create`, `git-name-cancel`.
    pub(super) fn render_naming(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.naming.as_ref().filter(|d| d.inline.is_none())?;
        let fields = dialog.fields.as_ref()?;
        let problem = dialog.problem(cx);
        let words = if matches!(dialog.ask, NameAsk::Rename { .. }) { "Rename" } else { "Create" };
        let create = DialogButton::new("git-name-create", words, button::Tone::Neutral, |this: &mut Self, _, cx| this.submit_name(cx)).dimmed(problem);
        let cancel = DialogButton::new("git-name-cancel", "Cancel", button::Tone::Neutral, |this: &mut Self, _, cx| this.cancel_name(cx));
        let checkout = matches!(dialog.ask, NameAsk::Branch { .. }).then(|| {
            let tick = button::focusable_checkbox(&self.hits, "git-name-checkout", dialog.checkout, &fields.tick, cx.listener(|this, _: &Press, _, cx| this.toggle_checkout(cx)), cx);
            let ring = supraman_ui::focus::ring(&fields.tick, supraman_ui::tokens::radius::S);
            div().id("git-name-checkout-row").relative().flex().items_center().gap_2().cursor_pointer().child(tick).child("Check out after creating").child(ring).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_checkout(cx)))
        });
        let shown_problem = problem.filter(|_| !dialog.name(cx).is_empty() || !dialog.ask.has_name());
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .when(dialog.ask.has_name(), |d| d.child(form::input(&self.hits, "git-name-field", &fields.name)))
            .children(fields.message.as_ref().map(|m| form::input(&self.hits, "git-name-message", m)))
            .children(shown_problem.map(|p| div().relative().text_xs().text_color(cx.theme().status().error).child(p).child(hit_target(&self.hits, "git-name-problem".into()))))
            .children(checkout);
        let mut members: Vec<FocusHandle> = Vec::new();
        if dialog.ask.has_name() {
            members.push(fields.name.focus_handle(cx));
        }
        members.extend(fields.message.as_ref().map(|m| m.focus_handle(cx)));
        if matches!(dialog.ask, NameAsk::Branch { .. }) {
            members.push(fields.tick.clone());
        }
        let panel = Panel {
            key: "git-name".into(),
            title: Some(SharedString::from(dialog.ask.title())),
            body: body.into_any_element(),
            fields: members,
            buttons: vec![create, cancel],
            first: None,
            default: Some("git-name-create".into()),
            close: None,
            cancel: std::rc::Rc::new(|this: &mut Self, _, cx| this.cancel_name(cx)),
        };
        Some(self.writes.dialog().render(&self.hits, panel, cx))
    }
}

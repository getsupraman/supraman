//! The name of a new or renamed branch or tag, or an annotated tag's message: a dialog, or a field in the row's cell.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, Subscription, Window, div, prelude::*, px};
use supraman_gitman::write::branch::name_problem;
use supraman_ui::button;
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
        first.update(cx, |i, cx| i.focus_all(window, cx));
        let listen = |input: &Entity<TextInput>, cx: &mut Context<Self>| {
            cx.subscribe(input, |this: &mut BranchActions, _, e: &InputEvent, cx| match e {
                InputEvent::Confirm => this.submit_name(cx),
                InputEvent::Cancel => this.cancel_name(cx),
                _ => cx.notify(),
            })
        };
        let mut events = vec![listen(&name, cx)];
        events.extend(message.as_ref().map(|m| listen(m, cx)));
        dialog.fields = Some(Fields { name, message, _events: events });
    }

    pub(crate) fn cancel_name(&mut self, cx: &mut Context<Self>) {
        self.naming = None;
        cx.notify();
    }

    /// The field a row of `sha` shows in place of its pills.
    pub(crate) fn inline_field(&self, sha: &str) -> Option<Entity<TextInput>> {
        self.naming.as_ref().filter(|d| d.inline.as_deref() == Some(sha))?.input()
    }

    pub(super) fn render_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.naming.as_ref().filter(|d| d.inline.is_none())?;
        let fields = dialog.fields.as_ref()?;
        let colors = cx.theme().colors();
        let problem = dialog.problem(cx);
        let create = button::bordered(&self.hits, "git-name-create", if matches!(dialog.ask, NameAsk::Rename { .. }) { "Rename" } else { "Create" }, cx);
        let create = match problem {
            Some(_) => button::dimmed(create),
            None => create.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_name(cx))),
        };
        let cancel = button::bordered(&self.hits, "git-name-cancel", "Cancel", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_name(cx)));
        let checkout = matches!(dialog.ask, NameAsk::Branch { .. }).then(|| {
            let tick = button::checkbox(&self.hits, "git-name-checkout", dialog.checkout, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(d) = &mut this.naming {
                    d.checkout = !d.checkout;
                }
                cx.notify();
            }));
            div().flex().items_center().gap_2().child(tick).child("Check out after creating")
        });
        let card = div()
            .flex()
            .flex_col()
            .gap_2()
            .m_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .child(div().text_xs().text_color(colors.text_muted).child(dialog.ask.title()))
            .when(dialog.ask.has_name(), |d| d.child(fields.name.clone()))
            .children(fields.message.clone())
            .children(problem.filter(|_| !dialog.name(cx).is_empty() || !dialog.ask.has_name()).map(|p| div().text_xs().text_color(cx.theme().status().error).child(p)))
            .children(checkout)
            .child(div().flex().justify_end().gap_2().child(create).child(cancel));
        Some(div().max_w(px(480.)).child(card).into_any_element())
    }
}

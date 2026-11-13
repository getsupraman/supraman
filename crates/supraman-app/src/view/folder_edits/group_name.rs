//! Add Group's and Rename's one field: a `validate_name` name no other group has; Enter adds, Esc cancels.
//! Hit keys `group-name`, `group-name-field`, `group-name-ok`, `group-name-close`.

use gpui::{Context, Entity, EventEmitter, FocusHandle, Focusable as _, IntoElement, Render, SharedString, Subscription, Window};
use supraman_core::{name_char, validate_name};
use supraman_ui::focus_order::FocusOrder;
use supraman_ui::form::{self, Answers, Form};
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;
use supraman_ui::text_input::TextInput;

use super::FolderEdit;

pub const KEY: &str = "group-name";
pub const NEEDS_NAME: &str = "Type the group's name";

const ANSWERS: Answers<GroupName> = Answers { ok: GroupName::submit, close: GroupName::close };

pub enum GroupNameEvent {
    Named(FolderEdit),
    Closed,
}

pub struct GroupName {
    renaming: Option<String>,
    /// Every group's name now.
    taken: Vec<String>,
    field: Entity<TextInput>,
    _edits: Subscription,
    ok: FocusHandle,
    hits: HitTargets,
}

impl EventEmitter<GroupNameEvent> for GroupName {}

impl GroupName {
    pub fn new(renaming: Option<String>, taken: Vec<String>, hits: HitTargets, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (field, edits) = form::field_with("name", renaming.as_deref().unwrap_or_default(), Some(name_char), ANSWERS, |_, cx| cx.notify(), window, cx);
        field.update(cx, |i, cx| i.focus(window, cx));
        Self { renaming, taken, field, _edits: edits, ok: cx.focus_handle(), hits }
    }

    fn typed(&self, cx: &gpui::App) -> String {
        self.field.read(cx).text().trim().to_string()
    }

    /// Why OK cannot act on what is typed.
    pub fn why_not(&self, cx: &gpui::App) -> Option<String> {
        let name = self.typed(cx);
        if name.is_empty() {
            return Some(NEEDS_NAME.into());
        }
        if let Err(e) = validate_name(&name) {
            return Some(e.to_string());
        }
        (self.renaming.as_deref() != Some(name.as_str()) && self.taken.contains(&name)).then(|| format!("A group named {name} exists already"))
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.why_not(cx).is_some() {
            return;
        }
        let to = self.typed(cx);
        cx.emit(match self.renaming.clone() {
            Some(from) if from == to => GroupNameEvent::Closed,
            Some(from) => GroupNameEvent::Named(FolderEdit::RenameGroup { from, to }),
            None => GroupNameEvent::Named(FolderEdit::AddGroup(to)),
        });
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(GroupNameEvent::Closed);
    }

    #[cfg(test)]
    pub fn field(&self) -> &Entity<TextInput> {
        &self.field
    }
}

impl Render for GroupName {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, ok) = if self.renaming.is_some() { ("Rename Group", "Rename") } else { ("Add Group", "Add Group") };
        let why = self.why_not(cx).map(SharedString::from);
        let form = Form { key: KEY.into(), icon: Icon::Group, title: title.into(), ok: ok.into(), why, running: false, answers: ANSWERS };
        let fields = vec![(SharedString::from("Name"), form::input(&self.hits, format!("{KEY}-field"), &self.field))];
        let order = FocusOrder::new([self.field.focus_handle(cx), self.ok.clone()]);
        form.render(&self.hits, fields, order, Some(&self.ok), cx)
    }
}

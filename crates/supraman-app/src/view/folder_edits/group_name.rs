//! A group's name typed in place on its row (Rename…, F2, Add Group's new row): a `validate_name` name no other group has.
//! Enter names it, Esc cancels, focus lost in the active window names it while it is valid and cancels otherwise.

use gpui::{AppContext as _, Context, Entity, EventEmitter, Focusable as _, Subscription, Window};
use supraman_core::{name_char, validate_name};
use supraman_ui::text_input::{InputEvent, TextInput};

use super::FolderEdit;

/// The field's hit key.
pub const FIELD: &str = "group-name-field";
pub const NEEDS_NAME: &str = "Type the group's name";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupNameEvent {
    Named(FolderEdit),
    /// Enter on a name it cannot take, and why; the field stays open.
    Refused(String),
    Closed,
}

pub struct GroupName {
    renaming: Option<String>,
    /// Every group's name now.
    taken: Vec<String>,
    field: Entity<TextInput>,
    /// Enter was refused on an empty name: its why shows until the next edit.
    empty_refused: bool,
    done: bool,
    _subs: [Subscription; 2],
}

impl EventEmitter<GroupNameEvent> for GroupName {}

impl GroupName {
    pub fn new(renaming: Option<String>, taken: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| TextInput::new("", Some(name_char), window, cx).inline());
        field.update(cx, |i, cx| {
            i.set_text(renaming.as_deref().unwrap_or_default(), cx);
            i.focus_all(window, cx);
        });
        let edits = cx.subscribe(&field, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Edited => this.edited(cx),
            InputEvent::Confirm => this.submit(cx),
            InputEvent::Cancel => this.close(cx),
            _ => {}
        });
        let blur = cx.on_blur(&field.focus_handle(cx), window, |this, window, cx| {
            if window.is_window_active() {
                this.left(cx);
            }
        });
        Self { renaming, taken, field, empty_refused: false, done: false, _subs: [edits, blur] }
    }

    /// The group renamed, or `None` for Add Group's new row.
    pub fn renaming(&self) -> Option<&str> {
        self.renaming.as_deref()
    }

    pub fn field(&self) -> &Entity<TextInput> {
        &self.field
    }

    fn typed(&self, cx: &gpui::App) -> String {
        self.field.read(cx).text().trim().to_string()
    }

    /// Why Enter cannot name what is typed.
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

    /// The why the field shows now: a name typed that it cannot take, or Enter refused on nothing typed.
    pub fn shown_why(&self, cx: &gpui::App) -> Option<String> {
        if self.typed(cx).is_empty() && !self.empty_refused {
            return None;
        }
        self.why_not(cx)
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        self.empty_refused = false;
        self.mark(cx);
    }

    fn mark(&mut self, cx: &mut Context<Self>) {
        let invalid = self.shown_why(cx).is_some();
        self.field.update(cx, |i, cx| i.set_invalid(invalid, cx));
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        if self.renaming.is_none() && self.typed(cx).is_empty() {
            return self.close(cx);
        }
        if let Some(why) = self.why_not(cx) {
            self.empty_refused = self.typed(cx).is_empty();
            self.mark(cx);
            return cx.emit(GroupNameEvent::Refused(why));
        }
        self.done = true;
        let to = self.typed(cx);
        cx.emit(match self.renaming.clone() {
            Some(from) if from == to => GroupNameEvent::Closed,
            Some(from) => GroupNameEvent::Named(FolderEdit::RenameGroup { from, to }),
            None => GroupNameEvent::Named(FolderEdit::AddGroup(to)),
        });
    }

    fn left(&mut self, cx: &mut Context<Self>) {
        match self.why_not(cx) {
            None => self.submit(cx),
            Some(_) => self.close(cx),
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.done, true) {
            cx.emit(GroupNameEvent::Closed);
        }
    }
}

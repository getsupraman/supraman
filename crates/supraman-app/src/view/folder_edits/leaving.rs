//! The schedules a Remove Folder or Remove Group leaves without a row ([`crate::fleet::Tree::schedule_row`]), the
//! question that offers to delete them, and the toast's words for what went.

use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui::{AnyElement, Context, FocusHandle, SharedString, div, prelude::*};
use supraman_core::paths::{folder_name, path_key};
use supraman_core::schedule::folder_key;
use supraman_core::text;
use supraman_ui::button;
use supraman_ui::dialog::{Button as DialogButton, Dialog, Panel};
use supraman_ui::hit::hit_target;
use theme::ActiveTheme as _;

use super::FolderEdit;
use crate::fleet::Tree;
use crate::view::FleetView;
use supraman_ui::checkbox::Checkbox;

/// The question's hit keys: `<KEY>-tick`, `-text`, `-note`, `-yes`, `-alt`, `-cancel` and the dialog's own.
pub const KEY: &str = "remove-folder";

/// The `(folder key, id)` of `schedules` whose folder has a row only among the rows `edit` takes from `tree`.
pub fn orphans<'a>(edit: &FolderEdit, tree: &Tree, schedules: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(String, String)> {
    let gone = removed_rows(edit, tree);
    if gone.is_empty() {
        return Vec::new();
    }
    let (lost, kept): (Vec<_>, Vec<_>) = tree.folders().map(|(_, f)| f).filter(|f| !f.excluded).partition(|f| gone.contains(f.key.as_str()));
    let kept: BTreeSet<String> = kept.iter().map(|f| folder_key(&f.path)).collect();
    let lost: BTreeSet<String> = lost.iter().map(|f| folder_key(&f.path)).filter(|k| !kept.contains(k)).collect();
    schedules.into_iter().filter(|(k, _)| lost.contains(*k)).map(|(k, id)| (k.to_string(), id.to_string())).collect()
}

/// The keys of the rows `edit` takes: a listed folder's with its shown children's, or a group's folders' with it.
fn removed_rows<'t>(edit: &FolderEdit, tree: &'t Tree) -> BTreeSet<&'t str> {
    let rows = || tree.folders().map(|(_, f)| f);
    match edit {
        FolderEdit::RemoveFolder { pos, path } => {
            let Some(row) = rows().find(|f| f.pos == Some(*pos) && path_key(&f.path) == path_key(path)) else {
                return BTreeSet::new();
            };
            rows().filter(|f| f.key == row.key || f.parent.as_deref() == Some(row.key.as_str())).map(|f| f.key.as_str()).collect()
        }
        FolderEdit::RemoveGroup { name, with_folders: true } => tree.entries.iter().filter(|e| e.group && e.name == *name).flat_map(|e| e.folders.iter().map(|f| f.key.as_str())).collect(),
        _ => BTreeSet::new(),
    }
}

/// The schedules a removal leaves without a row, by folder and id, and whether they go with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Leaving {
    pub schedules: Vec<(PathBuf, String)>,
    pub delete: bool,
}

impl Leaving {
    /// The toast's words for `edit` once written; `None` while it leaves no schedule.
    pub fn words(&self, edit: &FolderEdit) -> Option<String> {
        let n = self.schedules.len();
        if n == 0 {
            return None;
        }
        let tasks = text::count(n, "scheduled task", "scheduled tasks");
        let are = if n == 1 { "is" } else { "are" };
        Some(match (edit, self.delete) {
            (FolderEdit::RemoveFolder { path, .. }, true) => format!("Removed {} and {tasks}.", folder_name(path)),
            (FolderEdit::RemoveFolder { path, .. }, false) => format!("Removed {}; its {tasks} {are} under Other schedules.", folder_name(path)),
            (FolderEdit::RemoveGroup { name, .. }, true) => format!("Removed group {name} with its folders and {tasks}."),
            (FolderEdit::RemoveGroup { name, .. }, false) => format!("Removed group {name} with its folders; their {tasks} {are} under Other schedules."),
            _ => return None,
        })
    }
}

/// One answer: its key's suffix, its words and its edit; `ticks` when the tick box deletes its orphans.
pub struct Answer {
    suffix: &'static str,
    words: String,
    edit: FolderEdit,
    ticks: bool,
}

/// Remove Folder's, or Remove Group's, question while it would leave schedules without a row.
pub struct RemoveQuestion {
    title: &'static str,
    text: String,
    answers: Vec<Answer>,
    pub schedules: Vec<(PathBuf, String)>,
    pub delete: bool,
    tick: FocusHandle,
    dialog: Dialog,
}

impl RemoveQuestion {
    pub fn folder(edit: FolderEdit, schedules: Vec<(PathBuf, String)>, tick: FocusHandle) -> Self {
        let name = match &edit {
            FolderEdit::RemoveFolder { path, .. } => folder_name(path),
            _ => String::new(),
        };
        let answers = vec![Answer { suffix: "yes", words: "Remove Folder".into(), edit, ticks: true }];
        Self { title: "Remove folder", text: format!("Remove the folder {name}? Its files and sessions stay."), answers, schedules, delete: false, tick, dialog: Dialog::default() }
    }

    /// Both of Remove Group's answers; the tick box deletes only with Remove its folders too.
    pub fn group(name: &str, folders: usize, schedules: Vec<(PathBuf, String)>, tick: FocusHandle) -> Self {
        let edit = |with_folders| FolderEdit::RemoveGroup { name: name.to_string(), with_folders };
        let answers = vec![
            Answer { suffix: "yes", words: "Move its folders to the top level".into(), edit: edit(false), ticks: false },
            Answer { suffix: "alt", words: "Remove its folders too".into(), edit: edit(true), ticks: true },
        ];
        let text = format!("Remove the group {name}? Its {} can move to the top level or go from the list with it; no files or sessions are touched.", text::count(folders, "folder", "folders"));
        Self { title: "Remove group", text, answers, schedules, delete: false, tick, dialog: Dialog::default() }
    }

    fn note(&self) -> String {
        let keeping = match self.schedules.len() {
            1 => "eeping it moves it to Other schedules",
            _ => "eeping them moves them to Other schedules",
        };
        match self.answers.len() {
            1 => format!("K{keeping}"),
            _ => format!("Only with Remove its folders too; k{keeping}"),
        }
    }

    /// The `i`th answer's edit and what goes with it.
    fn take(self, i: usize) -> Option<(FolderEdit, Leaving)> {
        let answer = self.answers.into_iter().nth(i)?;
        let leaving = if answer.ticks { Leaving { schedules: self.schedules, delete: self.delete } } else { Leaving::default() };
        Some((answer.edit, leaving))
    }
}

impl FleetView {
    fn toggle_remove_tick(&mut self, cx: &mut Context<Self>) {
        if let Some(q) = &mut self.dialogs.remove {
            q.delete = !q.delete;
        }
        cx.notify();
    }

    pub fn answer_remove(&mut self, answer: Option<usize>, cx: &mut Context<Self>) {
        if let Some((edit, leaving)) = self.dialogs.remove.take().zip(answer).and_then(|(q, i)| q.take(i)) {
            self.edit_folders_leaving(edit, leaving, cx);
        }
        cx.notify();
    }

    /// The question in the shared dialog: its text, the tick box with its note, then its answers and Cancel.
    pub(in crate::view) fn render_remove_question(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let q = self.dialogs.remove.as_ref()?;
        let muted = supraman_ui::themes::secondary(cx.theme().colors());
        let label = text::count(q.schedules.len(), "scheduled task", "scheduled tasks");
        let tick = Checkbox::new(&self.hits, format!("{KEY}-tick"), q.delete).label(format!("Also delete {label}")).focus(&q.tick).on_toggle(cx.listener(|this, _: &bool, _, cx| this.toggle_remove_tick(cx)));
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().relative().child(SharedString::from(q.text.clone())).child(hit_target(&self.hits, format!("{KEY}-text"))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(div().flex().child(tick))
                    .child(div().relative().pl_6().text_xs().text_color(muted).child(q.note()).child(hit_target(&self.hits, format!("{KEY}-note")))),
            );
        let mut buttons: Vec<_> = q
            .answers
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let tone = if a.ticks && q.delete { button::Tone::Danger } else { button::Tone::Neutral };
                DialogButton::new(format!("{KEY}-{}", a.suffix), a.words.clone(), tone, move |this: &mut Self, _, cx| this.answer_remove(Some(i), cx))
            })
            .collect();
        buttons.push(DialogButton::new(format!("{KEY}-cancel"), "Cancel", button::Tone::Neutral, |this: &mut Self, _, cx| this.answer_remove(None, cx)));
        let panel = Panel {
            key: KEY.into(),
            title: Some(q.title.into()),
            body: body.into_any_element(),
            fields: vec![q.tick.clone()],
            buttons,
            first: Some(format!("{KEY}-yes").into()),
            default: Some(format!("{KEY}-yes").into()),
            close: None,
            cancel: std::rc::Rc::new(|this: &mut Self, _, cx| this.answer_remove(None, cx)),
        };
        Some(q.dialog.render(&self.hits, panel, cx))
    }
}

#[cfg(test)]
mod tests;

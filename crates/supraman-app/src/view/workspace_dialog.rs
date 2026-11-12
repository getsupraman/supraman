//! Add workspace and Edit folders…: a name and a [`FolderList`].

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{AppContext as _, ClickEvent, Context, Entity, EventEmitter, SharedString, Subscription, Window, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_core::{CoreError, DEFAULT_DEPTH, Edit, Folder, Registry, Scan, Workspace, discover, name_char, validate_name};
use supraman_ui::banner;
use supraman_ui::button;
use supraman_ui::clipped::clipped;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::text_input::{InputEvent, TEXT_LEFT, TextInput};
use theme::ActiveTheme as _;

use super::folder_list::{self, FolderList, RowAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Its subfolders, ticked; it is the home and is scanned.
    Parent(PathBuf),
    Custom,
    /// A registered workspace as it was opened; Save emits its change as [`Edit`]s.
    Edit(Workspace),
}

pub enum DialogEvent {
    Saved { workspace: Workspace, taken: Vec<(String, String)> },
    Edit { workspace: String, edits: Vec<Edit> },
    /// Esc: the owner closes a popup of its own first.
    Cancel,
}

pub struct WorkspaceDialog {
    pub mode: Mode,
    pub name: Entity<TextInput>,
    pub path: Entity<TextInput>,
    pub list: FolderList,
    pub scanning: bool,
    pub saving: bool,
    pub error: Option<String>,
    pub hint: Option<String>,
    pub note: Option<String>,
    home: Option<PathBuf>,
    hits: HitTargets,
    _events: [Subscription; 2],
}

impl EventEmitter<DialogEvent> for WorkspaceDialog {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKey {
    Save,
    Cancel,
    Edited,
}

pub fn name_event(event: &InputEvent, error: &mut Option<String>, hint: &mut Option<String>) -> NameKey {
    match event {
        InputEvent::Confirm | InputEvent::ConfirmBack | InputEvent::Submit => NameKey::Save,
        InputEvent::Cancel => NameKey::Cancel,
        InputEvent::Refused(c) => {
            *hint = Some(format!("'{c}' is not taken: a name is lowercase letters, digits and '-'."));
            NameKey::Edited
        }
        InputEvent::Edited => {
            *error = None;
            *hint = None;
            NameKey::Edited
        }
    }
}

fn add_error(e: &CoreError) -> String {
    match e {
        CoreError::AlreadyExists(name) => format!("The name {name} is taken: choose another."),
        CoreError::InvalidName(_) => "Use lowercase letters, digits and '-' only for the name.".into(),
        other => other.to_string(),
    }
}

/// The toast's tail for the folders [`Registry::claim_folders`] disabled.
pub fn supervised_elsewhere(taken: &[(String, String)]) -> String {
    let mut by: Vec<(&str, Vec<&str>)> = Vec::new();
    for (folder, owner) in taken {
        match by.iter_mut().find(|(o, _)| o == owner) {
            Some((_, list)) => list.push(folder),
            None => by.push((owner, vec![folder])),
        }
    }
    by.iter()
        .map(|(owner, list)| {
            let (names, verb) = match list.as_slice() {
                [one] => (one.to_string(), "stays"),
                [rest @ .., last] => (format!("{} and {last}", rest.join(", ")), "stay"),
                [] => unreachable!(),
            };
            format!(" {names} {verb} supervised in {owner}, disabled here.")
        })
        .collect()
}

/// A parent's ticked folders with a scan that excludes the rest, or the custom rows with home `None` while it is the first.
fn workspace_of(name: String, mode: &Mode, list: &FolderList) -> Workspace {
    let marked = list.home_index().map(|i| list.rows[i].folder.path.clone());
    match mode {
        Mode::Parent(root) => {
            let mut w = Workspace::new(name, marked.unwrap_or_else(|| root.clone()), list.folders());
            let mut scan = Scan::new(root.clone(), DEFAULT_DEPTH);
            let left_out = list.unticked().chain(&list.removed).filter(|f| !f.added);
            scan.exclude = left_out.map(|f| f.path.clone()).collect();
            w.scans.push(scan);
            w
        }
        Mode::Custom | Mode::Edit(_) => {
            let mut w = Workspace::new(name, PathBuf::new(), list.folders());
            w.home = list.home_index().filter(|&i| i > 0).and(marked);
            w
        }
    }
}

/// A gone folder is dropped, not skipped; an unticked new row is neither added nor skipped.
pub fn edits_of(original: &Workspace, list: &FolderList) -> Vec<Edit> {
    let listed = |f: &Folder| list.rows.iter().any(|r| path_key(&r.folder.path) == path_key(&f.path));
    let (gone, taken): (Vec<&Folder>, Vec<&Folder>) = original.folders.iter().filter(|f| !listed(f)).partition(|f| list.gone.contains(&f.name));
    let mut edits: Vec<Edit> = taken.iter().map(|f| Edit::RemoveFolder(f.name.clone())).collect();
    if !gone.is_empty() {
        edits.push(Edit::DropGone(gone.iter().map(|f| f.name.clone()).collect()));
    }
    let new = list.rows.iter().filter(|r| r.included != Some(false) && original.folder_at(&r.folder.path).is_none());
    edits.extend(new.map(|r| Edit::AddFolder { path: r.folder.path.clone(), enable: true }));    let home = match list.home_index() {
        None => original.home.clone(),
        Some(0) if list.first_is_home => None,
        Some(i) => Some(list.rows[i].folder.path.clone()),
    };
    if home.as_deref().map(path_key) != original.home.as_deref().map(path_key) {
        edits.push(Edit::SetHome(home));
    }
    edits
}

/// Save's registry write: checked, its folders claimed ([`Registry::claim_folders`]), never over another.
fn add(registry: Registry, mut workspace: Workspace) -> Result<(Workspace, Vec<(String, String)>), String> {
    validate_name(&workspace.name).map_err(|e| add_error(&e))?;
    registry.check_new(&workspace).map_err(|e| add_error(&e))?;
    let taken = registry.claim_folders(&mut workspace, |_| true).map_err(|e| add_error(&e))?;
    registry.save(&workspace, false).map_err(|e| add_error(&e))?;
    Ok((workspace, taken))
}

/// `path` as a folder beside `folders`, named apart from them; `None` when it is one of them.
fn folder_of(folders: Vec<Folder>, path: &Path) -> Result<Option<Folder>, String> {
    let mut w = Workspace::new("draft", PathBuf::new(), folders);
    let before = w.folders.len();
    let folder = w.add_directory(path).map_err(|e| e.to_string())?.clone();
    Ok((w.folders.len() > before).then_some(folder))
}

impl WorkspaceDialog {
    /// `home`: the Supraman home; `others`: the registered workspaces.
    pub fn new(mode: Mode, home: Option<PathBuf>, others: &[Workspace], hits: HitTargets, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| TextInput::new("workspace name", Some(name_char), window, cx));
        let path = cx.new(|cx| TextInput::new("a folder's path", None, window, cx));
        let events = [cx.subscribe_in(&name, window, Self::on_name), cx.subscribe_in(&path, window, Self::on_path)];
        name.update(cx, |name, cx| name.focus_all(window, cx));
        let scanning = matches!(mode, Mode::Parent(_));
        let list = match &mode {
            Mode::Edit(w) => {
                name.update(cx, |name, cx| name.set_text(&w.name, cx));
                path.update(cx, |path, cx| path.focus_all(window, cx));
                let others: Vec<Workspace> = others.iter().filter(|o| o.name != w.name).cloned().collect();
                FolderList::of_workspace(w, &others)
            }
            _ => FolderList::new(others, mode == Mode::Custom),
        };
        let this = Self { mode, name, path, list, scanning, saving: false, error: None, hint: None, note: None, home, hits, _events: events };
        if let Mode::Parent(root) = &this.mode {
            this.scan(root.clone(), cx);
        }
        this
    }

    fn scan(&self, root: PathBuf, cx: &mut Context<Self>) {
        let scan = cx.background_executor().spawn(async move {
            match discover(&root, None, DEFAULT_DEPTH) {
                Err(CoreError::InvalidName(_)) => discover(&root, Some("workspace"), DEFAULT_DEPTH).map(|w| (w, true)),
                other => other.map(|w| (w, false)),
            }
        });
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            this.update(cx, |this, cx| {
                this.scanning = false;
                match found {
                    Ok((workspace, renamed)) => {
                        let root = workspace.home().map(Path::to_path_buf).unwrap_or_default();
                        this.name.update(cx, |name, cx| name.set_text(&workspace.name, cx));
                        for f in workspace.folders {
                            this.list.push(f, Some(true));
                        }
                        this.list.mark_home(&root);
                        this.mode = Mode::Parent(root);
                        this.error = renamed.then(|| "The folder's name gives no valid workspace name: choose one.".into());
                    }
                    Err(e) => this.error = Some(add_error(&e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A rescan's new subfolders, unticked, and its folders gone from disk, out of the list until Save drops them.
    pub fn offer(&mut self, found: Vec<Folder>, gone: &[String]) {
        for f in found {
            self.list.push(f, Some(false));
        }
        self.list.drop_gone(gone);
    }

    fn prefix(&self) -> &'static str {
        if matches!(self.mode, Mode::Edit(_)) { "edit" } else { "add" }
    }

    pub fn name_text(&self, cx: &gpui::App) -> String {
        self.name.read(cx).text().to_string()
    }

    pub fn focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.name.read(cx).is_focused(window) || self.path.read(cx).is_focused(window)
    }

    pub fn can_save(&self, cx: &gpui::App) -> bool {
        if matches!(self.mode, Mode::Edit(_)) {
            return !self.saving;
        }
        !self.scanning && !self.saving && validate_name(&self.name_text(cx)).is_ok() && !self.list.folders().is_empty()
    }

    fn on_name(&mut self, _: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match name_event(event, &mut self.error, &mut self.hint) {
            NameKey::Save => self.save(window, cx),
            NameKey::Cancel => cx.emit(DialogEvent::Cancel),
            NameKey::Edited => {}
        }
        cx.notify();
    }

    fn on_path(&mut self, _: &Entity<TextInput>, event: &InputEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Edited | InputEvent::Refused(_) => self.error = None,
            InputEvent::Confirm | InputEvent::ConfirmBack | InputEvent::Submit => self.add_typed(cx),
            InputEvent::Cancel => cx.emit(DialogEvent::Cancel),
        }
        cx.notify();
    }

    pub fn act(&mut self, action: RowAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            RowAction::Include(i) => self.list.toggle(i),
            RowAction::Home(i) => self.list.set_home(i),
            RowAction::Remove(i) => self.list.remove(i),
            RowAction::Add => self.add_typed(cx),
            RowAction::Browse => {
                supraman_ui::folder_picker::pick_folder("Add folder", window, cx, |this: &mut Self, path, _, cx| this.add_path(path, cx));
            }
        }
        cx.notify();
    }

    fn add_typed(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = folder_list::typed(&self.path, cx) {
            self.add_path(path, cx);
        }
    }

    /// Reads the folder off the UI thread; the typed path clears once it is in the list.
    pub fn add_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let folders: Vec<Folder> = self.list.rows.iter().map(|r| r.folder.clone()).collect();
        let read = cx.background_executor().spawn(async move { folder_of(folders, &path).map(|f| (f, path)) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                let included = matches!(this.mode, Mode::Parent(_)).then_some(true);
                match read {
                    Ok((Some(folder), _)) => {
                        this.list.push(folder, included);
                        this.path.update(cx, |p, cx| p.set_text("", cx));
                    }
                    Ok((_, path)) => this.error = Some(format!("{} is in the list already.", path.display())),
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        if let Mode::Edit(w) = &self.mode {
            let edits = edits_of(w, &self.list);
            cx.emit(DialogEvent::Edit { workspace: w.name.clone(), edits });
            return;
        }
        let workspace = workspace_of(self.name_text(cx), &self.mode, &self.list);
        self.saving = true;
        let registry = self.home.clone().map(Registry::at).ok_or_else(|| "no Supraman home".to_string());
        let save = cx.background_executor().spawn(async move { add(registry?, workspace) });
        cx.spawn_in(window, async move |this, cx| {
            let result = save.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok((workspace, taken)) => cx.emit(DialogEvent::Saved { workspace, taken }),
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for WorkspaceDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let hits = self.hits.clone();
        let on: folder_list::OnRow<Self> = Rc::new(|this, action, window, cx| this.act(action, window, cx));
        let p = self.prefix();
        let list = folder_list::render(&self.list, &self.path, p, &hits, on, cx);
        let can_save = self.can_save(cx);
        let save = button::action(&hits, format!("{p}-save"), "Save", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            this.save(window, cx);
        }));
        let save = if can_save { save } else { button::dimmed(save) };
        let cancel = button::action(&hits, format!("{p}-cancel"), "Cancel", button::Tone::Neutral, button::Fill::Outline, cx).on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            cx.emit(DialogEvent::Cancel);
        }));
        let name = match &self.mode {
            Mode::Edit(w) => div().id("edit-name-line").relative().flex_1().min_w_0().child(clipped("edit-name", w.name.clone(), false)).child(hit_target(&hits, "edit-name".into())),
            _ => div()
                .id("add-name")
                .relative()
                .flex_1()
                .min_w_0()
                .child(self.name.clone())
                .child(hit_target(&hits, "add-name".into()))
                .child(div().absolute().top_0().bottom_0().left(TEXT_LEFT).right(TEXT_LEFT).child(hit_target(&hits, "add-name-text".into()))),
        };
        let parent = match &self.mode {
            Mode::Parent(root) if !root.as_os_str().is_empty() => Some(root.display().to_string()),
            Mode::Edit(w) if !w.scans.is_empty() => {
                Some(format!("parent folder {}", w.scans.iter().map(|s| s.path.display().to_string()).collect::<Vec<_>>().join(" · ")))
            }
            _ => None,
        };
        let title = if matches!(self.mode, Mode::Edit(_)) { "EDIT FOLDERS" } else { "ADD WORKSPACE" };
        div()
            .id(SharedString::from(format!("{p}-dialog")))
            .relative()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .mx_3()
            .my_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border_focused)
            .bg(colors.editor_background)
            .child(hit_target(&hits, format!("{p}-dialog")))
            .child(div().text_xs().text_color(colors.text_muted).child(title))
            .child(div().flex().items_center().gap_2().text_sm().child("name").child(name))
            .children(parent.map(|parent| {
                let line = clipped(format!("{p}-parent"), parent, true).text_xs().text_color(colors.text_muted);
                div().relative().min_w_0().child(line).child(hit_target(&hits, format!("{p}-parent")))
            }))
            .children(self.note.clone().map(|n| div().text_xs().text_color(colors.text_muted).child(SharedString::from(n))))
            .children(self.hint.clone().map(|h| div().text_xs().text_color(status.warning).child(SharedString::from(h))))
            .when(self.scanning, |d| d.child(div().text_xs().text_color(colors.text_muted).child("scanning…")))
            .child(list)
            .children(self.error.clone().map(|e| {
                banner::tinted(status.error).relative().px_1().py_0().text_xs().child(SharedString::from(e)).child(hit_target(&hits, format!("{p}-error")))
            }))
            .child(div().flex().gap_2().justify_end().pt(px(2.)).child(save).child(cancel))
    }
}

#[cfg(test)]
mod tests {
    use supraman_core::FolderKind;

    use super::*;

    fn folder(root: &Path, name: &str) -> Folder {
        Folder { kind: FolderKind::Plain, ..Folder::new(name, root.join(name)) }
    }

    #[test]
    fn a_parent_save_registers_the_ticked_folders_and_scans_the_parent_without_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let mut list = FolderList::new(&[], false);
        for name in ["alpha", "beta", "notes"] {
            list.push(folder(&root, name), Some(true));
        }
        list.toggle(2);
        let (saved, taken) = add(Registry::at(tmp.path()), workspace_of("one".into(), &Mode::Parent(root.clone()), &list)).unwrap();
        assert_eq!(saved.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["alpha", "beta"]);
        assert_eq!((saved.home.as_deref(), taken.len()), (Some(root.as_path()), 0));
        assert_eq!(saved.scans, [Scan { path: root.clone(), depth: DEFAULT_DEPTH, exclude: vec![root.join("notes")] }]);
        assert_eq!(Registry::at(tmp.path()).load("one").unwrap(), saved, "the value shown at once is the one saved");

        list.set_home(1);
        list.remove(0);
        let w = workspace_of("two".into(), &Mode::Parent(root.clone()), &list);
        assert_eq!(w.home, Some(root.join("beta")));
        assert_eq!(w.scans[0].exclude, [root.join("notes"), root.join("alpha")], "a removed folder stays out of a rescan too");
        let (second, taken) = add(Registry::at(tmp.path()), w).unwrap();
        assert_eq!(taken, [("beta".to_string(), "one".to_string())]);
        assert!(!second.folders[0].enabled);

        let e = add(Registry::at(tmp.path()), workspace_of("one".into(), &Mode::Parent(root.clone()), &list)).unwrap_err();
        assert_eq!(e, "The name one is taken: choose another.");
        let e = add(Registry::at(tmp.path()), workspace_of("Bad Name".into(), &Mode::Parent(root), &list)).unwrap_err();
        assert_eq!(e, "Use lowercase letters, digits and '-' only for the name.");
    }

    #[test]
    fn a_custom_save_stores_no_home_for_the_first_folder_and_the_row_marked_otherwise() {
        let tmp = tempfile::tempdir().unwrap();
        let mut list = FolderList::new(&[], true);
        list.push(folder(tmp.path(), "a"), None);
        list.push(folder(tmp.path(), "b"), None);
        let w = workspace_of("first".into(), &Mode::Custom, &list);
        assert_eq!((w.home.as_deref(), w.home()), (None, Some(tmp.path().join("a").as_path())));
        assert!(w.scans.is_empty());
        list.set_home(1);
        let (saved, _) = add(Registry::at(tmp.path().join("home")), workspace_of("second".into(), &Mode::Custom, &list)).unwrap();
        assert_eq!(saved.home, Some(tmp.path().join("b")));
        assert_eq!(Registry::at(tmp.path().join("home")).load("second").unwrap().home, Some(tmp.path().join("b")));
    }

    #[test]
    fn a_typed_folder_is_read_named_apart_and_refused_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("x").join("a")).unwrap();
        std::fs::create_dir_all(tmp.path().join("y").join("a")).unwrap();
        let first = folder_of(Vec::new(), &tmp.path().join("x").join("a")).unwrap().unwrap();
        let second = folder_of(vec![first.clone()], &tmp.path().join("y").join("a")).unwrap().unwrap();
        assert_eq!((first.name.as_str(), second.name.as_str(), second.kind), ("a", "a-2", FolderKind::Plain));
        assert_eq!(folder_of(vec![first.clone()], &first.path).unwrap(), None);
        assert!(folder_of(Vec::new(), &tmp.path().join("gone")).is_err());
    }

    #[test]
    fn the_name_takes_only_what_a_workspace_name_allows() {
        use supraman_ui::text_input::filtered;
        assert_eq!(filtered("My-Repo2", name_char), ("my-repo2".to_string(), None));
        assert_eq!(filtered("a b", name_char), ("ab".to_string(), Some(' ')));
        assert_eq!(filtered("a_b", name_char), ("ab".to_string(), Some('_')));
        assert_eq!(filtered("aä7", name_char), ("a7".to_string(), Some('ä')));
        assert_eq!(filtered("ä", name_char), (String::new(), Some('ä')));
    }

    #[test]
    fn the_name_says_why_a_character_is_not_taken_until_the_next_edit() {
        let (mut error, mut hint) = (Some("e".to_string()), None);
        assert_eq!(name_event(&InputEvent::Refused('ä'), &mut error, &mut hint), NameKey::Edited);
        assert_eq!(hint.as_deref(), Some("'ä' is not taken: a name is lowercase letters, digits and '-'."));
        assert_eq!(error.as_deref(), Some("e"));
        name_event(&InputEvent::Edited, &mut error, &mut hint);
        assert_eq!((error, hint), (None, None));
        assert_eq!(name_event(&InputEvent::Confirm, &mut None, &mut None), NameKey::Save);
        assert_eq!(name_event(&InputEvent::Cancel, &mut None, &mut None), NameKey::Cancel);
    }

    #[test]
    fn errors_read_as_what_to_do() {
        assert_eq!(add_error(&CoreError::AlreadyExists("x".into())), "The name x is taken: choose another.");
        assert_eq!(supervised_elsewhere(&[]), "");
        let taken = [("Alpha".to_string(), "larchsys".to_string()), ("Beta".to_string(), "larchsys".to_string())];
        assert_eq!(supervised_elsewhere(&taken), " Alpha and Beta stay supervised in larchsys, disabled here.");
        assert_eq!(supervised_elsewhere(&taken[..1]), " Alpha stays supervised in larchsys, disabled here.");
    }
}

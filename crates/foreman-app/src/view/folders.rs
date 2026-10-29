//! Edit folders…: a registered workspace's folders, home and scans, changed through [`Registry::edit`].

use std::path::{Path, PathBuf};

use foreman_core::paths::within;
use foreman_core::{DEFAULT_DEPTH, Edit, Edited, Folder, Registry, Workspace, owners};
use foreman_supervisor::{DesiredState, repo_session_name};
use foreman_ui::button;
use foreman_ui::hit::hit_target;
use foreman_ui::text_input::{InputEvent, TextInput};
use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, SharedString, Subscription, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use super::FleetView;
use super::left::{secondary, tag};
use super::menu::{Confirm, Confirmed};
use crate::fleet;

pub struct EditFolders {
    pub workspace: String,
    pub path: Entity<TextInput>,
    pub error: Option<String>,
    _path_events: Subscription,
}

/// What the edit's toast says beyond "done": folders claimed by their owner, sessions stopped, and a running
/// session the new folder holds.
pub fn edited_note(done: &Edited, stopped: &[String], live_in: &[(String, String, bool)]) -> String {
    let mut out = String::new();
    for (folder, owner) in &done.claimed {
        out.push_str(&format!(" {folder} stays supervised in {owner}, disabled here."));
    }
    for name in stopped {
        out.push_str(&format!(" {name} stops (stop, never rm)."));
    }
    for (session, folder, enabled) in live_in {
        out.push_str(&format!(" {session} in {folder} now shows here"));
        out.push_str(if *enabled { "." } else { "; Enable to supervise a session of its own." });
    }
    out
}

/// The supervised sessions `edit` would stop: those of the folders it removes or disables that `wanted` holds.
pub fn stops(workspace: &Workspace, edit: &Edit, wanted: &[String]) -> Vec<String> {
    let folders: Vec<&Folder> = match edit {
        Edit::RemoveFolder(f) | Edit::SetEnabled { folder: f, enabled: false } => workspace.find_folder(f).into_iter().collect(),
        Edit::RemoveScan(path) => {
            let rest: Vec<&Path> = workspace.scans.iter().map(|s| s.path.as_path()).filter(|p| !foreman_core::same_path(p, path)).collect();
            workspace.folders.iter().filter(|f| !f.added && within(&f.path, path) && !rest.iter().any(|r| within(&f.path, r))).collect()
        }
        _ => Vec::new(),
    };
    folders.iter().filter_map(|f| repo_session_name(&workspace.name, &f.name)).filter(|n| wanted.contains(n)).collect()
}

fn verb(edit: &Edit) -> String {
    match edit {
        Edit::RemoveFolder(f) => format!("Remove folder {f}"),
        Edit::SetEnabled { folder, .. } => format!("Disable folder {folder}"),
        Edit::RemoveScan(p) => format!("Remove the scan of {} and the folders only it found", p.display()),
        _ => "Change it".into(),
    }
}

impl FleetView {
    pub fn edit_folders(&mut self, workspace: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.adding = None;
        let path = cx.new(|cx| TextInput::new("a folder's path", None, window, cx));
        let events = cx.subscribe_in(&path, window, |this: &mut Self, _, event: &InputEvent, window, cx| this.on_path_event(event, window, cx));
        self.editing = Some(EditFolders { workspace: workspace.to_string(), path, error: None, _path_events: events });
        self.collapsed.remove(&format!("workspace:{workspace}"));
        cx.notify();
    }

    fn on_path_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = &mut self.editing else {
            return;
        };
        match event {
            InputEvent::Edited | InputEvent::Refused(_) => editing.error = None,
            InputEvent::Confirm | InputEvent::ConfirmBack => self.add_typed(false, cx),
            InputEvent::Cancel if self.dismiss_popup() => {}
            InputEvent::Cancel => {
                self.editing = None;
                self.focus.focus(window, cx);
            }
        }
        cx.notify();
    }

    pub fn close_edit_folders(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    fn typed_path(&self, cx: &gpui::App) -> Option<PathBuf> {
        let text = self.editing.as_ref()?.path.read(cx).text().trim().trim_matches('"').to_string();
        (!text.is_empty()).then(|| PathBuf::from(text))
    }

    /// The path typed, as a folder (`scan` false) or a scan; an empty field opens the folder picker.
    pub fn add_typed(&mut self, scan: bool, cx: &mut Context<Self>) {
        let Some(w) = self.editing.as_ref().map(|e| e.workspace.clone()) else {
            return;
        };
        match self.typed_path(cx) {
            Some(path) => self.add_path(&w, path, scan, cx),
            None => self.browse(scan, cx),
        }
    }

    pub fn set_typed_home(&mut self, cx: &mut Context<Self>) {
        let Some(w) = self.editing.as_ref().map(|e| e.workspace.clone()) else {
            return;
        };
        match self.typed_path(cx) {
            Some(path) => self.apply_edit(&w, Edit::SetHome(Some(path)), cx),
            None => self.edit_failed("Type or browse to the folder to run the workspace's own sessions in.".into(), cx),
        }
    }

    fn add_path(&mut self, w: &str, path: PathBuf, scan: bool, cx: &mut Context<Self>) {
        let edit = if scan { Edit::AddScan { path, depth: DEFAULT_DEPTH } } else { Edit::AddFolder { path, enable: true } };
        self.apply_edit(w, edit, cx);
    }

    /// The folder picker, or [`FleetView::pick_folder`] when a selftest set one.
    pub fn browse(&mut self, scan: bool, cx: &mut Context<Self>) {
        let Some(w) = self.editing.as_ref().map(|e| e.workspace.clone()) else {
            return;
        };
        if let Some(path) = self.pick_folder.take() {
            self.add_path(&w, path, scan, cx);
            return;
        }
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(if scan { "Scan for repos" } else { "Add folder" }.into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                this.update(cx, |this, cx| this.add_path(&w, path, scan, cx)).ok();
            }
        })
        .detach();
    }

    /// Asks first when the edit stops a supervised session ([`stops`]).
    pub fn request_edit(&mut self, w: &str, edit: Edit, cx: &mut Context<Self>) {
        let wanted = self.desired.get(w).map(|d| d.1.clone()).unwrap_or_default();
        let stopping = self.workspaces.iter().find(|x| x.name == w).map(|x| stops(x, &edit, &wanted)).unwrap_or_default();
        if stopping.is_empty() {
            self.apply_edit(w, edit, cx);
            return;
        }
        self.confirm = Some(Confirm::ask(
            format!(
                "{} in workspace {w}? Its supervised session {} is stopped (stop, never rm) and can be resumed; no files are deleted.",
                verb(&edit),
                stopping.join(", ")
            ),
            "Stop and remove",
            Confirmed::Edit { workspace: w.to_string(), edit },
        ));
        cx.notify();
    }

    pub fn apply_edit(&mut self, w: &str, edit: Edit, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return self.edit_failed("no Foreman home".into(), cx);
        };
        let disabling = match &edit {
            Edit::SetEnabled { folder, enabled: false } => Some(folder.clone()),
            _ => None,
        };
        let result = Registry::at(&home).edit(w, edit).map_err(|e| e.to_string()).and_then(|done| {
            let mut released: Vec<&str> = done.removed.iter().map(|f| f.name.as_str()).collect();
            let disabled = disabling.as_deref().and_then(|f| done.workspace.find_folder(f)).map(|f| f.name.clone());
            released.extend(disabled.as_deref());
            let stopped = DesiredState::release(&home, w, &released).map_err(|e| e.to_string())?;
            Ok((done, stopped))
        });
        match result {
            Ok((done, stopped)) => {
                if let Some(e) = &mut self.editing {
                    e.error = None;
                    if !done.added.is_empty() {
                        e.path.update(cx, |p, cx| p.set_text("", cx));
                    }
                }
                let live_in = self.live_in(&done);
                self.reload_workspaces();
                let what = if done.added.is_empty() && done.removed.is_empty() { format!("Saved {w}.") } else { summary(w, &done) };
                self.show_toast(format!("{what}{}", edited_note(&done, &stopped, &live_in)), cx);
            }
            Err(e) => self.edit_failed(e, cx),
        }
        cx.notify();
    }

    /// `(session, folder, enabled)` for each live session that does not belong to Foreman in a folder just added.
    fn live_in(&self, done: &Edited) -> Vec<(String, String, bool)> {
        let now = foreman_claude::now_ms();
        let mut out = Vec::new();
        for f in done.workspace.folders.iter().filter(|f| done.added.contains(&f.name)) {
            for s in self.sessions.iter().filter(|s| !super::menu::is_foremans(s) && fleet::is_alive(&s.health(now)) && within(&s.cwd, &f.path)) {
                out.push((fleet::display_name(s).to_string(), f.name.clone(), f.enabled));
            }
        }
        out
    }

    fn edit_failed(&mut self, error: String, cx: &mut Context<Self>) {
        match &mut self.editing {
            Some(e) => e.error = Some(error),
            None => self.action_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn render_edit_folders(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editing = self.editing.as_ref()?;
        let w = self.workspaces.iter().find(|x| x.name == editing.workspace)?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let others: Vec<Workspace> = self.workspaces.iter().filter(|x| x.name != w.name).cloned().collect();
        let owned = owners(&others);
        let name = w.name.clone();
        let edit_button = |key: String, label: &str, edit: Edit, cx: &mut Context<Self>| {
            let w = name.clone();
            button::bordered(&self.hits, key, label.to_string(), cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.request_edit(&w, edit.clone(), cx);
            }))
        };
        let mut folders = div().flex().flex_col().gap(px(2.));
        if w.folders.is_empty() {
            folders = folders.child(div().text_xs().text_color(secondary(cx)).child("No folders yet: add one below."));
        }
        for f in &w.folders {
            let elsewhere = owned.get(&foreman_core::paths::path_key(&f.path)).filter(|_| !f.enabled).cloned();
            let is_home = w.home().is_some_and(|h| foreman_core::same_path(h, &f.path));
            let kind = if f.is_git() { f.branch.as_deref().map(|b| format!("git ({b})")).unwrap_or_else(|| "git".into()) } else { "folder".into() };
            let mut row = div()
                .id(SharedString::from(format!("edit-folder-{}", f.name)))
                .relative()
                .flex()
                .items_center()
                .gap_2()
                .px_1()
                .text_sm()
                .child(hit_target(&self.hits, format!("edit-folder-{}", f.name)));
            row = match &elsewhere {
                Some(owner) => row.child(tag(format!("supervised in {owner}"), cx).relative().child(hit_target(&self.hits, format!("edit-elsewhere-{}", f.name)))),
                None => {
                    let edit = Edit::SetEnabled { folder: f.name.clone(), enabled: !f.enabled };
                    let w = name.clone();
                    row.child(
                        div()
                            .id(SharedString::from(format!("edit-enable-{}", f.name)))
                            .relative()
                            .flex_none()
                            .cursor_pointer()
                            .child(if f.enabled { "☑" } else { "☐" })
                            .child(hit_target(&self.hits, format!("edit-enable-{}", f.name)))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.request_edit(&w, edit.clone(), cx);
                            })),
                    )
                }
            };
            row = row
                .child(div().flex_none().whitespace_nowrap().when(!f.enabled, |d| d.text_color(secondary(cx))).child(SharedString::from(f.name.clone())))
                .child(div().flex_none().text_xs().text_color(secondary(cx)).child(SharedString::from(kind)))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_xs().text_color(colors.text_muted).child(SharedString::from(f.path.display().to_string())))
                .when(is_home, |d| d.child(tag("home", cx)))
                .when(!is_home, |d| d.child(edit_button(format!("edit-home-{}", f.name), "Home", Edit::SetHome(Some(f.path.clone())), cx)))
                .child(edit_button(format!("edit-remove-{}", f.name), "Remove", Edit::RemoveFolder(f.name.clone()), cx));
            folders = folders.child(row);
        }
        let mut scans = div().flex().flex_col().gap(px(2.));
        for (i, s) in w.scans.iter().enumerate() {
            let skips = if s.exclude.is_empty() { String::new() } else { format!(" · skips {}", s.exclude.len()) };
            scans = scans.child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .text_xs()
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(SharedString::from(format!("{}  depth {}{skips}", s.path.display(), s.depth))))
                    .child(edit_button(format!("edit-scan-remove-{i}"), "Remove", Edit::RemoveScan(s.path.clone()), cx)),
            );
        }
        let home_line = match (&w.home, w.home()) {
            (Some(h), _) => format!("home {}", h.display()),
            (None, Some(first)) => format!("home: the first folder, {}", first.display()),
            (None, None) => "home: none yet (the first folder added)".into(),
        };
        let typed = |key: &'static str, label: &'static str, run: fn(&mut Self, &mut Context<Self>), cx: &mut Context<Self>| {
            button::bordered(&self.hits, key, label, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                run(this, cx);
            }))
        };
        let reset = w.home.is_some().then(|| edit_button("edit-reset-home".into(), "Reset", Edit::SetHome(None), cx));
        Some(
            div()
                .id("edit-folders")
                .relative()
                .flex()
                .flex_col()
                .gap_2()
                .mx_3()
                .my_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(colors.border_focused)
                .bg(colors.editor_background)
                .child(hit_target(&self.hits, "edit-folders".into()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(SharedString::from(format!("EDIT FOLDERS · {}", w.name)))
                        .child(typed("edit-done", "Done", |this, cx| this.close_edit_folders(cx), cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(SharedString::from(home_line)))
                        .children(reset),
                )
                .child(div().text_xs().text_color(colors.text_muted).child("FOLDERS  ☑ may have a supervised session"))
                .child(folders)
                .when(!w.scans.is_empty(), |d| d.child(div().text_xs().text_color(colors.text_muted).child("SCANS")).child(scans))
                .child(
                    div()
                        .id("edit-path")
                        .relative()
                        .child(editing.path.clone())
                        .child(hit_target(&self.hits, "edit-path".into()))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(foreman_ui::text_input::TEXT_LEFT)
                                .right(foreman_ui::text_input::TEXT_LEFT)
                                .child(hit_target(&self.hits, "edit-path-text".into())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .justify_end()
                        .child(typed("edit-add-folder", "Add folder…", |this, cx| this.add_typed(false, cx), cx))
                        .child(typed("edit-add-scan", "Add scan…", |this, cx| this.add_typed(true, cx), cx))
                        .child(typed("edit-set-home", "Set as home", |this, cx| this.set_typed_home(cx), cx)),
                )
                .child(div().text_xs().text_color(colors.text_muted).child("An empty path opens the folder picker. Enter adds the folder typed · Esc closes"))
                .children(editing.error.clone().map(|e| {
                    div()
                        .relative()
                        .px_1()
                        .rounded_sm()
                        .bg(status.error.opacity(0.14))
                        .text_xs()
                        .text_color(status.error)
                        .child(SharedString::from(e))
                        .child(hit_target(&self.hits, "edit-error".into()))
                }))
                .into_any_element(),
        )
    }
}

fn summary(w: &str, done: &Edited) -> String {
    let list = |v: Vec<&str>| v.join(", ");
    let mut parts = Vec::new();
    if !done.added.is_empty() {
        parts.push(format!("added {}", list(done.added.iter().map(String::as_str).collect())));
    }
    if !done.removed.is_empty() {
        parts.push(format!("removed {}", list(done.removed.iter().map(|f| f.name.as_str()).collect())));
    }
    format!("{w}: {}.", parts.join("; "))
}

#[cfg(test)]
mod tests {
    use foreman_core::{Folder, Scan};

    use super::*;

    fn workspace() -> Workspace {
        let mut w = Workspace::new(
            "gaia",
            r"C:\P",
            vec![Folder::new("Gaia", r"C:\P\Gaia"), Folder::new("Web", r"C:\S\Web"), Folder { added: true, ..Folder::new("Own", r"C:\S\Own") }],
        );
        w.scans.push(Scan::new(r"C:\S", 1));
        w
    }

    #[test]
    fn an_edit_stops_only_the_wanted_sessions_of_the_folders_it_takes_away() {
        let w = workspace();
        let wanted: Vec<String> = ["fm-gaia-gaia", "fm-gaia-web", "fm-gaia-own", "fm-gaia-workspace"].map(String::from).to_vec();
        assert_eq!(stops(&w, &Edit::RemoveFolder("gaia".into()), &wanted), ["fm-gaia-gaia"]);
        assert_eq!(stops(&w, &Edit::SetEnabled { folder: "Web".into(), enabled: false }, &wanted), ["fm-gaia-web"]);
        assert!(stops(&w, &Edit::SetEnabled { folder: "Web".into(), enabled: true }, &wanted).is_empty());
        assert_eq!(stops(&w, &Edit::RemoveScan(r"C:\S".into()), &wanted), ["fm-gaia-web"], "a folder added by hand stays");
        assert!(stops(&w, &Edit::RemoveFolder("Gaia".into()), &[]).is_empty(), "nothing wanted: no question");
        assert!(stops(&w, &Edit::SetHome(None), &wanted).is_empty());
    }

    #[test]
    fn the_toast_names_what_is_claimed_stopped_and_shown() {
        let done = Edited { workspace: workspace(), added: vec!["NodeApi".into()], removed: vec![], claimed: vec![("Skills".into(), "larchsys".into())] };
        assert_eq!(
            edited_note(&done, &["fm-gaia-web".into()], &[("node bindings".into(), "NodeApi".into(), false)]),
            " Skills stays supervised in larchsys, disabled here. fm-gaia-web stops (stop, never rm). node bindings in NodeApi now shows here; Enable to supervise a session of its own."
        );
        assert_eq!(edited_note(&done, &[], &[("x".into(), "NodeApi".into(), true)]), " Skills stays supervised in larchsys, disabled here. x in NodeApi now shows here.");
        assert_eq!(summary("gaia", &done), "gaia: added NodeApi.");
    }
}

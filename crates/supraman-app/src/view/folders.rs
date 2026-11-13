//! Edit folders… and Rescan: a workspace's change through [`Registry::edit`] off the UI thread.

use std::path::Path;

use gpui::{Context, Window};
use supraman_core::paths::within;
use supraman_core::{Edit, Edited, Folder, Registry, Workspace, rescan};

use super::FleetView;
use super::workspace_dialog::Mode;
use crate::fleet;

/// What the edit's toast says beyond "done".
pub fn edited_note(done: &Edited, live_in: &[(String, String)]) -> String {
    let mut out = String::new();
    for (folder, owner) in &done.claimed {
        out.push_str(&format!(" {folder} stays supervised in {owner}, disabled here."));
    }
    for (session, folder) in live_in {
        out.push_str(&format!(" {session} in {folder} now shows here."));
    }
    out
}

/// A Save refused part way: the edits applied before it, and why.
#[derive(Debug)]
pub struct Refused {
    pub applied: Option<Edited>,
    pub why: String,
}

impl From<String> for Refused {
    fn from(why: String) -> Self {
        Self { applied: None, why }
    }
}

/// Stops at the first refused edit.
pub fn edit_all(home: &Path, w: &str, edits: Vec<Edit>) -> Result<Edited, Refused> {
    let registry = Registry::at(home);
    let (mut sum, mut refused): (Option<Edited>, Option<String>) = (None, None);
    for edit in edits {
        match registry.edit(w, edit) {
            Ok(done) => {
                sum = Some(match sum {
                    None => done,
                    Some(mut s) => {
                        s.added.extend(done.added);
                        s.removed.extend(done.removed);
                        s.claimed.extend(done.claimed);
                        s.workspace = done.workspace;
                        s
                    }
                });
            }
            Err(e) => {
                refused = Some(e.to_string());
                break;
            }
        }
    }
    match (refused, sum) {
        (None, Some(sum)) => Ok(sum),
        (Some(why), sum) => Err(Refused { applied: sum, why }),
        (None, None) => Err("nothing to change".to_string().into()),
    }
}

/// What a refused Save says: what it applied before the refusal, then why.
pub fn refusal_words(w: &str, refused: &Refused) -> String {
    let Some(done) = &refused.applied else {
        return refused.why.clone();
    };
    let what = if done.added.is_empty() && done.removed.is_empty() { format!("{w}: part of the change.") } else { summary(w, done) };
    format!("Applied {what}{} Refused: {}", edited_note(done, &[]), refused.why)
}

/// What Rescan offers: the workspace, the subfolders new to it, and its folders gone from disk.
#[derive(Debug)]
pub struct Offer {
    pub workspace: Workspace,
    pub found: Vec<Folder>,
    pub gone: Vec<String>,
}

/// Changes nothing: Save applies the offer through [`FleetView::apply_edits`].
pub fn rescan_offer(home: &Path, name: &str) -> Result<Offer, String> {
    let workspace = Registry::at(home).load(name).map_err(|e| e.to_string())?;
    if workspace.scans.is_empty() {
        return Err(format!("{name} has no parent folder to look in: Edit folders… adds a folder."));
    }
    let (next, added, gone) = rescan(&workspace).map_err(|e| e.to_string())?;
    let found = next.folders.into_iter().filter(|f| added.contains(&f.name)).collect();
    Ok(Offer { workspace, found, gone })
}

impl FleetView {
    pub fn edit_folders(&mut self, workspace: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.dialogs.menu = None;
        let Some(w) = self.workspaces.iter().find(|x| x.name == workspace).cloned() else {
            return;
        };
        self.sidebar.collapsed.remove(&format!("workspace:{workspace}"));
        self.open_add(Mode::Edit(w), window, cx);
    }

    pub(super) fn rescan_workspace(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        self.scan_other_schedules(cx);
        let name = name.to_string();
        let task = cx.background_executor().spawn(async move { rescan_offer(&home, &name) });
        cx.spawn_in(window, async move |this, cx| {
            let done = task.await;
            this.update_in(cx, |this, window, cx| {
                match done {
                    Ok(offer) => this.offered(offer, window, cx),
                    Err(e) => this.toast.show_why(e, |this: &mut Self| &mut this.toast, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn offered(&mut self, offer: Offer, window: &mut Window, cx: &mut Context<Self>) {
        let note = offer_note(&offer);
        self.open_add(Mode::Edit(offer.workspace), window, cx);
        if let Some(dialog) = self.add_dialog().cloned() {
            dialog.update(cx, |d, _| {
                d.offer(offer.found, &offer.gone);
                d.note = note;
            });
        }
    }

    pub fn apply_edit(&mut self, w: &str, edit: Edit, cx: &mut Context<Self>) {
        self.apply_edits(w, vec![edit], cx);
    }

    fn editing(&self, w: &str, cx: &gpui::App) -> Option<gpui::Entity<super::workspace_dialog::WorkspaceDialog>> {
        self.add_dialog().filter(|d| matches!(&d.read(cx).mode, Mode::Edit(x) if x.name == w)).cloned()
    }

    pub fn apply_edits(&mut self, w: &str, edits: Vec<Edit>, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            self.action_error = Some("no Supraman home".into());
            return cx.notify();
        };
        if let Some(dialog) = self.editing(w, cx) {
            dialog.update(cx, |d, cx| {
                d.saving = true;
                cx.notify();
            });
        }
        let w = w.to_string();
        let task = cx.background_executor().spawn({
            let w = w.clone();
            async move { edit_all(&home, &w, edits) }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.edits_applied(&w, result, cx)).ok();
        })
        .detach();
    }

    fn edits_applied(&mut self, w: &str, result: Result<Edited, Refused>, cx: &mut Context<Self>) {
        let dialog = self.editing(w, cx);
        match result {
            Ok(done) => {
                if dialog.is_some() {
                    self.dialogs.adding = None;
                }
                let live_in = self.live_in(&done);
                if let Some(added) = done.added.first() {
                    self.sidebar.collapsed.remove(&format!("workspace:{w}"));
                    self.reveal_new(super::left::PendingReveal::Row(format!("repo:{w}/{added}")));
                }
                self.put_workspace(done.workspace.clone());
                let what = if done.added.is_empty() && done.removed.is_empty() { format!("Saved {w}.") } else { summary(w, &done) };
                self.show_toast(format!("{what}{}", edited_note(&done, &live_in)), cx);
            }
            Err(refused) => {
                let words = refusal_words(w, &refused);
                let applied = refused.applied.map(|done| done.workspace);
                if let Some(workspace) = &applied {
                    self.put_workspace(workspace.clone());
                }
                match dialog {
                    Some(dialog) => dialog.update(cx, |d, cx| {
                        d.saving = false;
                        d.error = Some(words);
                        if let (Mode::Edit(original), Some(workspace)) = (&mut d.mode, applied) {
                            *original = workspace;
                        }
                        cx.notify();
                    }),
                    None => self.action_error = Some(words),
                }
            }
        }
        self.reload_workspaces(cx);
        cx.notify();
    }

    /// `(session, folder)` for each live session that does not belong to Supraman in a folder just added.
    fn live_in(&self, done: &Edited) -> Vec<(String, String)> {
        let now = supraman_claude::now_ms();
        let mut out = Vec::new();
        for f in done.workspace.folders.iter().filter(|f| done.added.contains(&f.name)) {
            for s in self.sessions.iter().filter(|s| !super::menu::is_foremans(s) && fleet::is_alive(&s.health(now)) && within(&s.cwd, &f.path)) {
                out.push((fleet::display_name(s).to_string(), f.name.clone()));
            }
        }
        out
    }
}

fn offer_note(offer: &Offer) -> Option<String> {
    let mut parts = Vec::new();
    if !offer.gone.is_empty() {
        parts.push(format!("{} gone from disk: Save removes it.", offer.gone.join(", ")));
    }
    if offer.found.is_empty() {
        parts.push("No new subfolders: nothing to add.".to_string());
    }
    (!parts.is_empty()).then(|| parts.join(" "))
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
    use supraman_core::{Folder, FolderKind};

    use super::super::folder_list::FolderList;
    use super::super::workspace_dialog::edits_of;
    use super::*;

    #[test]
    fn the_toast_names_what_is_claimed_and_shown() {
        let done = Edited { workspace: Workspace::new("gaia", r"C:\P", vec![Folder::new("Gaia", r"C:\P\Gaia")]), added: vec!["NodeApi".into()], removed: vec![], claimed: vec![("Skills".into(), "larchsys".into())] };
        assert_eq!(edited_note(&done, &[("x".into(), "NodeApi".into())]), " Skills stays supervised in larchsys, disabled here. x in NodeApi now shows here.");
        assert_eq!(summary("gaia", &done), "gaia: added NodeApi.");
    }

    fn dir(path: &Path) -> std::path::PathBuf {
        std::fs::create_dir_all(path).unwrap();
        path.to_path_buf()
    }

    /// `demo` of the plain folders alpha and beta in `P`, scanned, home `P`.
    fn registered(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let (home, parent) = (tmp.join("home"), tmp.join("P"));
        dir(&parent.join("alpha"));
        dir(&parent.join("beta"));
        let w = supraman_core::discover(&parent, Some("demo"), 1).unwrap();
        Registry::at(&home).save(&w, false).unwrap();
        (home, parent)
    }

    fn names(w: &Workspace) -> Vec<&str> {
        w.folders.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn a_save_removes_adds_and_moves_home() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, parent) = registered(tmp.path());
        let original = Registry::at(&home).load("demo").unwrap();
        let mut list = FolderList::of_workspace(&original, &[]);
        let extra = dir(&tmp.path().join("extra"));
        list.remove(1);
        list.push(Folder { kind: FolderKind::Plain, ..Folder::new("extra", &extra) }, None);
        list.set_home(0);
        let edits = edits_of(&original, &list);
        assert_eq!(
            edits,
            [Edit::RemoveFolder("beta".into()), Edit::AddFolder { path: extra.clone(), enable: true }, Edit::SetHome(Some(parent.join("alpha")))]
        );

        let done = edit_all(&home, "demo", edits).unwrap();
        let saved = Registry::at(&home).load("demo").unwrap();
        assert_eq!(saved, done.workspace);
        assert_eq!((names(&saved), saved.home.clone()), (vec!["alpha", "extra"], Some(parent.join("alpha"))));
        assert_eq!(saved.scans[0].exclude, [parent.join("beta")], "× on a folder keeps it out of the next Rescan");
        assert!(edits_of(&saved, &FolderList::of_workspace(&saved, &[])).is_empty(), "no change, no edit");
    }

    #[test]
    fn a_save_refused_part_way_says_what_it_applied_and_the_next_save_repeats_none_of_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, parent) = registered(tmp.path());
        let original = Registry::at(&home).load("demo").unwrap();
        let mut list = FolderList::of_workspace(&original, &[]);
        list.remove(1);
        list.push(Folder { kind: FolderKind::Plain, ..Folder::new("missing", tmp.path().join("missing")) }, None);
        let refused = edit_all(&home, "demo", edits_of(&original, &list)).unwrap_err();
        let words = refusal_words("demo", &refused);
        assert!(words.starts_with("Applied demo: removed beta. Refused: "), "{words}");
        let done = refused.applied.unwrap();
        assert_eq!(names(&done.workspace), ["alpha"]);

        let shown = done.workspace;
        list.remove(1);
        list.push(Folder { kind: FolderKind::Plain, ..Folder::new("extra", dir(&tmp.path().join("extra"))) }, None);
        let again = edit_all(&home, "demo", edits_of(&shown, &list)).unwrap();
        assert_eq!(names(&again.workspace), ["alpha", "extra"]);
        assert_eq!(Registry::at(&home).load("demo").unwrap().scans[0].exclude, [parent.join("beta")]);
    }

    #[test]
    fn rescan_only_offers_new_subfolders_and_an_unticked_one_comes_back_the_next_time() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, parent) = registered(tmp.path());
        dir(&parent.join("fresh"));
        let offer = rescan_offer(&home, "demo").unwrap();
        assert_eq!(offer.found.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["fresh"]);
        assert!(offer.gone.is_empty());
        assert_eq!(names(&Registry::at(&home).load("demo").unwrap()), ["alpha", "beta"], "nothing saved before Save");

        let mut list = FolderList::of_workspace(&offer.workspace, &[]);
        list.push(offer.found[0].clone(), Some(false));
        list.remove(0);
        edit_all(&home, "demo", edits_of(&offer.workspace, &list)).unwrap();
        let saved = Registry::at(&home).load("demo").unwrap();
        assert_eq!((names(&saved), saved.scans[0].exclude.clone()), (vec!["beta"], vec![parent.join("alpha")]), "unticked: not skipped");

        let again = rescan_offer(&home, "demo").unwrap();
        assert_eq!(again.found.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["fresh"], "alpha (×) not offered, fresh again");
        let mut list = FolderList::of_workspace(&again.workspace, &[]);
        list.push(again.found[0].clone(), Some(false));
        list.toggle(1);
        edit_all(&home, "demo", edits_of(&again.workspace, &list)).unwrap();
        assert_eq!(names(&Registry::at(&home).load("demo").unwrap()), ["beta", "fresh"]);
        assert!(rescan_offer(&home, "demo").unwrap().found.is_empty(), "a folder taken is not offered again");

        std::fs::remove_dir_all(parent.join("beta")).unwrap();
        let gone = rescan_offer(&home, "demo").unwrap();
        assert_eq!(gone.gone, ["beta"]);
        assert_eq!(names(&Registry::at(&home).load("demo").unwrap()), ["beta", "fresh"], "nothing dropped before Save");
        let mut list = FolderList::of_workspace(&gone.workspace, &[]);
        list.drop_gone(&gone.gone);
        assert_eq!(list.rows.iter().map(|r| r.folder.name.as_str()).collect::<Vec<_>>(), ["fresh"], "the dialog lists beta no more");
        let edits = edits_of(&gone.workspace, &list);
        assert_eq!(edits, [Edit::DropGone(vec!["beta".into()])]);
        let done = edit_all(&home, "demo", edits).unwrap();
        assert_eq!(done.removed.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["beta"]);
        let saved = Registry::at(&home).load("demo").unwrap();
        assert_eq!((names(&saved), saved.scans[0].exclude.clone()), (vec!["fresh"], vec![parent.join("alpha")]), "gone: dropped, not skipped");

        let custom = Workspace::new("custom", tmp.path(), vec![Folder::new("x", tmp.path())]);
        Registry::at(&home).save(&custom, false).unwrap();
        assert_eq!(rescan_offer(&home, "custom").unwrap_err(), "custom has no parent folder to look in: Edit folders… adds a folder.");
    }
}

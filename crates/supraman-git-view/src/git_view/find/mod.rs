//! Ctrl+P and Ctrl+Shift+P in the Git tab: the shared palette over GitMan's commands, refs, files, history and loaded commits.

pub(crate) mod entries;
pub(crate) mod frecency;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui::{AnyElement, App, AppContext as _, Context, Entity, FocusHandle, SharedString, Subscription, Window, div, prelude::*, px};
use supraman_ui::palette::{Entry as _, Mode, Palette, PaletteEvent, Rank};
use supraman_ui::store::save_off_thread;

use super::GitView;
use super::details::FileView;
use super::diff::Target;
use super::load::GraphRow;
use super::remote::{Button, RemoteAct};
use supraman_gitman::write::remote::PullMode;
use entries::{COMMITS, Command, FILES, GitEntry, HISTORY, REFS};

/// The palette's id: rows paint under `git-find-row-<key>`.
pub(crate) const ID: &str = "git-find";

pub(crate) struct Find {
    pub(crate) palette: Entity<Palette<GitEntry>>,
    _events: Subscription,
    /// What had the keyboard when it opened.
    back: Option<FocusHandle>,
}

fn mode(key: &'static str, chip: &str, placeholder: &str, prefix: Option<char>, empty: String, rank: Rank<GitEntry>) -> Mode<GitEntry> {
    Mode { key: key.into(), chip: chip.to_string().into(), placeholder: placeholder.to_string().into(), prefix, empty: empty.into(), entries: Vec::new(), rank, loading: false }
}

fn modes(refs: Vec<GitEntry>, rows: &[GraphRow], loaded: usize) -> [Mode<GitEntry>; 4] {
    let files_empty = || "No file at HEAD matches.".to_string();
    let commits_empty = format!("No commit matches among the {loaded} loaded in the graph; scroll the graph to load more.");
    [
        Mode { entries: refs, ..mode(REFS, "Refs", "Search branches, remote branches, tags and stashes", Some('@'), "No ref matches.".into(), entries::ref_rank()) },
        Mode { loading: true, ..mode(FILES, "Files", "Search the files at HEAD to open in the viewer", Some('/'), files_empty(), entries::file_rank()) },
        Mode { loading: true, ..mode(HISTORY, "History", "Search a file to open at HEAD: GitMan has no file history yet", None, files_empty(), entries::file_rank()) },
        Mode { entries: entries::commits(rows), ..mode(COMMITS, "Commits", "Search the loaded commits by summary, author or sha", Some('#'), commits_empty, entries::commit_rank()) },
    ]
}

impl GitView {
    #[cfg(test)]
    pub(crate) fn find(&self) -> Option<&Entity<Palette<GitEntry>>> {
        self.find.as_ref().map(|f| &f.palette)
    }

    /// Opens the palette, in `mode` when given; focuses it when it is open already.
    pub fn open_find(&mut self, mode: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.load, super::Load::Ready) {
            return;
        }
        if let Some(find) = &self.find {
            find.palette.update(cx, |p, cx| {
                if let Some(mode) = mode {
                    p.enter_mode(mode, cx);
                }
                p.focus(window, cx);
            });
            return;
        }
        let back = window.focused(cx);
        let palette = self.new_palette(window, cx);
        if let Some(mode) = mode {
            palette.update(cx, |p, cx| p.enter_mode(mode, cx));
        }
        palette.update(cx, |p, cx| p.focus(window, cx));
        let events = cx.subscribe_in(&palette, window, Self::on_find);
        self.find = Some(Find { palette, _events: events, back });
        self.read_files(cx);
        if let Some(why) = frecency::take_notice(cx) {
            self.toast.show_why(why, |v: &mut GitView| &mut v.toast, cx);
        }
        cx.notify();
    }

    fn new_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Palette<GitEntry>> {
        let mut top = self.commands(cx);
        top.extend(entries::modes());
        let picks = frecency::snapshot(cx);
        let now = supraman_core::time::now_ms();
        let refs = entries::refs(&self.ref_panel.read(cx).entries(), self.ref_panel.read(cx).walk());
        let pages = self.pages.read(cx);
        let modes = modes(refs, &pages.rows, pages.commits);
        let hits = self.hits.clone();
        cx.new(|cx| {
            let mut p = Palette::new(ID, hits, top, Rc::new(entries::paint), window, cx);
            p.set_top("Search commands, or type @ refs, / files, # commits", "COMMANDS", cx);
            p.set_rank(entries::top_rank(move |key, query| picks.boost(key, query, now)), cx);
            for mode in modes {
                p.add_mode(mode);
            }
            p
        })
    }

    /// The files at HEAD, read off the UI thread for the Files and History modes.
    fn read_files(&mut self, cx: &mut Context<Self>) {
        let (Some(repo), Some(head)) = (self.repo.clone(), self.heads.head.clone()) else {
            let none = Err("This repository has no commit yet.".to_string());
            self.with_find(|p, cx| [FILES, HISTORY].iter().for_each(|m| p.set_mode_entries(m, none.clone(), cx)), cx);
            return;
        };
        let read = cx.background_spawn(async move { supraman_gitman::tree(&repo, &head).map(|files| files.into_iter().map(|f| f.path).collect::<Vec<_>>()).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let paths = read.await;
            this.update(cx, |this, cx| {
                this.with_find(|p, cx| {
                    p.set_mode_entries(FILES, paths.clone().map(|paths| entries::files(paths, false)), cx);
                    p.set_mode_entries(HISTORY, paths.map(|paths| entries::files(paths, true)), cx);
                }, cx)
            })
            .ok();
        })
        .detach();
    }

    fn with_find(&mut self, f: impl FnOnce(&mut Palette<GitEntry>, &mut Context<Palette<GitEntry>>), cx: &mut Context<Self>) {
        if let Some(find) = &self.find {
            find.palette.update(cx, f);
        }
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(find) = self.find.take() else { return };
        match find.back {
            Some(back) => window.focus(&back, cx),
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    fn on_find(&mut self, palette: &Entity<Palette<GitEntry>>, event: &PaletteEvent<GitEntry>, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaletteEvent::Query(_) => {}
            PaletteEvent::Close => self.close_find(window, cx),
            PaletteEvent::Open(entry) => {
                let (query, top) = palette.read_with(cx, |p, cx| (p.query(cx), p.mode().is_none()));
                if top {
                    let save = frecency::remember(&entry.key(), &query, supraman_core::time::now_ms(), cx);
                    save_off_thread(save, cx, |_, _, _| {});
                }
                self.close_find(window, cx);
                self.open_entry(entry.clone(), cx);
            }
        }
    }

    fn open_entry(&mut self, entry: GitEntry, cx: &mut Context<Self>) {
        match entry {
            GitEntry::Command { command, .. } => self.run_command(command, cx),
            GitEntry::Ref { sha, walked, .. } => {
                self.focus_graph = true;
                self.picked(sha.into(), walked, cx);
            }
            GitEntry::Commit { sha, .. } => {
                self.focus_graph = true;
                self.reveal_sha(sha, cx);
            }
            GitEntry::File { path, .. } => {
                let Some(head) = self.heads.head.clone() else { return };
                self.open_diff(Target::commit(head.to_string(), None, path, None), cx);
            }
            GitEntry::Mode { .. } => {}
        }
    }
}

impl GitView {
    /// The commands, each with why it cannot act now.
    fn commands(&self, cx: &App) -> Vec<GitEntry> {
        let why = |can: bool, why: &str| (!can).then(|| SharedString::from(why.to_string()));
        let remote = self.remote.read(cx);
        let dim = |button| remote.why_not(button, cx).map(SharedString::from);
        let pulls = [PullMode::FastForwardIfPossible, PullMode::FastForwardOnly, PullMode::Rebase].map(|mode| (Command::Pull(mode), dim(Button::Pull(Some(mode)))));
        let walk = self.ref_panel.read(cx).walk().clone();
        let view = self.panel.read(cx).view();
        let rows = !self.pages.read(cx).rows.is_empty();
        [
            (Command::Refresh, why(self.repo.is_some(), "The history is still loading.")),
            (Command::ToggleRefs, None),
            (Command::FilterRefs, None),
            (Command::ShowAllRefs, why(walk != Default::default(), "No ref is hidden or soloed.")),
            (Command::GoToHead, why(self.heads.head.is_some() && rows, "This repository has no commit yet.")),
            (Command::GoToUpstream, why(self.heads.upstream.is_some(), "The current branch has no upstream.")),
            (Command::GoToWip, why(self.wip_row(cx).is_some(), "The working tree is clean.")),
            if view == FileView::Tree { (Command::FilesAsPaths, None) } else { (Command::FilesAsTree, None) },
            (Command::CloseViewer, why(self.diff(cx).is_some(), "No file is open in the viewer.")),
            (Command::Push, dim(Button::Push)),
            (Command::PullDefault(remote.pull_choice()), dim(Button::Pull(None))),
        ]
        .into_iter()
        .chain(pulls)
        .chain([(Command::FetchAll, dim(Button::FetchAll))])
        .map(|(command, why_not)| GitEntry::Command { command, why_not })
        .collect()
    }

    fn run_command(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Refresh => self.refresh_graph(None, cx),
            Command::ToggleRefs => self.ref_panel.update(cx, |p, cx| p.toggle_shut(cx)),
            Command::FilterRefs => self.ref_panel.update(cx, |p, cx| p.focus_filter(cx)),
            Command::ShowAllRefs => self.ref_panel.update(cx, |p, cx| p.show_all(cx)),
            Command::GoToHead => self.go_to(self.heads.head.clone(), cx),
            Command::GoToUpstream => self.go_to(self.heads.upstream.clone(), cx),
            Command::GoToWip => self.select_wip(cx),
            Command::FilesAsTree => self.set_file_view(FileView::Tree, cx),
            Command::FilesAsPaths => self.set_file_view(FileView::Path, cx),
            Command::CloseViewer => self.back_to_graph(cx),
            Command::Push => self.remote.update(cx, |r, cx| r.push_head(cx)),
            Command::PullDefault(_) => self.remote.update(cx, |r, cx| r.pull_default(cx)),
            Command::Pull(mode) => self.remote.update(cx, |r, cx| r.act(RemoteAct::Pull(mode), cx)),
            Command::FetchAll => self.remote.update(cx, |r, cx| r.act(RemoteAct::FetchAll, cx)),
        }
        if !matches!(command, Command::FilterRefs | Command::CloseViewer) {
            self.focus_graph = self.diff(cx).is_none();
        }
    }

    pub(super) fn render_find(&self) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        Some(div().absolute().top_0().left_0().right_0().flex().justify_center().pt(px(48.)).px_4().child(find.palette.clone()).into_any_element())
    }
}

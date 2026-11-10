//! A file's change in the graph's place, on Zed's editor (`supraman_editor`), read-only and read off the UI thread.

mod image;
mod invisible;
mod load;
mod picks;
mod render;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, KeyBinding, Window};
use supraman_editor::{Diff, SplitDiff, ZedEditor};
use supraman_gitman::Repo;
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;

pub(crate) use load::{MAX_SHOWN_BYTES, Of, Sizes, TooLarge};
pub use load::Target;
pub(crate) use picks::{PickEvent, Picks};
#[cfg(test)]
pub(crate) use picks::{Act, Line};
#[cfg(test)]
pub(crate) use picks::hunk_key;

gpui::actions!(git_diff, [NextChange, PreviousChange, CloseDiff, PreviousFile, NextFile, FirstFile, LastFile]);

pub(crate) const KEY_CONTEXT: &str = "GitDiff";

pub(super) fn bind_keys(cx: &mut App) {
    let c = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("f7", NextChange, c),
        KeyBinding::new("alt-down", NextChange, c),
        KeyBinding::new("shift-f7", PreviousChange, c),
        KeyBinding::new("alt-up", PreviousChange, c),
        KeyBinding::new("escape", CloseDiff, c),
        KeyBinding::new("left", CloseDiff, c),
        KeyBinding::new("up", PreviousFile, c),
        KeyBinding::new("k", PreviousFile, c),
        KeyBinding::new("down", NextFile, c),
        KeyBinding::new("j", NextFile, c),
        KeyBinding::new("ctrl-up", FirstFile, c),
        KeyBinding::new("ctrl-home", FirstFile, c),
        KeyBinding::new("ctrl-down", LastFile, c),
        KeyBinding::new("ctrl-end", LastFile, c),
    ]);
}

/// Which file of the panel's list to show next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileStep {
    /// The one before, the last from the first.
    Previous,
    /// The one after, the first from the last.
    Next,
    First,
    Last,
}

impl FileStep {
    /// The index `self` leads to from `at` in a list of `len`; `None` for an empty list.
    pub(crate) fn index(self, len: usize, at: Option<usize>) -> Option<usize> {
        let last = len.checked_sub(1)?;
        Some(match (self, at) {
            (FileStep::First, _) | (FileStep::Next, None) => 0,
            (FileStep::Last, _) | (FileStep::Previous, None) => last,
            (FileStep::Next, Some(i)) => if i >= last { 0 } else { i + 1 },
            (FileStep::Previous, Some(i)) => if i == 0 { last } else { i - 1 },
        })
    }
}

/// The viewer asks its owner for another file of the list.
pub(crate) struct Go(pub FileStep);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mode {
    File,
    #[default]
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Layout {
    #[default]
    Split,
    Inline,
}

/// The viewer asks its owner to close it.
pub(crate) struct Close;

pub(crate) struct Text {
    pub(crate) diff: Diff,
    /// One side's encoding, or `old → new`.
    pub(crate) encoding: String,
    /// What changed when only bytes the text does not show did.
    pub(crate) invisible: Vec<String>,
    /// The old and new texts the diff was computed from.
    shown: (String, String),
    /// The editors' scroll subscriptions that repaint the picks.
    watching: Vec<gpui::Subscription>,
    file: Option<Entity<ZedEditor>>,
    inline: Option<Entity<ZedEditor>>,
    split: Option<Entity<SplitDiff>>,
    /// The top rows of the editors a reload replaced, each set on its editor as it is made.
    place: Place,
}

#[derive(Debug, Clone, Copy, Default)]
struct Place {
    file: Option<f64>,
    inline: Option<f64>,
    split: Option<f64>,
}

impl Text {
    fn place(&self, cx: &mut App) -> Place {
        let split = self.split.as_ref().map(|s| s.read(cx).new.clone());
        let row = |editor: Option<&Entity<ZedEditor>>, cx: &mut App| editor.map(|e| supraman_editor::scroll_row(e, cx));
        Place { file: row(self.file.as_ref(), cx), inline: row(self.inline.as_ref(), cx), split: row(split.as_ref(), cx) }
    }
}

pub(crate) enum Content {
    Loading,
    Failed(String),
    Binary(Sizes),
    TooLarge(TooLarge),
    Image(Box<image::Images>),
    Text(Box<Text>),
}

pub(crate) struct DiffView {
    hits: HitTargets,
    pub(crate) target: Target,
    mode: Mode,
    layout: Layout,
    content: Content,
    /// The change last moved to with previous/next.
    current: Option<usize>,
    /// The first read came in and its first change is not yet scrolled to; a reload keeps the place.
    to_first: bool,
    /// Bumped by each read; a read that finishes under another one is dropped.
    token: u64,
    focus: FocusHandle,
    /// The line picks of a working-tree diff, once the tab offers them.
    picks: Option<Picks>,
    /// The other worktree whose file this is, read from its folder.
    pub(crate) worktree: Option<Arc<Path>>,
}

impl EventEmitter<Close> for DiffView {}
impl EventEmitter<Go> for DiffView {}

impl DiffView {
    pub(crate) fn open(repo: Repo, target: Target, mode: Mode, layout: Layout, hits: HitTargets, cx: &mut App) -> Entity<DiffView> {
        cx.new(|cx| {
            let mode = if target.of == Of::Conflicted { Mode::File } else { mode };
            let mut view = DiffView { hits, target, mode, layout, content: Content::Loading, current: None, to_first: false, token: 0, focus: cx.focus_handle(), picks: None, worktree: None };
            view.reload(repo, cx);
            view
        })
    }

    /// Reads the file's two sides again off the UI thread; what is shown stays until they are in.
    pub(crate) fn reload(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.read_pick_tools(repo.clone(), cx);
        self.token += 1;
        let (token, job, svg) = (self.token, self.target.clone(), cx.svg_renderer());
        let loaded = cx.background_spawn(async move { load::load(&repo, &job, &svg) });
        cx.spawn(async move |this: gpui::WeakEntity<DiffView>, cx| {
            let loaded = loaded.await;
            let diff = this.update(cx, |this, cx| if this.token == token { this.apply(loaded, cx) } else { None }).ok().flatten();
            if let Some((diff, encoding, invisible, shown)) = diff {
                let diff = diff.await;
                this.update(cx, |this, cx| if this.token == token { this.show_read(diff, encoding, invisible, shown, cx) }).ok();
            }
        })
        .detach();
    }

    #[allow(clippy::type_complexity)]
    fn apply(&mut self, loaded: Result<load::Loaded, String>, cx: &mut Context<Self>) -> Option<(gpui::Task<Diff>, String, Vec<String>, (String, String))> {
        let _stall = stall::section("diff view: buffers");
        self.content = match loaded {
            Ok(load::Loaded::Text { old, new, encoding, invisible, language }) => {
                let shown = (old.clone(), new.clone());
                return Some((Diff::compute(old, new, language, cx), encoding, invisible, shown));
            }
            Ok(load::Loaded::Binary(sizes)) => Content::Binary(sizes),
            Ok(load::Loaded::TooLarge(why)) => Content::TooLarge(why),
            Ok(load::Loaded::Image(images)) => Content::Image(images),
            Err(why) => Content::Failed(why),
        };
        self.current = None;
        cx.notify();
        None
    }

    /// [`DiffView::show`] of a read, with the texts the diff was computed from.
    fn show_read(&mut self, diff: Diff, encoding: String, invisible: Vec<String>, shown: (String, String), cx: &mut Context<Self>) {
        self.show(diff, encoding, invisible, cx);
        if let Some(text) = self.text_mut() {
            text.shown = shown;
        }
    }

    /// The text read; a reload of a text keeps its editors' top rows and the current change.
    fn show(&mut self, diff: Diff, encoding: String, invisible: Vec<String>, cx: &mut Context<Self>) {
        let place = match &self.content {
            Content::Text(text) => {
                self.current = self.current.filter(|c| *c < diff.hunks.len());
                text.place(cx)
            }
            _ => {
                self.current = None;
                Place::default()
            }
        };
        self.to_first = matches!(self.content, Content::Loading);
        if let Some(picks) = &mut self.picks {
            picks.forget_rows();
        }
        self.content = Content::Text(Box::new(Text { diff, encoding, invisible, shown: Default::default(), watching: Vec::new(), file: None, inline: None, split: None, place }));
        cx.notify();
    }

    /// What the viewer shows, in words, for the selftest's probe.
    pub(crate) fn shown(&self) -> String {
        let size = |s: Option<usize>| s.map_or("none".to_string(), |b| supraman_core::text::byte_size(b as u64));
        let picture = |p: &Option<image::Picture>| p.as_ref().map_or("none".to_string(), |p| format!("{}×{}", p.width, p.height));
        match &self.content {
            Content::Loading => "loading".into(),
            Content::Failed(why) => format!("failed: {why}"),
            Content::Binary(s) => format!("binary {} → {}", size(s.old), size(s.new)),
            Content::TooLarge(TooLarge::Bytes(s)) => format!("too large {} → {}", size(s.old), size(s.new)),
            Content::TooLarge(TooLarge::Pixels { old, new }) => {
                let px = |p: &Option<(u32, u32)>| p.map_or("none".to_string(), |(w, h)| format!("{w}×{h}"));
                format!("too large {} → {}", px(old), px(new))
            }
            Content::Image(i) => format!("image {} → {}", picture(&i.old), picture(&i.new)),
            Content::Text(t) if !t.invisible.is_empty() => format!("same text: {}", t.invisible.join("; ")),
            Content::Text(_) => "text".into(),
        }
    }

    /// Where this file is read: its worktree's folder, else the tab's `repo`.
    pub(crate) fn repo_of(&self, repo: &Repo) -> Repo {
        match &self.worktree {
            Some(path) => Repo::new(repo.git().clone(), path.to_path_buf()),
            None => repo.clone(),
        }
    }

    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    pub(crate) fn layout(&self) -> Layout {
        self.layout
    }

    #[cfg(test)]
    pub(crate) fn current(&self) -> Option<usize> {
        self.current
    }

    #[cfg(test)]
    pub(crate) fn content(&self) -> &Content {
        &self.content
    }

    pub(crate) fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub(crate) fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    pub(crate) fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.current = None;
        cx.notify();
    }

    pub(crate) fn hunk_count(&self) -> usize {
        match &self.content {
            Content::Text(t) => t.diff.hunks.len(),
            _ => 0,
        }
    }

    /// Moves to the change after the current one, the first when none is.
    pub(crate) fn next_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.hunk_count();
        if count == 0 {
            return;
        }
        let ix = self.current.map_or(0, |c| (c + 1).min(count - 1));
        self.go_to(ix, window, cx);
    }

    /// Moves to the change before the current one, the last when none is.
    pub(crate) fn previous_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.hunk_count();
        if count == 0 {
            return;
        }
        let ix = self.current.map_or(count - 1, |c| c.saturating_sub(1));
        self.go_to(ix, window, cx);
    }

    /// Scrolls to the first change once after the first read came in, while Diff View shows.
    pub(super) fn first_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Diff || !std::mem::take(&mut self.to_first) {
            return;
        }
        if self.hunk_count() > 0 {
            self.go_to(0, window, cx);
        }
    }

    fn go_to(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.current = Some(ix);
        self.mode = Mode::Diff;
        let Some(hunk) = self.text().and_then(|t| t.diff.hunks.get(ix).cloned()) else { return };
        match self.layout {
            Layout::Inline => {
                let editor = self.inline_editor(window, cx);
                supraman_editor::select_row(&editor, hunk.new_rows.start, true, window, cx);
            }
            Layout::Split => {
                let split = self.split(window, cx);
                let (old, new) = split.read_with(cx, |s, _| (s.old.clone(), s.new.clone()));
                let new_side = !hunk.new_rows.is_empty();
                supraman_editor::select_row(&old, hunk.old_rows.start, !new_side, window, cx);
                supraman_editor::select_row(&new, hunk.new_rows.start, new_side, window, cx);
            }
        }
        cx.notify();
    }

    fn text(&self) -> Option<&Text> {
        match &self.content {
            Content::Text(t) => Some(t),
            _ => None,
        }
    }

    fn text_mut(&mut self) -> Option<&mut Text> {
        match &mut self.content {
            Content::Text(t) => Some(t),
            _ => None,
        }
    }

    /// The editor of File View, made on first use: the file at the commit, before it for a deleted one.
    pub(crate) fn file_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<ZedEditor>> {
        let deleted = self.target.kind() == Some(supraman_gitman::ChangeKind::Deleted);
        let text = self.text_mut()?;
        if let Some(editor) = &text.file {
            return Some(editor.clone());
        }
        let buffer = if deleted { text.diff.old.clone() } else { text.diff.new.clone() };
        let editor = supraman_editor::file_editor(buffer, window, cx);
        text.file = Some(editor.clone());
        if let Some(row) = text.place.file.take() {
            supraman_editor::set_scroll_row(&editor, row, window, cx);
        }
        Some(editor)
    }

    pub(crate) fn inline_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<ZedEditor> {
        let text = self.text_mut().expect("a diff is shown");
        if let Some(editor) = &text.inline {
            return editor.clone();
        }
        let editor = supraman_editor::inline_editor(&text.diff, window, cx);
        text.inline = Some(editor.clone());
        if let Some(row) = text.place.inline.take() {
            supraman_editor::set_scroll_row(&editor, row, window, cx);
        }
        editor
    }

    pub(crate) fn split(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<SplitDiff> {
        let text = self.text_mut().expect("a diff is shown");
        if let Some(split) = &text.split {
            return split.clone();
        }
        let diff = text.diff.clone();
        let split = cx.new(|cx| SplitDiff::new(&diff, window, cx));
        text.split = Some(split.clone());
        if let Some(row) = text.place.split.take() {
            let (old, new) = split.read_with(cx, |s, _| (s.old.clone(), s.new.clone()));
            supraman_editor::set_scroll_row(&new, row, window, cx);
            supraman_editor::set_scroll_row(&old, row, window, cx);
        }
        split
    }
}

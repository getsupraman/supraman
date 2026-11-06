//! A file's change in the graph's place: the file at the commit, or its diff against the parent split or
//! inline, on Zed's editor (`supraman_editor`), read-only (C146), read off the UI thread.

mod image;
mod load;
mod render;

#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod tests;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, KeyBinding, Window};
use supraman_editor::{Diff, SplitDiff, ZedEditor};
use supraman_gitman::Repo;
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;

pub use load::{Of, Sizes, Target};

gpui::actions!(git_diff, [NextChange, PreviousChange, CloseDiff, PreviousFile, NextFile, FirstFile, LastFile]);

pub const KEY_CONTEXT: &str = "GitDiff";

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
pub enum FileStep {
    /// The one before, the last from the first.
    Previous,
    /// The one after, the first from the last.
    Next,
    First,
    Last,
}

impl FileStep {
    /// The index `self` leads to from `at` in a list of `len`; `None` for an empty list.
    pub fn index(self, len: usize, at: Option<usize>) -> Option<usize> {
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
pub struct Go(pub FileStep);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    File,
    #[default]
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Split,
    Inline,
}

/// The viewer asks its owner to close it.
pub struct Close;

pub(crate) struct Text {
    pub diff: Diff,
    pub encoding: String,
    file: Option<Entity<ZedEditor>>,
    inline: Option<Entity<ZedEditor>>,
    split: Option<Entity<SplitDiff>>,
}

pub(crate) enum Content {
    Loading,
    Failed(String),
    Binary(Sizes),
    Image(Box<image::Images>),
    Text(Box<Text>),
}

pub struct DiffView {
    hits: HitTargets,
    pub target: Target,
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
}

impl EventEmitter<Close> for DiffView {}
impl EventEmitter<Go> for DiffView {}

impl DiffView {
    pub fn open(repo: Repo, target: Target, mode: Mode, layout: Layout, hits: HitTargets, cx: &mut App) -> Entity<DiffView> {
        cx.new(|cx| {
            let mode = if target.of == Of::Conflicted { Mode::File } else { mode };
            let mut view = DiffView { hits, target, mode, layout, content: Content::Loading, current: None, to_first: false, token: 0, focus: cx.focus_handle() };
            view.reload(repo, cx);
            view
        })
    }

    /// Reads the file's two sides again off the UI thread; what is shown stays until they are in.
    pub fn reload(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.token += 1;
        let (token, job, svg) = (self.token, self.target.clone(), cx.svg_renderer());
        let loaded = cx.background_spawn(async move { load::load(&repo, &job, &svg) });
        cx.spawn(async move |this: gpui::WeakEntity<DiffView>, cx| {
            let loaded = loaded.await;
            let diff = this.update(cx, |this, cx| if this.token == token { this.apply(loaded, cx) } else { None }).ok().flatten();
            if let Some((diff, encoding)) = diff {
                let diff = diff.await;
                this.update(cx, |this, cx| if this.token == token { this.show(diff, encoding, cx) }).ok();
            }
        })
        .detach();
    }

    fn apply(&mut self, loaded: Result<load::Loaded, String>, cx: &mut Context<Self>) -> Option<(gpui::Task<Diff>, String)> {
        let _stall = stall::section("diff view: buffers");
        self.content = match loaded {
            Ok(load::Loaded::Text { old, new, encoding, language }) => return Some((Diff::compute(old, new, language, cx), encoding)),
            Ok(load::Loaded::Binary(sizes)) => Content::Binary(sizes),
            Ok(load::Loaded::Image(images)) => Content::Image(images),
            Err(why) => Content::Failed(why),
        };
        self.current = None;
        cx.notify();
        None
    }

    fn show(&mut self, diff: Diff, encoding: String, cx: &mut Context<Self>) {
        self.current = None;
        self.to_first = matches!(self.content, Content::Loading);
        self.content = Content::Text(Box::new(Text { diff, encoding, file: None, inline: None, split: None }));
        cx.notify();
    }

    /// What the viewer shows, in words, for the selftest's probe.
    pub fn shown(&self) -> String {
        let size = |s: Option<usize>| s.map_or("none".to_string(), |b| supraman_core::text::byte_size(b as u64));
        let picture = |p: &Option<image::Picture>| p.as_ref().map_or("none".to_string(), |p| format!("{}×{}", p.width, p.height));
        match &self.content {
            Content::Loading => "loading".into(),
            Content::Failed(why) => format!("failed: {why}"),
            Content::Binary(s) => format!("binary {} → {}", size(s.old), size(s.new)),
            Content::Image(i) => format!("image {} → {}", picture(&i.old), picture(&i.new)),
            Content::Text(_) => "text".into(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    #[cfg(test)]
    pub(crate) fn content(&self) -> &Content {
        &self.content
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.current = None;
        cx.notify();
    }

    pub fn hunk_count(&self) -> usize {
        match &self.content {
            Content::Text(t) => t.diff.hunks.len(),
            _ => 0,
        }
    }

    /// Moves to the change after the current one, the first when none is.
    pub fn next_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.hunk_count();
        if count == 0 {
            return;
        }
        let ix = self.current.map_or(0, |c| (c + 1).min(count - 1));
        self.go_to(ix, window, cx);
    }

    /// Moves to the change before the current one, the last when none is.
    pub fn previous_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub fn file_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<ZedEditor>> {
        let deleted = self.target.kind() == Some(supraman_gitman::ChangeKind::Deleted);
        let text = self.text_mut()?;
        let buffer = if deleted { text.diff.old.clone() } else { text.diff.new.clone() };
        Some(text.file.get_or_insert_with(|| supraman_editor::file_editor(buffer, window, cx)).clone())
    }

    pub fn inline_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<ZedEditor> {
        let text = self.text_mut().expect("a diff is shown");
        let diff = text.diff.clone();
        text.inline.get_or_insert_with(|| supraman_editor::inline_editor(&diff, window, cx)).clone()
    }

    pub fn split(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<SplitDiff> {
        let text = self.text_mut().expect("a diff is shown");
        let diff = text.diff.clone();
        text.split.get_or_insert_with(|| cx.new(|cx| SplitDiff::new(&diff, window, cx))).clone()
    }
}

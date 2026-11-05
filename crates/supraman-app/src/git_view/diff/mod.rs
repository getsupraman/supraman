//! A file's change in the graph's place: the file at the commit, or its diff against the parent split or
//! inline, on Zed's editor (`supraman_editor`), read-only (C146), read off the UI thread.

mod load;
mod render;

#[cfg(test)]
mod tests;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, KeyBinding, Window};
use supraman_core::text::Encoding;
use supraman_editor::{Diff, SplitDiff, ZedEditor};
use supraman_gitman::Repo;
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;

pub use load::{Of, Target};

gpui::actions!(git_diff, [NextChange, PreviousChange, CloseDiff]);

pub const KEY_CONTEXT: &str = "GitDiff";

pub fn bind_keys(cx: &mut App) {
    let c = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("f7", NextChange, c),
        KeyBinding::new("alt-down", NextChange, c),
        KeyBinding::new("shift-f7", PreviousChange, c),
        KeyBinding::new("alt-up", PreviousChange, c),
        KeyBinding::new("escape", CloseDiff, c),
    ]);
}

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
    pub encoding: Encoding,
    file: Option<Entity<ZedEditor>>,
    inline: Option<Entity<ZedEditor>>,
    split: Option<Entity<SplitDiff>>,
}

pub(crate) enum Content {
    Loading,
    Failed(String),
    Binary,
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
    /// Bumped by each read; a read that finishes under another one is dropped.
    token: u64,
    focus: FocusHandle,
}

impl EventEmitter<Close> for DiffView {}

impl DiffView {
    pub fn open(repo: Repo, target: Target, mode: Mode, layout: Layout, hits: HitTargets, cx: &mut App) -> Entity<DiffView> {
        cx.new(|cx| {
            let mode = if target.of == Of::Conflicted { Mode::File } else { mode };
            let mut view = DiffView { hits, target, mode, layout, content: Content::Loading, current: None, token: 0, focus: cx.focus_handle() };
            view.reload(repo, cx);
            view
        })
    }

    /// Reads the file's two sides again off the UI thread; what is shown stays until they are in.
    pub fn reload(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.token += 1;
        let (token, job) = (self.token, self.target.clone());
        let loaded = cx.background_spawn(async move { load::load(&repo, &job) });
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

    fn apply(&mut self, loaded: Result<load::Loaded, String>, cx: &mut Context<Self>) -> Option<(gpui::Task<Diff>, Encoding)> {
        let _stall = stall::section("diff view: buffers");
        match loaded {
            Ok(load::Loaded::Text { old, new, encoding, language }) => Some((Diff::compute(old, new, language, cx), encoding)),
            Ok(load::Loaded::Binary) => {
                self.content = Content::Binary;
                cx.notify();
                None
            }
            Err(why) => {
                self.content = Content::Failed(why);
                cx.notify();
                None
            }
        }
    }

    fn show(&mut self, diff: Diff, encoding: Encoding, cx: &mut Context<Self>) {
        self.current = None;
        self.content = Content::Text(Box::new(Text { diff, encoding, file: None, inline: None, split: None }));
        cx.notify();
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

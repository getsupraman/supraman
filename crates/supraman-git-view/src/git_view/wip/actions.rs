//! The WIP panel's writes through the tab's write gate; their outcome goes to the tab as a [`PanelEvent`].

use supraman_core::text::count;

use gpui::{App, Context};
use supraman_gitman::write::conflicts::{Marked, Sequence, Side};
use supraman_gitman::write::rebase::{Autostash, Rebased};
use supraman_gitman::write::wip::{self, Step};
use supraman_gitman::write::{LockSet, Resource, Session, WriteError};
use supraman_gitman::{ChangeKind, FileChange};
use supraman_ui::confirm::Confirm;

use super::compose::{self, Act};
use super::{Section, WipPanel, sections};
use crate::git_view::details::reword::kept_words;
use crate::git_view::details::{FileView, PanelEvent};
use crate::git_view::write_gate::{Verb, Written};
use supraman_core::text::capitalised;

/// A write the panel asks about first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ask {
    Discard(Vec<FileChange>),
    DiscardAll,
    Step(Step),
    /// Mark resolved of these files (`None`: every conflicted one), whose `markers` files still hold conflict markers.
    ResolveAnyway { paths: Option<Vec<String>>, markers: Vec<String> },
}

/// What a step's or a mark resolved's job left for [`WipPanel::written`].
pub(crate) enum Kept {
    Stepped(Rebased),
    Marked(Marked),
}

/// The panel's write under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Doing {
    Stage,
    Unstage,
    Discard(usize),
    DiscardAll,
    Commit { summary: String, amend: bool },
    Step(Step),
    Take(Side),
    Resolve { paths: Option<Vec<String>>, anyway: bool },
    Unresolve,
}

impl Verb for Doing {
    fn running(&self) -> String {
        match self {
            Doing::Stage => "Staging".into(),
            Doing::Unstage => "Unstaging".into(),
            Doing::Discard(n) => format!("Discarding the changes to {}", count(*n, "file", "files")),
            Doing::DiscardAll => "Discarding all changes".into(),
            Doing::Commit { amend: true, .. } => "Amending".into(),
            Doing::Commit { .. } => "Committing".into(),
            Doing::Step(step) => match step {
                Step::ContinueRebase => "Continuing the rebase".into(),
                Step::SkipCommit => "Skipping the commit".into(),
                Step::AbortRebase => "Aborting the rebase".into(),
                Step::AbortMerge => "Aborting the merge".into(),
                Step::Continue(seq) => format!("Continuing the {}", noun(*seq)),
                Step::Skip(seq) => format!("Skipping the {}'s commit", noun(*seq)),
                Step::Abort(seq) => format!("Aborting the {}", noun(*seq)),
            },
            Doing::Take(_) => "Taking a side".into(),
            Doing::Resolve { .. } => "Marking resolved".into(),
            Doing::Unresolve => "Marking conflicted".into(),
        }
    }
}

pub(crate) const ANOTHER_CONFLICT: &str = "The rebase stopped on another conflict";

/// A cherry-pick's, revert's or am's name in the panel's words.
pub(crate) fn noun(sequence: Sequence) -> &'static str {
    match sequence {
        Sequence::CherryPick => "cherry-pick",
        Sequence::Revert => "revert",
        Sequence::Am => "patch series",
    }
}
pub(crate) const ABORTED_POPPED: &str = "Aborted the rebase; your changes are back";

impl Ask {
    pub(crate) fn question(&self) -> (String, &'static str) {
        match self {
            Ask::Discard(files) => (format!("Discard the changes to {}? You can undo it.", count(files.len(), "file", "files")), "Discard"),
            Ask::DiscardAll => ("Discard all changes? Staged, unstaged and new untracked files go; ignored files stay. You can undo it.".into(), "Discard all"),
            Ask::Step(Step::SkipCommit) => ("Skip this commit of the rebase? Its changes are left out.".into(), "Skip commit"),
            Ask::Step(Step::AbortRebase) => ("Abort the rebase? The branch goes back to where it started and the resolutions so far are lost.".into(), "Abort rebase"),
            Ask::Step(Step::AbortMerge) => ("Abort the merge? The resolutions so far are lost.".into(), "Abort merge"),
            Ask::Step(Step::ContinueRebase) => ("Continue the rebase?".into(), "Continue rebase"),
            Ask::Step(Step::Continue(seq)) => (format!("Continue the {}?", noun(*seq)), "Continue"),
            Ask::Step(Step::Skip(seq)) => (format!("Skip this commit of the {}? Its changes are left out.", noun(*seq)), "Skip commit"),
            Ask::Step(Step::Abort(seq)) => (format!("Abort the {}? The branch goes back to where it started and the resolutions so far are lost.", noun(*seq)), "Abort"),
            Ask::ResolveAnyway { markers, .. } => (markers_question(markers), "Mark resolved"),
        }
    }
}

/// `<file> still has conflict markers. Mark it resolved anyway?`, or the first file and how many more.
fn markers_question(markers: &[String]) -> String {
    match markers {
        [one] => format!("{one} still has conflict markers. Mark it resolved anyway?"),
        [first, rest @ ..] => format!("{first} and {} more still have conflict markers. Mark them resolved anyway?", count(rest.len(), "file", "files")),
        [] => "Mark resolved anyway?".into(),
    }
}

/// Stages every unstaged file the status lists now but the conflicted ones, which only mark resolved stages.
fn stage_everything(s: &Session) -> Result<(), WriteError> {
    let status = supraman_gitman::status(s.writer().repo())?;
    let files: Vec<FileChange> = sections(&status).0.into_iter().filter(|f| f.kind != ChangeKind::Unmerged).collect();
    wip::stage(s, &files)
}

pub(super) fn index_write() -> LockSet {
    LockSet::new().write(Resource::Index)
}

impl WipPanel {
    /// Why no write can start now, from the tab's gate.
    pub(crate) fn busy(&self, cx: &App) -> Option<&'static str> {
        self.writes.busy(cx)
    }

    pub(super) fn request(&mut self, doing: Doing, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<(), WriteError> + Send + 'static, cx: &mut Context<Self>) {
        if let Err(why) = self.writes.request(doing, locks, job, cx) {
            cx.emit(PanelEvent::Why(why.into()));
        }
    }

    pub(crate) fn stage(&mut self, files: Vec<FileChange>, cx: &mut Context<Self>) {
        if !files.is_empty() {
            self.request(Doing::Stage, index_write(), move |s| wip::stage(s, &files), cx);
        }
    }

    pub(crate) fn unstage(&mut self, files: Vec<FileChange>, cx: &mut Context<Self>) {
        if !files.is_empty() {
            self.request(Doing::Unstage, index_write(), move |s| wip::unstage(s, &files), cx);
        }
    }

    /// Every unstaged file, as the status lists them when the write runs.
    pub(crate) fn stage_all(&mut self, cx: &mut Context<Self>) {
        self.request(Doing::Stage, index_write(), stage_everything, cx);
    }

    pub(crate) fn unstage_all(&mut self, cx: &mut Context<Self>) {
        self.request(Doing::Unstage, index_write(), |s| wip::unstage(s, &sections(&supraman_gitman::status(s.writer().repo())?).1), cx);
    }

    pub(crate) fn selected_file(&self, section: Section) -> Option<FileChange> {
        let shown = self.shown()?;
        let list = self.list(section);
        let files = shown.files(section);
        match self.view {
            FileView::Path => list.paths.selected().and_then(|ix| files.get(ix).cloned()),
            FileView::Tree => list.tree.selected().and_then(|p| files.iter().find(|f| f.path == p.as_ref()).cloned()),
        }
    }

    pub(crate) fn stage_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(file) = self.selected_file(Section::Unstaged) {
            self.stage(vec![file], cx);
        }
    }

    pub(crate) fn unstage_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(file) = self.selected_file(Section::Staged) {
            self.unstage(vec![file], cx);
        }
    }

    /// Asks before `ask` runs; nothing is asked while a write waits or runs.
    pub(crate) fn ask(&mut self, ask: Ask, cx: &mut Context<Self>) {
        let (text, yes) = ask.question();
        let destroys = matches!(ask, Ask::Discard(_) | Ask::DiscardAll);
        let asked = Confirm::ask(text, yes, ask);
        self.writes.ask(if destroys { asked.danger() } else { asked }, cx);
    }

    pub(crate) fn answer(&mut self, yes: bool, cx: &mut Context<Self>) {
        let Some(ask) = self.writes.answer(yes) else { return cx.notify() };
        match ask {
            Ask::Discard(files) => self.request(Doing::Discard(files.len()), LockSet::reset(), move |s| wip::discard_unstaged(s, &files), cx),
            Ask::DiscardAll => self.request(Doing::DiscardAll, LockSet::reset(), |s| wip::discard_all(s).map(drop), cx),
            Ask::Step(step) => self.step(step, false, cx),
            Ask::ResolveAnyway { paths, .. } => self.resolve(paths, true, cx),
        }
    }

    #[cfg(test)]
    pub(crate) fn asking(&self) -> Option<&str> {
        self.writes.asked()
    }

    /// The commit button's words and act now; `stage_all` counts the unstaged files as staged.
    pub(crate) fn commit_button(&self, stage_all: bool, cx: &App) -> Option<(String, Option<Act>)> {
        let (shown, c) = (self.shown()?, self.composer.as_ref()?);
        let staged = shown.staged.len() + if stage_all { shown.unstaged.len() } else { 0 };
        let has_message = !c.summary.read(cx).text().trim().is_empty();
        if matches!(shown.state, supraman_gitman::OperationState::Rebase { external: true }) {
            return None;
        }
        Some(compose::button(shown.state, staged, c.amend, has_message))
    }

    /// What the commit button does, `stage_all` staging every file first; says why when it cannot.
    pub(crate) fn commit(&mut self, stage_all: bool, cx: &mut Context<Self>) {
        let Some((words, act)) = self.commit_button(stage_all, cx) else { return };
        let Some(act) = act else { return cx.emit(PanelEvent::Why(words)) };
        let Some(c) = &self.composer else { return };
        let (summary, description) = c.texts(cx);
        let (amend, no_verify) = (c.amend, self.skips_hooks());
        match act {
            Act::Commit => {
                let message = wip::message(&summary, &description);
                let doing = Doing::Commit { summary: summary.trim().to_string(), amend };
                self.request(doing, LockSet::commit(), move |s| {
                    if stage_all {
                        stage_everything(s)?;
                    }
                    wip::commit(s, &message, amend, no_verify)
                }, cx);
            }
            Act::ContinueRebase => self.step(Step::ContinueRebase, stage_all, cx),
            Act::SkipCommit => self.ask(Ask::Step(Step::SkipCommit), cx),
            Act::Continue(seq) => self.step(Step::Continue(seq), stage_all, cx),
            Act::Skip(seq) => self.ask(Ask::Step(Step::Skip(seq)), cx),
        }
    }

    /// `step` through the gate, `stage_all` staging every file first; how it ended kept for [`Self::written`].
    fn step(&mut self, step: Step, stage_all: bool, cx: &mut Context<Self>) {
        let job = self.writes.keeping(Kept::Stepped, move |s| {
            if stage_all {
                stage_everything(s)?;
            }
            wip::step(s, step)
        });
        self.request(Doing::Step(step), LockSet::rebase(), job, cx);
    }

    /// A rebase's step done: once finished, the new HEAD selected with its autostash's outcome; else [`PanelEvent::Wrote`]'s toast.
    fn stepped(&mut self, step: Step, cx: &mut Context<Self>) -> Option<Option<String>> {
        let outcome = match self.writes.kept() {
            Some(Kept::Stepped(rebased)) => Some(rebased),
            _ => None,
        };
        match (step, outcome) {
            (Step::AbortMerge, _) | (_, None) => Some(None),
            (Step::Continue(seq) | Step::Skip(seq) | Step::Abort(seq), Some(Rebased::Conflicted)) => Some(Some(format!("The {} stopped on another conflict", noun(seq)))),
            (_, Some(Rebased::Conflicted)) => Some(Some(ANOTHER_CONFLICT.into())),
            (step, Some(Rebased::Done { head, autostash })) => {
                let toast = match (step, &autostash) {
                    (Step::AbortRebase, Autostash::Popped) => Some(ABORTED_POPPED.into()),
                    _ => kept_words(&autostash),
                };
                cx.emit(PanelEvent::HeadMoved(head.into(), toast));
                None
            }
        }
    }

    /// A write of the gate ran; this panel's says how it went.
    pub(super) fn written(&mut self, w: &Written, cx: &mut Context<Self>) {
        let Some(doing) = self.writes.written(w) else { return };
        if w.cancelled.is_some() {
            return;
        }
        let toast = match (&doing, &w.result) {
            (Doing::Commit { .. }, Err(why)) => {
                if let Some(c) = &mut self.composer {
                    c.failure = Some(capitalised(why));
                }
                None
            }
            (_, Err(why)) => return cx.emit(PanelEvent::Why(capitalised(why))),
            (Doing::Commit { summary, amend }, Ok(())) => {
                if let Some(c) = &mut self.composer {
                    c.clear(cx);
                }
                Some(format!("{} \"{summary}\"", if *amend { "Amended" } else { "Committed" }))
            }
            (Doing::Discard(n), Ok(())) => Some(format!("Discarded the changes to {}", count(*n, "file", "files"))),
            (Doing::DiscardAll, Ok(())) => Some("Discarded all changes".into()),
            (Doing::Step(step), Ok(())) => match self.stepped(*step, cx) {
                Some(toast) => toast,
                None => return cx.notify(),
            },
            (Doing::Resolve { paths, anyway: false }, Ok(())) => {
                if let Some(markers) = self.markers_left() {
                    self.ask(Ask::ResolveAnyway { paths: paths.clone(), markers }, cx);
                }
                None
            }
            (Doing::Stage | Doing::Unstage | Doing::Take(_) | Doing::Resolve { .. } | Doing::Unresolve, Ok(())) => None,
        };
        cx.emit(PanelEvent::Wrote(toast));
        cx.notify();
    }
}

#[cfg(test)]
pub(crate) mod tests;

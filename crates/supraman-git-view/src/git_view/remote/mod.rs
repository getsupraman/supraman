//! The tab's fetch, pull and push, each through the tab's [`WriteGate`] off the UI thread with progress and Cancel.

mod add;
mod bar;
mod dialogs;
pub(crate) mod menu;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{AppContext as _, Context, Entity, EventEmitter, Pixels, Point, Render, Window, div, prelude::*};
use supraman_core::askpass::{Asked, Bridge};
use supraman_core::secret::SecretStore;
use supraman_gitman::write::remote::{self, Force, Progress, PullChoice, PullMode, Pulled, Pushed, Transfer};
use supraman_gitman::write::rebase::Rebased;
use supraman_gitman::write::{LockSet, Session, WriteError};
use supraman_core::Settings;
use supraman_gitman::{Head, Refs, Remote, Repo, mask_userinfo};
use supraman_ui::confirm::{Answer, Confirm};
use supraman_ui::hit::HitTargets;
use supraman_ui::menu::{ContextMenu, MenuItem};

use super::write_gate::{WriteGate, Writes, Written};
use supraman_core::text::capitalised;
pub(crate) use bar::{caption, words};
pub(crate) use dialogs::{Prompting, UpstreamDialog};
pub(crate) use menu::RemoteFacts;

pub(crate) const DETACHED: &str = "HEAD is detached: check out a branch first";
pub(crate) const NO_REMOTE: &str = "This repository has no remote";
const FORCE_PUSH: &str = "Force Push";

/// What a remote item or the toolbar asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteAct {
    FetchAll,
    Fetch(String),
    /// HEAD's branch pulled.
    Pull(PullMode),
    /// `local` pushed to its upstream, or the upstream dialog first.
    Push(String),
    SetUpstream(String),
    /// `local` pushed to `branch` on `remote` (a pill dropped on a remote branch).
    PushTo { local: String, remote: String, branch: String },
    PushTag { tag: String, remotes: Vec<String> },
    /// `refname` (`refs/heads/<b>`, `refs/tags/<t>`) deleted on `remote`, asked first.
    DeleteOnRemote { remote: String, refname: String },
    EditRemote(String),
    RemoveRemote(String),
    SetDefault(PullChoice),
}

/// A remote call the toolbar and the palette offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    /// `None`: the toolbar's default.
    Pull(Option<PullMode>),
    Push,
    FetchAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteEvent {
    /// A call wrote: its toast; the tab refreshes.
    Wrote(Option<String>),
    Why(String),
    /// The remotes and the pull default the ref menus read.
    Facts(Arc<RemoteFacts>),
}

/// A question before a remote call.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// The remote has commits `local` lacks: pull (yes) or force with lease (alt).
    Behind { local: String, remote: String, branch: String },
    /// The remote branch moved since the last fetch: fetch (yes) or force anyway (alt).
    Stale { local: String, remote: String, branch: String },
    /// The second, destructive question before a forced push.
    Force { local: String, remote: String, branch: String, force: Force },
    Delete { remote: String, refname: String },
    RemoveRemote(String),
}

fn short(refname: &str) -> &str {
    refname.strip_prefix("refs/heads/").or_else(|| refname.strip_prefix("refs/tags/")).unwrap_or(refname)
}

impl Ask {
    fn question(&self) -> Confirm<Ask> {
        let ask = |text: String, yes: &str| Confirm::ask(text, yes, self.clone());
        match self {
            Ask::Behind { local, remote, branch } => ask(format!("'refs/heads/{local}' is behind 'refs/remotes/{remote}/{branch}'. Update your branch by doing a Pull."), bar::words(PullChoice::FastForwardIfPossible))
                .or(FORCE_PUSH, Ask::Force { local: local.clone(), remote: remote.clone(), branch: branch.clone(), force: Force::Lease })
                .danger_alt(),
            Ask::Stale { local, remote, branch } => ask(format!("'refs/remotes/{remote}/{branch}' has changed since your last fetch. Update it by doing a Fetch."), "Fetch")
                .or(FORCE_PUSH, Ask::Force { local: local.clone(), remote: remote.clone(), branch: branch.clone(), force: Force::Anyway })
                .danger_alt(),
            Ask::Force { .. } => ask("Force push is destructive and rewrites history.".into(), FORCE_PUSH).danger(),
            Ask::Delete { remote, refname } => ask(format!("Delete {} from {remote}? It cannot be undone.", short(refname)), "Delete").danger(),
            Ask::RemoveRemote(name) => ask(format!("Remove the remote {name} and its remote-tracking branches? It cannot be undone."), "Remove").danger(),
        }
    }
}

/// The call under way, for its toast and its strip.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Doing {
    FetchAll,
    Fetch(String),
    Pull(PullMode),
    Push { local: String, remote: String, branch: String },
    AddRemote(String),
    Plain(String),
}

impl Doing {
    /// What the strip says while it runs.
    fn running(&self) -> String {
        match self {
            Doing::FetchAll => "Fetching".into(),
            Doing::Fetch(r) => format!("Fetching {r}"),
            Doing::Pull(_) => "Pulling".into(),
            Doing::Push { remote, branch, .. } => format!("Pushing to {remote}/{branch}"),
            Doing::AddRemote(r) => format!("Adding the remote {r}"),
            Doing::Plain(_) => "Working".into(),
        }
    }
}

/// What a call's job left.
#[derive(Debug, Clone)]
enum Done {
    Nothing,
    Pulled(Pulled),
    Pushed(Pushed),
    /// A remote added; why its first fetch failed.
    Added(Option<String>),
}

/// A call's progress and prompts, heard off the UI thread.
enum Heard {
    Progress(Progress),
    Asked(Asked),
}

struct Running {
    doing: Doing,
    progress: Option<Progress>,
    cancel: Arc<AtomicBool>,
}

pub(crate) struct RemoteActions {
    hits: HitTargets,
    writes: Writes<Ask, Running, Done>,
    home: Option<PathBuf>,
    store: Option<Arc<dyn SecretStore>>,
    refs: Arc<Refs>,
    under_way: bool,
    remotes: Vec<Remote>,
    pull: PullChoice,
    prune: bool,
    /// A cancelled call's ticket, said as cancelled when it ends.
    cancelled: Option<u64>,
    menu: Option<ContextMenu<RemoteAct>>,
    upstream: Option<UpstreamDialog>,
    editing: Option<dialogs::EditRemote>,
    adding: Option<add::AddRemote>,
    prompting: Option<Prompting>,
}

impl EventEmitter<RemoteEvent> for RemoteActions {}

impl RemoteActions {
    /// `home` holds `settings.toml` (`[gitman] pull`, `prune`) and the secret store a remembered passphrase goes to.
    pub(crate) fn new(gate: Entity<WriteGate>, hits: HitTargets, home: Option<PathBuf>, cx: &mut Context<Self>) -> RemoteActions {
        let store = home.as_deref().map(supraman_core::secret::default_store);
        RemoteActions {
            hits,
            writes: Writes::new(gate, Self::written, cx),
            home,
            store,
            refs: Arc::new(Refs::empty()),
            under_way: false,
            remotes: Vec::new(),
            pull: PullChoice::default(),
            prune: true,
            cancelled: None,
            menu: None,
            upstream: None,
            editing: None,
            adding: None,
            prompting: None,
        }
    }

    /// `[gitman]`'s remote keys, read by the tab's first load.
    pub(crate) fn set_settings(&mut self, settings: supraman_gitman::RemoteSettings, cx: &mut Context<Self>) {
        self.pull = settings.pull.unwrap_or_default();
        self.prune = settings.prune.unwrap_or(true);
        self.said_facts(cx);
    }

    /// The refs a read found; the remotes read again off the UI thread.
    pub(crate) fn set_refs(&mut self, repo: Repo, refs: Arc<Refs>, under_way: bool, cx: &mut Context<Self>) {
        self.refs = refs;
        self.under_way = under_way;
        let opening = self.writes.gate().update(cx, |g, cx| g.open(&repo, cx));
        let read = cx.background_spawn(async move {
            let writer = opening.await.ok_or_else(|| "the repository did not open".to_string())?;
            supraman_gitman::remotes(writer.repo()).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| match read {
                Ok(remotes) => {
                    this.remotes = remotes;
                    this.said_facts(cx);
                }
                Err(why) => log::warn!("git tab: cannot read the remotes: {why}"),
            })
            .ok();
        })
        .detach();
        self.said_facts(cx);
    }

    fn said_facts(&mut self, cx: &mut Context<Self>) {
        cx.emit(RemoteEvent::Facts(Arc::new(RemoteFacts::new(self.remotes.clone(), &self.refs, self.pull))));
        cx.notify();
    }

    pub(crate) fn has_remotes(&self) -> bool {
        !self.remotes.is_empty()
    }

    pub(crate) fn remote_names(&self) -> Vec<String> {
        self.remotes.iter().map(|r| r.name.clone()).collect()
    }

    pub(crate) fn prune(&self) -> bool {
        self.prune
    }

    pub(crate) fn under_way(&self) -> bool {
        self.under_way
    }

    /// The repository's worktrees as the last read listed them.
    pub(crate) fn worktrees(&self) -> Vec<PathBuf> {
        self.refs.worktrees.iter().map(|w| w.path.clone()).collect()
    }

    pub(crate) fn pull_choice(&self) -> PullChoice {
        self.pull
    }

    pub(crate) fn current(&self) -> Option<&str> {
        supraman_gitman::write::branch::current_branch(&self.refs)
    }

    /// The upstream of `local` as `(remote, branch)`.
    pub(crate) fn upstream(&self, local: &str) -> Option<(String, String)> {
        let name = &self.refs.branches.iter().find(|b| b.name == local)?.upstream.as_ref()?.name;
        menu::split_remote(&self.remotes, name).map(|(r, b)| (r.to_string(), b.to_string()))
    }

    /// Why a remote call cannot start: another runs or waits, the gate is busy.
    pub(crate) fn busy(&self, cx: &gpui::App) -> Option<String> {
        if let Some((_, running)) = self.writes.doing() {
            return Some(format!("{}…", running.doing.running()));
        }
        self.writes.busy(cx).map(str::to_string)
    }

    /// The call under way: its words and progress.
    pub(crate) fn progress(&self) -> Option<(String, Option<&Progress>)> {
        self.writes.doing().map(|(_, r)| (r.doing.running(), r.progress.as_ref()))
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some((ticket, running)) = self.writes.doing() {
            running.cancel.store(true, Ordering::SeqCst);
            self.cancelled = Some(ticket);
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn asking(&self) -> Option<&str> {
        self.writes.asked()
    }

    /// The question's answers as the band paints them, each with whether it destroys something, and what Enter answers.
    #[cfg(test)]
    pub(crate) fn answers(&self) -> Option<(Vec<(String, bool)>, Answer)> {
        let c = self.writes.asking.as_ref()?;
        let mut all = vec![(c.yes.clone(), c.danger)];
        all.extend(c.alt.as_ref().map(|(w, _)| (w.clone(), c.alt_danger)));
        all.push(("Cancel".into(), false));
        Some((all, c.default_answer()))
    }

    #[cfg(test)]
    pub(crate) fn menu_labels(&self) -> Vec<(String, Option<bool>)> {
        self.menu.as_ref().map(|m| m.items.iter().map(|i| (i.label.clone(), i.ticked)).collect()).unwrap_or_default()
    }

    /// Runs `job` with a [`Transfer`] whose prompts and progress come back here, holding `locks`.
    fn request(&mut self, doing: Doing, locks: LockSet, job: impl FnOnce(&Session<'_>, &Transfer) -> Result<Done, WriteError> + Send + 'static, cx: &mut Context<Self>) {
        if self.busy(cx).is_some() {
            return;
        }
        let (heard, mut hearing) = unbounded::<Heard>();
        let cancel = Arc::new(AtomicBool::new(false));
        let (store, stop) = (self.store.clone(), cancel.clone());
        let wrapped = self.writes.keeping(|done| done, move |s: &Session<'_>| {
            let to = heard.clone();
            let mut t = Transfer { cancel: stop, progress: Arc::new(move |p| drop(to.unbounded_send(Heard::Progress(p)))), env: Vec::new() };
            let slot = Arc::new(Mutex::new(Some(heard)));
            let bridge = askpass(store, slot.clone());
            if let Some((_, env)) = &bridge {
                t.env = env.clone();
            }
            let done = job(s, &t);
            drop(bridge);
            slot.lock().unwrap_or_else(PoisonError::into_inner).take();
            done
        });
        let ticket = match self.writes.request(Running { doing, progress: None, cancel }, locks, wrapped, cx) {
            Ok(Some(ticket)) => ticket,
            Ok(None) => return,
            Err(why) => return cx.emit(RemoteEvent::Why(why.into())),
        };
        cx.spawn(async move |this, cx| {
            while let Some(h) = hearing.next().await {
                if this.update(cx, |this, cx| this.heard(ticket, h, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn heard(&mut self, ticket: u64, heard: Heard, cx: &mut Context<Self>) {
        match heard {
            Heard::Progress(p) => {
                if let Some(running) = self.writes.doing_mut(ticket) {
                    running.progress = Some(p);
                }
            }
            Heard::Asked(asked) => self.prompting = Some(Prompting::new(asked)),
        }
        cx.notify();
    }

    fn written(&mut self, w: &Written, cx: &mut Context<Self>) {
        let Some(running) = self.writes.written(w) else { return };
        if let Some(p) = self.prompting.take() {
            p.cancel();
        }
        let done = self.writes.kept();
        if let (Doing::AddRemote(name), Ok(())) = (&running.doing, &w.result) {
            self.adding = None;
            let fetched = match done {
                Some(Done::Added(Some(why))) if self.cancelled.take() != Some(w.ticket) => Err(why),
                _ => Ok(()),
            };
            self.cancelled = None;
            return self.added(name, fetched, cx);
        }
        if self.cancelled.take() == Some(w.ticket) {
            cx.emit(RemoteEvent::Wrote(None));
            return cx.emit(RemoteEvent::Why(format!("{} was cancelled", running.doing.running())));
        }
        if let Err(why) = &w.result {
            cx.emit(RemoteEvent::Wrote(None));
            return cx.emit(RemoteEvent::Why(capitalised(why)));
        }
        let toast = match (running.doing, done.unwrap_or(Done::Nothing)) {
            (Doing::FetchAll, _) => "Fetched every remote".to_string(),
            (Doing::Fetch(r), _) => format!("Fetched {r}"),
            (Doing::Pull(mode), Done::Pulled(pulled)) => match pulled {
                Pulled::UpToDate => "Already up to date".into(),
                Pulled::Merged => "Pulled".into(),
                Pulled::Rebased(Rebased::Done { .. }) => "Pulled and rebased".into(),
                Pulled::Rebased(Rebased::Conflicted) | Pulled::Conflicted => {
                    cx.emit(RemoteEvent::Wrote(None));
                    return cx.emit(RemoteEvent::Why("The pull stopped on conflicts: resolve them in the WIP".into()));
                }
                Pulled::NoUpstream { branch } => return self.ask_upstream(branch, dialogs::Then::Pull(mode), cx),
                Pulled::Gone { upstream } => return cx.emit(RemoteEvent::Why(format!("{upstream} is gone from its remote: set another upstream"))),
            },
            (Doing::Push { local, remote, branch }, Done::Pushed(pushed)) => match pushed {
                Pushed::Done => format!("Pushed {local} to {remote}/{branch}"),
                Pushed::Behind => return self.ask(Ask::Behind { local, remote, branch }, cx),
                Pushed::StaleLease => return self.ask(Ask::Stale { local, remote, branch }, cx),
            },
            (Doing::Plain(words), _) => words,
            _ => String::new(),
        };
        cx.emit(RemoteEvent::Wrote(Some(toast).filter(|t| !t.is_empty())));
        cx.notify();
    }

    fn ask(&mut self, ask: Ask, cx: &mut Context<Self>) {
        self.writes.asking = Some(ask.question());
        cx.notify();
    }

    pub(crate) fn answer(&mut self, answer: Answer, cx: &mut Context<Self>) {
        let Some(asked) = self.writes.asking.take() else { return };
        match asked.take(answer) {
            Some(Ask::Behind { .. }) => self.act(RemoteAct::Pull(PullMode::FastForwardIfPossible), cx),
            Some(Ask::Stale { remote, .. }) => self.act(RemoteAct::Fetch(remote), cx),
            Some(ask @ Ask::Force { .. }) => self.ask(ask, cx),
            Some(Ask::Delete { remote, refname }) => {
                let words = format!("Deleted {} from {remote}", short(&refname));
                self.request(Doing::Plain(words), LockSet::push(), move |s, t| remote::delete_remote_ref(s, &remote, &refname, t).map(|_| Done::Nothing), cx);
            }
            Some(Ask::RemoveRemote(name)) => {
                let words = format!("Removed the remote {name}");
                self.request(Doing::Plain(words), LockSet::remote_config(), move |s, _| remote::remove_remote(s, &name).map(|_| Done::Nothing), cx);
            }
            None => {}
        }
        cx.notify();
    }

    /// The forced push the second question was about, once confirmed.
    fn answer_force(&mut self, answer: Answer, cx: &mut Context<Self>) {
        match self.writes.asking.as_ref().and_then(|a| a.action.clone()) {
            Some(Ask::Force { local, remote, branch, force }) if answer == Answer::Yes => {
                self.writes.asking = None;
                self.push(local, remote, branch, force, cx);
            }
            Some(Ask::Force { .. }) => {
                self.writes.asking = None;
                cx.notify();
            }
            _ => self.answer(answer, cx),
        }
    }

    fn push(&mut self, local: String, remote: String, branch: String, force: Force, cx: &mut Context<Self>) {
        let doing = Doing::Push { local: local.clone(), remote: remote.clone(), branch: branch.clone() };
        self.request(doing, LockSet::push(), move |s, t| remote::push(s, &local, &remote, &branch, force, t).map(Done::Pushed), cx);
    }

    /// What an item, a key or the toolbar asks for.
    pub(crate) fn act(&mut self, act: RemoteAct, cx: &mut Context<Self>) {
        self.menu = None;
        let names: Vec<String> = self.remotes.iter().map(|r| r.name.clone()).collect();
        let prune = self.prune;
        match act {
            RemoteAct::FetchAll if names.is_empty() => cx.emit(RemoteEvent::Why(NO_REMOTE.into())),
            RemoteAct::FetchAll => self.request(Doing::FetchAll, LockSet::fetch(), move |s, t| remote::fetch_all(s, &names, prune, t).map(|_| Done::Nothing), cx),
            RemoteAct::Fetch(name) => {
                let doing = Doing::Fetch(name.clone());
                self.request(doing, LockSet::fetch(), move |s, t| remote::fetch(s, &name, prune, t).map(|_| Done::Nothing), cx);
            }
            RemoteAct::Pull(mode) => match self.pull_why_not(Some(mode)) {
                Some(why) => cx.emit(RemoteEvent::Why(why.into())),
                None => self.request(Doing::Pull(mode), LockSet::pull(), move |s, t| remote::pull(s, mode, prune, t).map(Done::Pulled), cx),
            },
            RemoteAct::Push(local) => match self.upstream(&local) {
                Some((remote, branch)) => self.push(local, remote, branch, Force::No, cx),
                None if self.remotes.is_empty() => cx.emit(RemoteEvent::Why(NO_REMOTE.into())),
                None => self.ask_upstream(local, dialogs::Then::Push, cx),
            },
            RemoteAct::SetUpstream(local) => self.ask_upstream(local, dialogs::Then::Set, cx),
            RemoteAct::PushTo { local, remote, branch } => self.push(local, remote, branch, Force::No, cx),
            RemoteAct::PushTag { tag, remotes } => {
                let words = format!("Pushed {tag} to {}", remotes.join(", "));
                self.request(Doing::Plain(words), LockSet::push(), move |s, t| {
                    for r in &remotes {
                        if remote::push_tag(s, r, &tag, t)? != Pushed::Done {
                            return Err(WriteError::Remote(format!("{r} already has another {tag}")));
                        }
                    }
                    Ok(Done::Nothing)
                }, cx);
            }
            RemoteAct::DeleteOnRemote { remote, refname } => self.ask(Ask::Delete { remote, refname }, cx),
            RemoteAct::EditRemote(name) => self.edit_remote(name, cx),
            RemoteAct::RemoveRemote(name) => self.ask(Ask::RemoveRemote(name), cx),
            RemoteAct::SetDefault(choice) => self.set_default(choice, cx),
        }
        cx.notify();
    }

    /// Why a pull (`None`: the toolbar's default) cannot start now.
    pub(crate) fn pull_why_not(&self, mode: Option<PullMode>) -> Option<&'static str> {
        let fetch = mode.is_none() && self.pull == PullChoice::Fetch;
        if self.remotes.is_empty() {
            Some(NO_REMOTE)
        } else if fetch {
            None
        } else if matches!(self.refs.head, Head::Detached { .. }) {
            Some(DETACHED)
        } else if self.under_way {
            Some(supraman_gitman::write::UNDER_WAY)
        } else {
            None
        }
    }

    /// Why Push cannot start now.
    pub(crate) fn push_why_not(&self) -> Option<&'static str> {
        if self.remotes.is_empty() {
            Some(NO_REMOTE)
        } else if self.current().is_none() {
            Some(DETACHED)
        } else {
            None
        }
    }

    /// Why `button` is dimmed now, on the toolbar and in the palette.
    pub(crate) fn why_not(&self, button: Button, cx: &gpui::App) -> Option<String> {
        self.busy(cx).or_else(|| {
            let why = match button {
                Button::Pull(mode) => self.pull_why_not(mode),
                Button::Push => self.push_why_not(),
                Button::FetchAll => self.remotes.is_empty().then_some(NO_REMOTE),
            };
            why.map(str::to_string)
        })
    }

    /// The toolbar's Pull: its default.
    pub(crate) fn pull_default(&mut self, cx: &mut Context<Self>) {
        match self.pull.mode() {
            None => self.act(RemoteAct::FetchAll, cx),
            Some(mode) => self.act(RemoteAct::Pull(mode), cx),
        }
    }

    /// The toolbar's Push: HEAD's branch.
    pub(crate) fn push_head(&mut self, cx: &mut Context<Self>) {
        match (self.push_why_not(), self.current().map(str::to_string)) {
            (None, Some(local)) => self.act(RemoteAct::Push(local), cx),
            (Some(why), _) => cx.emit(RemoteEvent::Why(why.into())),
            _ => {}
        }
    }

    /// The default kept, then saved in `[gitman] pull` off the UI thread.
    fn set_default(&mut self, choice: PullChoice, cx: &mut Context<Self>) {
        self.pull = choice;
        self.said_facts(cx);
        let Some(home) = self.home.clone() else { return };
        let save = cx.background_spawn(async move { Settings::update(&home, |s| supraman_gitman::set_setting(s, "pull", choice)).map_err(|e| e.to_string()).and_then(|(_, set)| set) });
        cx.spawn(async move |this, cx| {
            if let Err(why) = save.await {
                this.update(cx, |_, cx| cx.emit(RemoteEvent::Why(format!("The default is not saved: {why}")))).ok();
            }
        })
        .detach();
    }

    /// The Pull button's menu at `position`: each pull, then the default ticked.
    pub(crate) fn open_pull_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let mut items: Vec<MenuItem<RemoteAct>> = Vec::new();
        for choice in PullChoice::ALL {
            let act = match choice.mode() {
                None => RemoteAct::FetchAll,
                Some(mode) => RemoteAct::Pull(mode),
            };
            let it = supraman_ui::menu::item(format!("pull-{}", bar::key_of(choice)), bar::words(choice), act);
            items.push(match self.pull_why_not(choice.mode()).filter(|_| choice.mode().is_some()) {
                Some(why) => it.dimmed(why),
                None => it,
            });
        }
        for (i, choice) in PullChoice::ALL.into_iter().enumerate() {
            let it = supraman_ui::menu::item(format!("pull-default-{}", bar::key_of(choice)), bar::words(choice), RemoteAct::SetDefault(choice)).ticked(choice == self.pull);
            items.push(if i == 0 { it.under("Set as default") } else { it });
        }
        self.menu = Some(ContextMenu { title: "The button does the default".into(), position, items });
        cx.notify();
    }

    fn ask_upstream(&mut self, local: String, then: dialogs::Then, cx: &mut Context<Self>) {
        if self.remotes.is_empty() {
            return cx.emit(RemoteEvent::Why(NO_REMOTE.into()));
        }
        self.upstream = Some(UpstreamDialog::new(local, then, &self.remotes));
        cx.notify();
    }

    /// The upstream dialog's answer: set, then pushed or pulled as it was asked for.
    fn submit_upstream(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.upstream else { return };
        let Some((local, remote, branch, then)) = dialog.chosen(&self.remotes, cx) else { return cx.notify() };
        self.upstream = None;
        let prune = self.prune;
        match then {
            dialogs::Then::Push => {
                let doing = Doing::Push { local: local.clone(), remote: remote.clone(), branch: branch.clone() };
                self.request(doing, LockSet::push(), move |s, t| remote::publish(s, &local, &remote, &branch, t).map(Done::Pushed), cx);
            }
            dialogs::Then::Pull(mode) => self.request(Doing::Pull(mode), LockSet::pull(), move |s, t| {
                remote::set_upstream(s, &local, &remote, &branch)?;
                remote::pull(s, mode, prune, t).map(Done::Pulled)
            }, cx),
            dialogs::Then::Set => {
                let words = format!("{local} tracks {remote}/{branch}");
                self.request(Doing::Plain(words), LockSet::branch(), move |s, _| remote::set_upstream(s, &local, &remote, &branch).map(|_| Done::Nothing), cx);
            }
        }
    }

    fn edit_remote(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(r) = self.remotes.iter().find(|r| r.name == name) else { return };
        self.editing = Some(dialogs::EditRemote::new(name, r.url.clone()));
        cx.notify();
    }

    fn submit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.take() else { return };
        let (name, url) = (edit.name.clone(), edit.url(cx));
        if url.is_empty() || url == mask_userinfo(&edit.was) {
            return cx.notify();
        }
        let words = format!("{name} now fetches from {}", mask_userinfo(&url));
        self.request(Doing::Plain(words), LockSet::remote_config(), move |s, _| remote::set_url(s, &name, &url).map(|_| Done::Nothing), cx);
    }
}

/// None without `sm.exe` beside the app; the call empties `heard` at its end, so the bridge never holds the last sender.
fn askpass(store: Option<Arc<dyn SecretStore>>, heard: Arc<Mutex<Option<UnboundedSender<Heard>>>>) -> Option<(Bridge, Vec<(String, OsString)>)> {
    let program = supraman_core::install::sm_exe().ok().filter(|p| p.is_file())?;
    let bridge = Bridge::start(store, move |asked| {
        if let Some(to) = heard.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            drop(to.unbounded_send(Heard::Asked(asked)));
        }
    })
    .ok()?;
    let env = bridge.env(&program);
    Some((bridge, env))
}

impl Render for RemoteActions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let menu = self.menu.as_ref().map(|menu| {
            menu.render(&self.hits, |this: &mut RemoteActions, act, _, cx| this.act(act, cx), |this: &mut RemoteActions, cx| {
                this.menu = None;
                cx.notify();
            }, cx)
        });
        div().children(menu)
    }
}

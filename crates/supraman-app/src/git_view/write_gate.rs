//! Every Git tab write passes here: on a worktree a live Supraman session works in it asks first (roadmap r4),
//! then runs on the background executor through [`Writer::run`]. One write at a time per tab: while one waits
//! for its answer or runs, [`WriteGate::busy`] says why another cannot start.

use std::path::Path;
use std::sync::Arc;

use gpui::{AppContext as _, Context, EventEmitter, Render, Window, div, prelude::*};
use supraman_gitman::write::{LockSet, Occupancy, Session, WriteError, Writer};
use supraman_ui::confirm::Confirm;
use supraman_ui::hit::HitTargets;

pub type Job = Box<dyn FnOnce(&Session<'_>) -> Result<(), WriteError> + Send>;

/// Why a write cannot start while another waits for its answer or runs.
pub const WAITING: &str = "Waiting for your answer";
pub const WORKING: &str = "Working…";

/// A write waiting for the user's answer.
pub struct Pending {
    ticket: u64,
    writer: Writer,
    locks: LockSet,
    job: Job,
}

/// The write of `ticket` ran: its error's words, if it failed.
pub struct Written {
    pub ticket: u64,
    pub result: Result<(), String>,
}

pub struct WriteGate {
    occupancy: Arc<dyn Occupancy>,
    hits: HitTargets,
    asking: Option<Confirm<Pending>>,
    running: bool,
    tickets: u64,
}

impl EventEmitter<Written> for WriteGate {}

/// A write's error as a toast says it: its first letter capitalised.
pub fn capitalised(why: &str) -> String {
    let mut chars = why.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// The question before a write on `folder`, where `sessions` work.
pub fn question(sessions: &[String], folder: &Path) -> String {
    let verb = if sessions.len() == 1 { "is" } else { "are" };
    format!("{} {verb} working in {}. Change it anyway?", sessions.join(", "), folder.display())
}

impl WriteGate {
    /// `occupancy` answers from memory: it is asked on the UI thread.
    pub fn new(occupancy: Arc<dyn Occupancy>, hits: HitTargets) -> WriteGate {
        WriteGate { occupancy, hits, asking: None, running: false, tickets: 0 }
    }

    /// Runs `job` holding `locks` on `writer`'s checkout, after the user's yes when a session works in it; the
    /// ticket its [`Written`] carries, or `None` (nothing asked or run) while [`WriteGate::busy`].
    pub fn request(&mut self, writer: Writer, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<(), WriteError> + Send + 'static, cx: &mut Context<Self>) -> Option<u64> {
        if self.busy().is_some() {
            return None;
        }
        self.tickets += 1;
        let ticket = self.tickets;
        let top = writer.dirs().top.clone();
        let sessions = self.occupancy.who_works_in(&top);
        let pending = Pending { ticket, writer, locks, job: Box::new(job) };
        if sessions.is_empty() {
            self.start(pending, cx);
        } else {
            self.asking = Some(Confirm::ask(question(&sessions, &top), "Change it anyway", pending));
        }
        cx.notify();
        Some(ticket)
    }

    /// Why no other write can start now: one waits for the user's answer, or one runs.
    pub fn busy(&self) -> Option<&'static str> {
        if self.asking.is_some() {
            Some(WAITING)
        } else if self.running {
            Some(WORKING)
        } else {
            None
        }
    }

    /// The user's answer to the question shown; a no drops the write.
    pub fn answer(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(pending) = self.asking.take().and_then(|asked| asked.action).filter(|_| yes) {
            self.start(pending, cx);
        }
        cx.notify();
    }

    #[cfg(test)]
    pub fn asking(&self) -> Option<&str> {
        self.asking.as_ref().map(|c| c.text.as_str())
    }

    fn start(&mut self, pending: Pending, cx: &mut Context<Self>) {
        let Pending { ticket, writer, locks, job } = pending;
        self.running = true;
        let ran = cx.background_spawn(async move { writer.run(locks, job).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let result = ran.await;
            this.update(cx, |this, cx| {
                this.running = false;
                cx.emit(Written { ticket, result });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for WriteGate {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let question = self.asking.as_ref().map(|asked| asked.render(&self.hits, "write-guard", |this: &mut Self, yes, _, cx| this.answer(yes, cx), cx));
        div().children(question)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
    use supraman_gitman::fixture::{Fixture, git_in};
    use supraman_gitman::write::{Call, Resource, working_in};
    use supraman_gitman::Repo;

    use super::*;

    fn staged(f: &Fixture, dir: &Path) -> Vec<String> {
        git_in(&f.git, dir, &["diff", "--cached", "--name-only"], &[], None).lines().map(str::to_string).collect()
    }

    fn stage(path: &'static str) -> impl FnOnce(&Session<'_>) -> Result<(), WriteError> + Send + 'static {
        move |s| s.output(&Call::new(&["add"]).paths([path])).map(drop)
    }

    fn click(gate: &Entity<WriteGate>, key: &str, cx: &mut VisualTestContext) {
        gate.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.update(|_, _| ());
        let at = gate.read_with(cx, |g, _| g.hits.borrow().get(key).map(|h| h.bounds)).unwrap_or_else(|| panic!("{key} is painted")).center();
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_write_where_a_session_works_asks_first_and_cancel_writes_nothing(cx: &mut TestAppContext) {
        cx.update(|cx| supraman_ui::init_globals(cx, "test"));
        let f = Fixture::copy();
        let (main, side) = (Writer::open(&f.repo()).unwrap(), Writer::open(&Repo::new(f.git.clone(), &f.worktree)).unwrap());
        let cwd: PathBuf = main.dirs().top.join("src");
        let occupancy: Arc<dyn Occupancy> = Arc::new(move |worktree: &Path| working_in([("fm-alpha", cwd.as_path())], worktree));
        let (gate, cx) = cx.add_window_view(|_, _| WriteGate::new(occupancy, HitTargets::default()));
        let written = Rc::new(RefCell::new(Vec::new()));
        let heard = written.clone();
        cx.update(|_, cx| cx.subscribe(&gate, move |_, w: &Written, _| heard.borrow_mut().push(w.result.clone())).detach());
        let top = main.dirs().top.clone();

        gate.update(cx, |g, cx| g.request(main.clone(), LockSet::new().write(Resource::Index), stage("u.txt"), cx));
        cx.run_until_parked();
        assert_eq!(gate.read_with(cx, |g, _| g.asking().map(str::to_string)), Some(question(&["fm-alpha".into()], &top)));
        assert_eq!(gate.read_with(cx, |g, _| g.busy()), Some(WAITING));
        assert_eq!(gate.update(cx, |g, cx| g.request(main.clone(), LockSet::new().write(Resource::Index), stage("u.txt"), cx)), None, "a second write is not queued while one waits");
        assert!(!staged(&f, &f.dir).contains(&"u.txt".into()), "nothing runs before the answer");
        click(&gate, "write-guard-cancel", cx);
        assert_eq!(gate.read_with(cx, |g, _| g.asking().is_none()), true);
        assert!(!staged(&f, &f.dir).contains(&"u.txt".into()) && written.borrow().is_empty(), "cancel writes nothing");

        gate.update(cx, |g, cx| g.request(main.clone(), LockSet::new().write(Resource::Index), stage("u.txt"), cx));
        click(&gate, "write-guard-yes", cx);
        assert_eq!(*written.borrow(), [Ok(())]);
        assert_eq!(gate.read_with(cx, |g, _| g.busy()), None, "free once the write ran");
        assert!(staged(&f, &f.dir).contains(&"u.txt".into()), "yes writes");

        std::fs::write(f.worktree.join("w.txt"), b"w\n").unwrap();
        gate.update(cx, |g, cx| g.request(side, LockSet::new().write(Resource::Index), stage("w.txt"), cx));
        cx.run_until_parked();
        assert_eq!(gate.read_with(cx, |g, _| g.asking().is_none()), true, "nobody works in the other worktree");
        assert!(staged(&f, &f.worktree).contains(&"w.txt".into()));
    }
}

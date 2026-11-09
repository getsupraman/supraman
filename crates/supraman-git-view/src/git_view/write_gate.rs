//! Every Git tab write passes here: it runs on the background executor through [`Writer::run`], one at a time per
//! tab; while one runs, [`WriteGate::busy`] says why another cannot start. A live session working in the checkout
//! shows as the graph's agent row, not as a question.

use std::sync::Arc;

use gpui::{AppContext as _, Context, EventEmitter};
use supraman_gitman::write::{LockSet, Occupancy, Session, WriteError, Writer};

/// Why a write cannot start while another runs.
pub(crate) const WORKING: &str = "Working…";

/// The write of `ticket` ran: its error's words, if it failed.
pub(crate) struct Written {
    pub(crate) ticket: u64,
    pub(crate) result: Result<(), String>,
}

pub(crate) struct WriteGate {
    occupancy: Arc<dyn Occupancy>,
    running: bool,
    tickets: u64,
}

impl EventEmitter<Written> for WriteGate {}

/// A write's error as a toast says it: its first letter capitalised.
pub(crate) fn capitalised(why: &str) -> String {
    let mut chars = why.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

impl WriteGate {
    /// `occupancy` answers from memory: it is asked on the UI thread.
    pub(crate) fn new(occupancy: Arc<dyn Occupancy>) -> WriteGate {
        WriteGate { occupancy, running: false, tickets: 0 }
    }

    /// Runs `job` holding `locks` on `writer`'s checkout; the ticket its [`Written`] carries, or `None` (nothing run)
    /// while [`WriteGate::busy`].
    pub(crate) fn request(&mut self, writer: Writer, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<(), WriteError> + Send + 'static, cx: &mut Context<Self>) -> Option<u64> {
        if self.busy().is_some() {
            return None;
        }
        self.tickets += 1;
        let ticket = self.tickets;
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
        cx.notify();
        Some(ticket)
    }

    /// Who works where: the agent and worktree rows and maintenance's skip.
    pub(crate) fn occupancy(&self) -> &Arc<dyn Occupancy> {
        &self.occupancy
    }

    /// Why no other write can start now: one runs.
    pub(crate) fn busy(&self) -> Option<&'static str> {
        self.running.then_some(WORKING)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use gpui::TestAppContext;
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

    #[gpui::test]
    fn a_write_where_a_session_works_runs_without_a_question(cx: &mut TestAppContext) {
        cx.update(|cx| supraman_ui::init_globals(cx, "test"));
        let f = Fixture::copy();
        let (main, side) = (Writer::open(&f.repo()).unwrap(), Writer::open(&Repo::new(f.git.clone(), &f.worktree)).unwrap());
        let cwd: PathBuf = main.dirs().top.join("src");
        let occupancy: Arc<dyn Occupancy> = Arc::new(move |worktree: &Path| working_in([("fm-alpha", cwd.as_path())], worktree));
        let gate = cx.new(|_| WriteGate::new(occupancy));
        let written = Rc::new(RefCell::new(Vec::new()));
        let heard = written.clone();
        cx.update(|cx| cx.subscribe(&gate, move |_, w: &Written, _| heard.borrow_mut().push(w.result.clone())).detach());

        gate.update(cx, |g, cx| g.request(main.clone(), LockSet::new().write(Resource::Index), stage("u.txt"), cx));
        assert_eq!(gate.read_with(cx, |g, _| g.busy()), Some(WORKING));
        assert_eq!(gate.update(cx, |g, cx| g.request(main.clone(), LockSet::new().write(Resource::Index), stage("u.txt"), cx)), None, "a second write is not queued while one runs");
        cx.run_until_parked();
        assert_eq!(*written.borrow(), [Ok(())]);
        assert_eq!(gate.read_with(cx, |g, _| g.busy()), None, "free once the write ran");
        assert!(staged(&f, &f.dir).contains(&"u.txt".into()), "the write ran with fm-alpha working there");

        std::fs::write(f.worktree.join("w.txt"), b"w
").unwrap();
        gate.update(cx, |g, cx| g.request(side, LockSet::new().write(Resource::Index), stage("w.txt"), cx));
        cx.run_until_parked();
        assert!(staged(&f, &f.worktree).contains(&"w.txt".into()));
    }
}

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// What a git operation reads or writes besides the objects (docs/research/gitkraken-12.6.1.md §2 "Locking").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resource {
    Index,
    Config,
    Remote,
    RepoSettings,
    GitAttributes,
    GitIgnore,
    Gpg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    Read,
    Write,
}

/// The resources one operation holds while it runs; a write on a resource excludes every other access to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockSet(BTreeMap<Resource, Access>);

impl LockSet {
    pub fn new() -> LockSet {
        LockSet::default()
    }

    pub fn read(mut self, resource: Resource) -> LockSet {
        self.0.entry(resource).or_insert(Access::Read);
        self
    }

    pub fn write(mut self, resource: Resource) -> LockSet {
        self.0.insert(resource, Access::Write);
        self
    }

    fn writes(resources: &[Resource]) -> LockSet {
        resources.iter().fold(LockSet::new(), |set, r| set.write(*r))
    }

    /// A refresh of the status, the work dir, the stashes or the refs: it waits for an index write.
    pub fn refresh() -> LockSet {
        LockSet::new().read(Resource::Index)
    }

    pub fn commit() -> LockSet {
        LockSet::writes(&[Resource::Config, Resource::Index])
    }

    pub fn push() -> LockSet {
        LockSet::writes(&[Resource::Config, Resource::GitAttributes, Resource::Index, Resource::RepoSettings])
    }

    pub fn branch_delete() -> LockSet {
        LockSet::writes(&[Resource::Config, Resource::Remote, Resource::Index, Resource::RepoSettings])
    }

    pub fn reset() -> LockSet {
        LockSet::writes(&[Resource::Index, Resource::RepoSettings])
    }

    /// A stash's push, apply, pop, drop or new message: the index, the work tree and `refs/stash`.
    pub fn stash() -> LockSet {
        LockSet::writes(&[Resource::Index, Resource::RepoSettings])
    }

    /// An undo or a redo: it may move HEAD, rewrite the index and a branch's upstream.
    pub fn undo() -> LockSet {
        LockSet::writes(&[Resource::Config, Resource::Index, Resource::RepoSettings])
    }

    pub fn access(&self, resource: Resource) -> Option<Access> {
        self.0.get(&resource).copied()
    }

    pub fn has_write(&self) -> bool {
        self.0.values().any(|a| *a == Access::Write)
    }

    fn conflicts(&self, other: &LockSet) -> bool {
        self.0.iter().any(|(r, a)| other.0.get(r).is_some_and(|b| *a == Access::Write || *b == Access::Write))
    }

    /// The part of `self` that `held` does not already grant.
    fn beyond(&self, held: &LockSet) -> LockSet {
        LockSet(self.0.iter().filter(|(r, a)| held.0.get(r).is_none_or(|h| h < a)).map(|(r, a)| (*r, *a)).collect())
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One repository's locks: every request waits in one queue, and on every change the requests whose locks are
/// all free run in queue order (first-fit: a later request on free resources overtakes a blocked one).
#[derive(Default)]
pub struct Scheduler {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    held: Vec<(u64, LockSet)>,
    queue: Vec<(u64, LockSet)>,
    granted: HashSet<u64>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

impl Scheduler {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Blocks until `locks` are free, then holds them until the [`Held`] drops; never on the UI thread.
    pub fn acquire(self: &Arc<Self>, locks: LockSet) -> Held {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut state = self.state();
        state.queue.push((id, locks.clone()));
        grant(&mut state);
        self.changed.notify_all();
        let mut state = self.changed.wait_while(state, |s| !s.granted.contains(&id)).unwrap_or_else(PoisonError::into_inner);
        state.granted.remove(&id);
        Held { scheduler: self.clone(), id, locks }
    }

    fn release(&self, id: u64) {
        let mut state = self.state();
        state.held.retain(|(held, _)| *held != id);
        grant(&mut state);
        self.changed.notify_all();
    }

    /// How many requests wait for their locks.
    pub fn waiting(&self) -> usize {
        self.state().queue.len()
    }

    /// Waits up to `timeout` until `n` requests wait; whether they do.
    pub fn await_waiting(&self, n: usize, timeout: Duration) -> bool {
        let (state, _) = self.changed.wait_timeout_while(self.state(), timeout, |s| s.queue.len() < n).unwrap_or_else(PoisonError::into_inner);
        state.queue.len() >= n
    }
}

fn grant(state: &mut State) {
    let mut i = 0;
    while i < state.queue.len() {
        if state.held.iter().any(|(_, held)| held.conflicts(&state.queue[i].1)) {
            i += 1;
            continue;
        }
        let (id, locks) = state.queue.remove(i);
        state.held.push((id, locks));
        state.granted.insert(id);
    }
}

/// Locks held; dropping it releases them and runs the requests they blocked.
pub struct Held {
    scheduler: Arc<Scheduler>,
    id: u64,
    locks: LockSet,
}

impl Held {
    pub fn locks(&self) -> &LockSet {
        &self.locks
    }

    /// The id of this hold, which also names its write for the watchers' pause.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// `locks` from inside this hold: what it already grants is not asked again; `None` when it grants all.
    /// Panics on an upgrade (a read held, its write asked), which would wait for itself.
    pub fn nested(&self, locks: LockSet) -> Option<Held> {
        let more = locks.beyond(&self.locks);
        assert!(!more.conflicts(&self.locks), "a nested lock upgrades one already held: {more:?} in {:?}", self.locks);
        (!more.is_empty()).then(|| self.scheduler.acquire(more))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.scheduler.release(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_excludes_its_read_and_reads_share() {
        let read = LockSet::refresh();
        let write = LockSet::new().write(Resource::Index);
        assert!(write.conflicts(&read) && read.conflicts(&write) && write.conflicts(&write));
        assert!(!read.conflicts(&read));
        assert!(!LockSet::new().write(Resource::Config).conflicts(&read));
    }

    #[test]
    fn a_nested_hold_asks_only_what_the_outer_does_not_grant() {
        let s = Arc::new(Scheduler::default());
        let outer = s.acquire(LockSet::commit());
        assert!(outer.nested(LockSet::refresh()).is_none(), "a write grants its read");
        let more = outer.nested(LockSet::commit().write(Resource::Remote)).expect("the remote is not held");
        assert_eq!(more.locks(), &LockSet::new().write(Resource::Remote));
    }
}

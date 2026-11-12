//! A write's Cancel and what a cancelled write leaves in the checkout.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Dirs, OperationState, operation_state};

/// The Cancel of one [`super::Writer::run_with`] job: once set, its git is killed and its later calls refused.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What a cancelled write left: the operation under way and the checkout's `index.lock`, which Supraman never deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left {
    pub state: OperationState,
    pub index_lock: Option<PathBuf>,
}

/// What the checkout of `dirs` holds now; blocking.
pub fn left(dirs: &Dirs) -> Left {
    let lock = dirs.git_dir.join("index.lock");
    Left { state: operation_state(&dirs.git_dir), index_lock: lock.exists().then_some(lock) }
}

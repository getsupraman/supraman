//! The app's loop over [`Keeping`]: kept Workman tokens checked at start, every [`CHECK_EVERY`] and after a sign-in.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::BackgroundExecutor;
use supraman_supervisor::{Bearer, CHECK_EVERY, HttpMinter, Keeper, Keeping, PING_TIMEOUT, bearer};
use supraman_ui::nudge::{self, Nudge};

static NUDGE: OnceLock<Nudge> = OnceLock::new();
static SIGNED_IN: AtomicBool = AtomicBool::new(false);

pub fn start(home: PathBuf, executor: &BackgroundExecutor) {
    let (nudge, mut nudged) = nudge::nudge();
    if NUDGE.set(nudge).is_err() {
        return;
    }
    let executor = executor.clone();
    executor
        .clone()
        .spawn(async move {
            let keeper = Keeper::open(&home);
            let auth = crate::signin::auth(&home);
            let mut keeping = Keeping::default();
            loop {
                if SIGNED_IN.swap(false, Ordering::SeqCst) {
                    keeping.signed_in();
                }
                let now = supraman_core::time::now_ms();
                let refused = |url: &str, token: &str| supraman_workman::refuses(url, token, PING_TIMEOUT);
                for notice in keeping.check(&keeper, &HttpMinter::default(), || Bearer::of(bearer(&auth)), now, refused) {
                    log::warn!("{notice}");
                }
                nudged.wait(&executor, CHECK_EVERY).await;
            }
        })
        .detach();
}

/// A sign-in finished: the loop mints every kept token afresh now.
pub fn signed_in() {
    SIGNED_IN.store(true, Ordering::SeqCst);
    if let Some(nudge) = NUDGE.get() {
        nudge.wake();
    }
}

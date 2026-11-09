//! A coalescing wake-up for a loop on a gpui executor: the loop sleeps until it is nudged or its
//! timeout passes, instead of waking on a timer to look at a flag.

use std::time::Duration;

use futures::channel::mpsc;
use futures::future::{Either, select};
use futures::{FutureExt as _, StreamExt as _};
use gpui::BackgroundExecutor;

/// The sending half; cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct Nudge(mpsc::UnboundedSender<()>);

/// The waiting half, owned by the one loop it wakes.
pub struct Nudged(mpsc::UnboundedReceiver<()>);

pub fn nudge() -> (Nudge, Nudged) {
    let (tx, rx) = mpsc::unbounded();
    (Nudge(tx), Nudged(rx))
}

impl Nudge {
    pub fn wake(&self) {
        self.0.unbounded_send(()).ok();
    }

    /// Its [`Nudged`] is gone: waking it reaches nobody.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

impl Nudged {
    /// Whether a nudge came since the last wait, without waiting; nudges that piled up count once.
    pub fn take(&mut self) -> bool {
        let mut woken = false;
        while self.0.try_recv().is_ok() {
            woken = true;
        }
        woken
    }

    /// Until a nudge or `timeout`; `true` when nudged. With every [`Nudge`] gone it waits out the timeout.
    pub async fn wait(&mut self, executor: &BackgroundExecutor, timeout: Duration) -> bool {
        if self.take() {
            return true;
        }
        let timer = executor.timer(timeout);
        let woken = match select(self.0.next().fuse(), timer).await {
            Either::Left((Some(()), _)) => true,
            Either::Left((None, timer)) => {
                timer.await;
                false
            }
            Either::Right(((), _)) => false,
        };
        self.take();
        woken
    }
}

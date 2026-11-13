//! The app's loop over [`Keeping`]: kept Workman tokens checked at start, every [`CHECK_EVERY`] and after a sign-in;
//! a failed renewal is a notice ([`Renewals`]) until that session's next renewal succeeds or its record goes.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Entity, Global};
use supraman_supervisor::{Bearer, CHECK_EVERY, Checked, HttpMinter, Keeper, Keeping, PING_TIMEOUT, bearer};
use supraman_ui::nudge::{self, Nudge};

static NUDGE: OnceLock<Nudge> = OnceLock::new();
static SIGNED_IN: AtomicBool = AtomicBool::new(false);

/// The failed renewals of kept Workman tokens, one notice per session.
#[derive(Debug, Default)]
pub struct Renewals {
    failed: BTreeMap<String, String>,
    dismissed: Vec<String>,
}

struct GlobalRenewals(Entity<Renewals>);

impl Global for GlobalRenewals {}

impl Renewals {
    /// A failure replaces its session's notice; a success or a record gone removes it. Whether a notice changed.
    pub fn apply(&mut self, checked: &Checked) -> bool {
        let before = self.failed.clone();
        for (name, outcome) in &checked.renewed {
            match outcome {
                Ok(()) => drop(self.failed.remove(name)),
                Err(e) => drop(self.failed.insert(name.clone(), format!("Cannot renew the Workman token of {name}: {e}"))),
            }
        }
        self.failed.retain(|name, _| checked.kept.contains(name));
        let current: Vec<String> = self.failed.values().cloned().collect();
        supraman_ui::notices::keep_dismissed(&mut self.dismissed, &current);
        self.failed != before
    }

    pub fn notices(&self) -> impl Iterator<Item = &String> {
        self.failed.values().filter(|t| !self.dismissed.contains(t))
    }

    pub fn dismiss(&mut self, text: &str) {
        self.dismissed.push(text.to_string());
    }
}

/// The window's renewal notices; none before [`start`].
pub fn notices(cx: &App) -> Vec<String> {
    cx.try_global::<GlobalRenewals>().map(|g| g.0.read(cx).notices().cloned().collect()).unwrap_or_default()
}

pub fn dismiss(text: &str, cx: &mut App) {
    let Some(renewals) = cx.try_global::<GlobalRenewals>().map(|g| g.0.clone()) else {
        return;
    };
    renewals.update(cx, |r, cx| {
        r.dismiss(text);
        cx.notify();
    });
}

pub fn start(home: PathBuf, cx: &mut App) {
    let (nudge, mut nudged) = nudge::nudge();
    if NUDGE.set(nudge).is_err() {
        return;
    }
    let renewals = cx.new(|_| Renewals::default());
    cx.set_global(GlobalRenewals(renewals.clone()));
    let (checks, mut checked) = mpsc::unbounded::<Checked>();
    let executor = cx.background_executor().clone();
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
                let outcome = keeping.check(&keeper, &HttpMinter::default(), || Bearer::of(bearer(&auth)), now, refused);
                for notice in &outcome.notices {
                    log::warn!("{notice}");
                }
                for (name, e) in outcome.renewed.iter().filter_map(|(n, r)| Some((n, r.as_ref().err()?))) {
                    log::warn!("Cannot renew the Workman token of {name}: {e}");
                }
                if checks.unbounded_send(outcome).is_err() {
                    return;
                }
                nudged.wait(&executor, CHECK_EVERY).await;
            }
        })
        .detach();
    cx.spawn(async move |cx| {
        while let Some(outcome) = checked.next().await {
            let _ = renewals.update(cx, |r, cx| {
                if r.apply(&outcome) {
                    cx.notify();
                }
            });
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

#[cfg(test)]
mod tests;

//! The update check's loop and what the window shows of it: a banner when a newer release is
//! available (view/left.rs) and a line on Settings → About.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::Context;
use supraman_core::update::Version;
use supraman_ui::nudge::{self, Nudge};
use supraman_workstate::updates::{self, Available, Checked, Outcome, Source};

use crate::view::{FleetView, Link, link_refused};

/// What Update does with the release the banner names; public/9 puts the download and the swap here.
pub type UpdateAction = fn(&mut FleetView, &Available, &mut Context<FleetView>);

pub fn open_release_page(view: &mut FleetView, available: &Available, cx: &mut Context<FleetView>) {
    view.open_web(&available.page_url.clone(), cx);
}

pub struct Updates {
    /// The selftest points it at its fake releases endpoint.
    pub source: Arc<Mutex<Source>>,
    nudge: Nudge,
    pub last: Option<Checked>,
    pub checking: bool,
    /// Checks answered since start, sent or not.
    pub checks: u64,
    pub dismissed: Option<Version>,
    pub action: UpdateAction,
}

impl Updates {
    /// The check at start, after the window is up, then every [`updates::EVERY`] or when nudged; one at a time.
    pub fn start(home: Option<PathBuf>, cx: &mut Context<FleetView>) -> Self {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let running = updates::running_version(env!("CARGO_PKG_VERSION"), exe_dir.as_deref());
        let source = Arc::new(Mutex::new(Source::github(running)));
        let (nudge, mut nudged) = nudge::nudge();
        if let Some(home) = home.clone() {
            let shared = source.clone();
            cx.spawn(async move |this, cx| {
                let executor = cx.background_executor().clone();
                loop {
                    let source = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let started = this.update(cx, |this, cx| {
                        this.updates.checking = true;
                        cx.notify();
                    });
                    if started.is_err() {
                        return;
                    }
                    let at = home.clone();
                    let checked = executor.spawn(async move { updates::check_if_on(&at, &source) }).await;
                    let alive = this.update(cx, |this, cx| {
                        match checked.as_ref().map(|c| &c.outcome) {
                            Some(Outcome::Failed { reason }) => log::warn!("update check: {reason}"),
                            Some(Outcome::Available(a)) => log::info!("update check: Supraman {} is available", a.version()),
                            Some(Outcome::UpToDate) => log::info!("update check: up to date"),
                            None => {}
                        }
                        let u = &mut this.updates;
                        u.checking = false;
                        u.checks += 1;
                        if checked.is_some() {
                            u.last = checked;
                        }
                        cx.notify();
                    });
                    if alive.is_err() {
                        return;
                    }
                    nudged.wait(&executor, updates::EVERY).await;
                }
            })
            .detach();
        }
        let (last, dismissed) = home.as_deref().map_or((None, None), |h| (updates::last(h), updates::dismissed(h)));
        Self { source, nudge, last, checking: false, checks: 0, dismissed, action: open_release_page }
    }

    pub fn check_now(&self) {
        self.nudge.wake();
    }

    pub fn running(&self) -> Version {
        self.source.lock().unwrap_or_else(|e| e.into_inner()).running.clone()
    }

    /// The release the banner offers: newer than this build and not dismissed.
    pub fn offered(&self) -> Option<&Available> {
        let Some(Checked { outcome: Outcome::Available(a), .. }) = &self.last else { return None };
        (a.manifest.semver() > self.running() && updates::shows(a, self.dismissed.as_ref())).then_some(a)
    }

    /// Settings → About's line: "Up to date, checked 10:42", "Supraman 0.2.0 is available" or why it failed.
    pub fn status_line(&self, on: bool) -> String {
        if !on {
            return "Checking is off.".into();
        }
        if self.checking {
            return "Checking…".into();
        }
        let Some(last) = &self.last else { return "Not checked yet.".into() };
        let at = chrono::DateTime::from_timestamp_millis(last.at_ms)
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_default();
        match &last.outcome {
            Outcome::Available(a) if a.manifest.semver() > self.running() => format!("Supraman {} is available", a.version()),
            Outcome::Available(_) | Outcome::UpToDate => format!("Up to date, checked {at}"),
            Outcome::Failed { reason } => format!("The check at {at} failed: {reason}"),
        }
    }
}

impl FleetView {
    pub fn check_updates_now(&self) {
        self.updates.check_now();
    }

    /// Opens an http(s) URL; anything else is refused with a toast.
    pub fn open_web(&mut self, url: &str, cx: &mut Context<Self>) {
        let scheme = url.split_once(':').map_or("", |(s, _)| s).to_ascii_lowercase();
        if scheme == "http" || scheme == "https" {
            (self.open_link_with)(&Link::Url(url.to_string()), cx);
        } else {
            self.pane_toast.show_why(link_refused(&scheme), |v: &mut Self| &mut v.pane_toast, cx);
        }
    }

    pub fn update_notes(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.updates.offered().map(|a| a.notes_url.clone()) {
            self.open_web(&url, cx);
        }
    }

    pub fn run_update(&mut self, cx: &mut Context<Self>) {
        if let Some(available) = self.updates.offered().cloned() {
            (self.updates.action)(self, &available, cx);
        }
    }

    pub fn dismiss_update(&mut self, cx: &mut Context<Self>) {
        let Some(version) = self.updates.offered().map(|a| a.version().to_string()) else { return };
        if let Some(home) = &self.home
            && let Err(e) = updates::dismiss(home, &version)
        {
            self.action_error = Some(format!("Cannot keep the dismissed update: {e}"));
        }
        self.updates.dismissed = Version::parse(&version).ok();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use supraman_core::update::Manifest;

    use super::*;

    fn available(version: &str) -> Available {
        Available {
            manifest: Manifest {
                version: version.into(),
                commit: "c52996b".into(),
                zip: format!("Supraman-{version}.zip"),
                sha256: "ab".repeat(32),
                size: 9,
                published: "2026-11-02T10:00:00Z".into(),
                notes_url: None,
            },
            page_url: "https://example.test/r".into(),
            notes_url: "https://example.test/r".into(),
            zip_url: None,
        }
    }

    fn updates(running: &str, outcome: Option<Outcome>) -> Updates {
        let source = Source { api: String::new(), repo: String::new(), keys: Err(String::new()), running: Version::parse(running).unwrap() };
        Updates {
            source: Arc::new(Mutex::new(source)),
            nudge: nudge::nudge().0,
            last: outcome.map(|outcome| Checked { at_ms: 0, outcome }),
            checking: false,
            checks: 0,
            dismissed: None,
            action: open_release_page,
        }
    }

    #[test]
    fn only_a_newer_release_not_dismissed_is_offered() {
        let mut u = updates("0.1.0", Some(Outcome::Available(available("0.2.0"))));
        assert_eq!(u.offered().map(Available::version), Some("0.2.0"));
        assert!(u.status_line(true).starts_with("Supraman 0.2.0 is available"));
        u.dismissed = Version::parse("0.2.0").ok();
        assert!(u.offered().is_none());
        let installed = updates("0.2.0", Some(Outcome::Available(available("0.2.0"))));
        assert!(installed.offered().is_none());
        assert!(installed.status_line(true).starts_with("Up to date, checked "));
    }

    #[test]
    fn the_status_line_says_off_checking_never_or_why() {
        let mut u = updates("0.1.0", None);
        assert_eq!(u.status_line(false), "Checking is off.");
        assert_eq!(u.status_line(true), "Not checked yet.");
        u.checking = true;
        assert_eq!(u.status_line(true), "Checking…");
        let failed = updates("0.1.0", Some(Outcome::Failed { reason: "no release is published yet".into() }));
        assert!(failed.status_line(true).ends_with("failed: no release is published yet"), "{}", failed.status_line(true));
    }
}

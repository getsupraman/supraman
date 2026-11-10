//! What the window starts with: `settings.toml`, the stars and the dismissals read off the UI thread and applied after, and the poll's loop.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gpui::{AsyncApp, Context, SharedString, WeakEntity, px};
use supraman_claude::check_supported;
use supraman_core::Settings;
use supraman_core::store::Save;
use supraman_ui::nudge::Nudged;
use supraman_ui::timeline;

use super::{Cli, FleetView, Poller, REFRESH_EVERY, Refresh, any_exit};
use crate::dismissed_messages::DismissedMessages;
use crate::stars::Stars;
use crate::supervise::Report;

const RUNS_DISMISSED: &str = "schedules/dismissed.json";
const WAITING_DISMISSED: &str = "schedules/waiting-dismissed.json";

struct Stores {
    stars: (Stars, Option<String>),
    messages: (DismissedMessages, Option<String>),
    runs: (DismissedMessages, Option<String>),
    waiting: (DismissedMessages, Option<String>),
}

impl Stores {
    fn load(home: &Path) -> Self {
        Self {
            stars: Stars::load(Some(home)),
            messages: DismissedMessages::load(Some(home)),
            runs: DismissedMessages::load_at(Some(home.join(RUNS_DISMISSED)), "every failed scheduled run shows"),
            waiting: DismissedMessages::load_at(Some(home.join(WAITING_DISMISSED)), "every schedule that waits for approval shows"),
        }
    }
}

fn absorbed(mine: &mut DismissedMessages, loaded: DismissedMessages) -> Option<Save> {
    let early = std::mem::replace(mine, loaded);
    mine.absorb(&early)
}

impl FleetView {
    // until they land nothing is starred or dismissed; a star or dismissal made meanwhile joins them
    pub(super) fn load_stores(home: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(home) = home else { return };
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_executor().spawn(async move { Stores::load(&home) }).await;
            this.update(cx, |this, cx| this.stores_loaded(loaded, cx)).ok();
        })
        .detach();
    }

    fn stores_loaded(&mut self, loaded: Stores, cx: &mut Context<Self>) {
        let Stores { stars, messages, runs, waiting } = loaded;
        let early = std::mem::replace(&mut self.stars, stars.0);
        let absorbed_stars = self.stars.absorb(&early);
        let save = self.stars.follow(&self.transcripts.continued).or(absorbed_stars);
        self.save_off_thread(save, "Cannot save the starred sessions", cx);
        let save = absorbed(&mut self.dismissed_messages, messages.0);
        self.save_off_thread(save, "Cannot save the dismissed messages", cx);
        let save = absorbed(&mut self.scheduling.runs_dismissed, runs.0);
        self.save_off_thread(save, "Cannot save the dismissed runs", cx);
        let save = absorbed(&mut self.scheduling.waiting_dismissed, waiting.0);
        self.save_off_thread(save, "Cannot save the dismissed approvals", cx);
        self.action_error = stars.1.or(messages.1).or(runs.1).or(waiting.1).or(self.action_error.take());
        if !self.stars.ids().is_empty() {
            self.polls.transcripts_nudge.wake();
        }
        self.invalidate_tree();
        cx.notify();
    }

    // until it lands the window shows what an unreadable settings.toml gives: the defaults
    pub(super) fn load_settings(home: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(home) = home else { return };
        cx.spawn(async move |this, cx| {
            let Ok(loaded) = cx.background_executor().spawn(async move { Settings::load(&home) }).await else { return };
            this.update(cx, |this, cx| this.settings_loaded(loaded, cx)).ok();
        })
        .detach();
    }

    fn settings_loaded(&mut self, loaded: Settings, cx: &mut Context<Self>) {
        if let Some(width) = self.sidebar.left_saves.read(cx).loaded(loaded.window.left_width) {
            self.sidebar.left_width = width;
        }
        self.term_look.term_font_size = px(loaded.terminal.font_size() as f32);
        self.term_look.term_font_family = SharedString::from(loaded.terminal.font_family(&self.term_look.installed_fonts));
        self.term_look.term_ligatures = loaded.terminal.ligatures();
        self.term_look.shell_default.read(loaded.terminal.clone());
        let mode = loaded.window.tab_keys();
        self.tab_keys.update(cx, |keys, _| keys.mode = mode);
        cx.notify();
    }

    pub(super) fn start_polling(poller: Poller, mut woken: Nudged, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let Some((mut poller, supported, refresh)) = first_poll(&this, poller, cx).await else { return };
            let mut first = Some(refresh);
            loop {
                let Ok(asks) = this.read_with(cx, |this, _| this.polls.refresh_asks) else {
                    return;
                };
                let refresh = match first.take() {
                    Some(refresh) => refresh,
                    None => {
                        let (done, refresh) = executor
                            .spawn(async move {
                                let refresh = poller.read(supported);
                                (poller, refresh)
                            })
                            .await;
                        poller = done;
                        refresh
                    }
                };
                if this.update(cx, |this, cx| this.applied_poll(refresh, cx)).is_err() {
                    return;
                }
                let (done, report) = executor
                    .spawn(async move {
                        let report = poller.tick();
                        (poller, report)
                    })
                    .await;
                poller = done;
                let Ok(pids) = this.update(cx, |this, cx| this.ticked(report, asks, cx)) else {
                    return;
                };
                let exited = std::pin::pin!(any_exit(&executor, pids, REFRESH_EVERY));
                futures::future::select(std::pin::pin!(woken.wait(&executor, REFRESH_EVERY)), exited).await;
            }
        })
        .detach();
    }

    fn applied_poll(&mut self, refresh: Refresh, cx: &mut Context<Self>) {
        self.writes.max.tick_starts();
        self.apply(refresh, cx);
        self.refresh_details(cx);
        self.read_tab_branches(cx);
        if self.scheduling.other_scans == 0 {
            self.scan_other_schedules(cx);
        }
        cx.notify();
    }

    // the interactive sessions' pids: an exit wakes the next poll
    fn ticked(&mut self, report: Option<Report>, asks: u64, cx: &mut Context<Self>) -> Vec<u32> {
        if let Some(report) = report {
            self.apply_report(report, cx);
        }
        self.polls.refreshes += 1;
        self.polls.refreshes_answered = asks;
        self.check_loaded(cx);
        let left = std::mem::take(&mut self.workman_state.left_pending);
        self.leave_servers(left, cx);
        self.resolve_tabs(cx);
        self.reopen_tabs(cx);
        let pids: Vec<u32> = self.sessions.iter().filter(|s| s.kind == supraman_claude::Kind::Interactive).filter_map(|s| s.pid).collect();
        if !self.removing.is_empty() {
            self.finish_removals(cx);
        }
        let named = self.workspaces.iter().map(|w| (w.name.clone(), w.folders.iter().map(|f| f.path.clone()).collect())).collect();
        self.work_inbox.update(cx, |inbox, cx| inbox.set_workspaces(named, cx));
        cx.notify();
        pids
    }
}

async fn first_poll(this: &WeakEntity<FleetView>, mut poller: Poller, cx: &mut AsyncApp) -> Option<(Poller, bool, Refresh)> {
    let executor = cx.background_executor().clone();
    let checking = Instant::now();
    let checked = executor.spawn(async move {
        let checked = check_supported();
        timeline("version check done", checking.elapsed());
        checked
    });
    let first = executor.spawn(async move {
        let setting_up = Instant::now();
        let setup = poller.setup();
        timeline("setup done", setting_up.elapsed());
        let refresh = poller.read(true);
        (poller, setup, refresh)
    });
    let (checked, (mut poller, setup, mut refresh)) = futures::join!(checked, first);
    let supported = checked.is_ok();
    if !supported {
        refresh.sessions = None;
        poller.snapshot = None;
    }
    this.update(cx, |this, cx| {
        this.cli = match checked {
            Ok(version) => Cli::Supported(version),
            Err(error) => Cli::Failed(error.to_string()),
        };
        this.plugin_dir = setup.plugin_dir;
        this.setup_notices = setup.notices;
        cx.notify();
    })
    .ok()?;
    Some((poller, supported, refresh))
}

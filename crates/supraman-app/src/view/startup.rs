//! What the window starts with: `settings.toml` read off the UI thread and applied after, and the poll's loop.

use std::path::PathBuf;
use std::time::Instant;

use gpui::{AsyncApp, Context, SharedString, WeakEntity, px};
use supraman_claude::check_supported;
use supraman_core::Settings;
use supraman_ui::nudge::Nudged;
use supraman_ui::timeline;

use super::{Cli, FleetView, MIN_LEFT_WIDTH, Poller, REFRESH_EVERY, Refresh, any_exit};
use crate::supervise::Report;

impl FleetView {
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
        if let Some(width) = loaded.window.left_width {
            self.left_width = px(width as f32).max(MIN_LEFT_WIDTH);
        }
        self.term_font_size = px(loaded.terminal.font_size() as f32);
        self.term_font_family = SharedString::from(loaded.terminal.font_family(&self.installed_fonts));
        self.term_ligatures = loaded.terminal.ligatures();
        self.shell_default.read(loaded.terminal.clone());
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
                let Ok(asks) = this.read_with(cx, |this, _| this.refresh_asks) else {
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
        if self.other_scans == 0 {
            self.scan_other_schedules(cx);
        }
        cx.notify();
    }

    // the interactive sessions' pids: an exit wakes the next poll
    fn ticked(&mut self, report: Option<Report>, asks: u64, cx: &mut Context<Self>) -> Vec<u32> {
        if let Some(report) = report {
            self.apply_report(report, cx);
        }
        self.refreshes += 1;
        self.refreshes_answered = asks;
        self.check_loaded(cx);
        let left = std::mem::take(&mut self.left_pending);
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

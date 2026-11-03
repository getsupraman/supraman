use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::{Session, now_ms};
use gpui::{AsyncWindowContext, Entity};

use crate::{
    fleet,
    saved_tabs::{self, SavedTab, SavedTabs},
    selftest::{WORKSPACE, click, hex6, remove_dir_retrying, sleep, until, until_app, wait_for_tab_terminal},
    selftest_closed::{RECENT, fake_claude, seen_file},
    view::{Detach, FleetView, TabKind},
};

const WITHIN: Duration = Duration::from_secs(15);
/// `claude attach`'s Ctrl+Z detach of the one attached tab, plus the shells' exits.
const CLOSED_WITHIN: Duration = Duration::from_secs(20);

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, own: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.claude_command, format!("& '{}'", fake_claude(home).display())));
    let gone = crate::selftest::home().join(format!("fm-selftest-reopen-{}", hex6()));
    let result = check(view, home, own, &gone, cx).await;
    let closed = close_all(view, false, cx).await;
    view.update(cx, |v, cx| {
        v.claude_command = saved;
        v.tab_notices.clear();
        cx.notify();
    });
    let reset = supraman_core::Settings::update(home, |s| s.sessions.resume_tabs_on_click = false).map_err(|e| anyhow!("settings.toml: {e}"));
    std::fs::remove_dir_all(&gone).ok();
    let result = result?;
    closed?;
    reset?;
    Ok(result)
}

/// An attach tab is closed once its `claude attach` runs, so its Ctrl+Z detach is not typed before it reads keys; a failed detach is retried.
async fn close_all(view: &Entity<FleetView>, unsaved: bool, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().filter(|t| matches!(t.kind, TabKind::Attach { .. })).map(|t| t.id).collect());
    for id in ids {
        if let Ok(terminal) = wait_for_tab_terminal(view, id, WITHIN, cx).await {
            let started = Instant::now();
            while terminal.read_with(cx, |t, _| t.get_content()).trim().is_empty() && started.elapsed() < WITHIN {
                sleep(cx, Duration::from_millis(50)).await;
            }
        }
    }
    if unsaved {
        view.update(cx, |v, cx| v.close_tabs_unsaved(cx));
    }
    let started = Instant::now();
    loop {
        let open: Vec<(u64, bool)> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| (t.id, matches!(t.detach, Detach::None | Detach::Failed))).collect());
        if open.is_empty() {
            return Ok(());
        }
        if started.elapsed() > CLOSED_WITHIN {
            bail!("tabs {open:?} still open {} s after their close", CLOSED_WITHIN.as_secs());
        }
        view.update(cx, |v, cx| open.iter().filter(|(_, closable)| *closable).for_each(|(id, _)| v.close_tab(*id, cx)));
        sleep(cx, Duration::from_millis(250)).await;
    }
}

fn kinds(saved: &SavedTabs) -> Vec<&'static str> {
    saved
        .tabs
        .iter()
        .map(|t| match t {
            SavedTab::Shell { .. } => "shell",
            SavedTab::Claude { .. } => "claude",
            SavedTab::Attach { .. } => "attach",
            SavedTab::Work { .. } => "work",
        })
        .collect()
}

async fn fake_ran(home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let seen = seen_file(home);
    let deadline = Instant::now() + WITHIN;
    loop {
        let text = std::fs::read_to_string(&seen).unwrap_or_default();
        if text.lines().count() >= 2 {
            let args = text.lines().next().unwrap_or_default().trim().to_string();
            if !args.starts_with(&format!("--resume {RECENT}")) {
                bail!("the fake claude got `{args}`, not --resume {RECENT}");
            }
            return Ok(args);
        }
        if Instant::now() > deadline {
            bail!("the fake claude never ran (no {} within {} s)", seen.display(), WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
}

/// `gone` is deleted once its shell has closed.
async fn restart(view: &Entity<FleetView>, home: &Path, expected: usize, gone: Option<&Path>, cx: &mut AsyncWindowContext) -> anyhow::Result<Duration> {
    let on_disk = saved_tabs::load(home).0;
    close_all(view, true, cx).await.context("every tab closed as a quit closes them")?;
    if saved_tabs::load(home).0 != on_disk {
        bail!("closing the tabs for the restart changed {}", saved_tabs::path(home).display());
    }
    if let Some(gone) = gone {
        remove_dir_retrying(gone).context("the doomed tab's folder")?;
    }
    std::fs::remove_file(seen_file(home)).ok();
    let started = Instant::now();
    view.update(cx, |v, cx| v.restart_tabs(cx));
    until(view, cx, WITHIN, &format!("{expected} tabs reopened"), |v| (v.tabs_restored && v.tabs.len() == expected).then_some(())).await?;
    Ok(started.elapsed())
}

async fn check(view: &Entity<FleetView>, home: &Path, own: &str, gone: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    close_all(view, false, cx).await.context("the earlier tabs closed")?;
    std::fs::create_dir_all(gone).context("the folder that goes away")?;
    std::fs::remove_file(seen_file(home)).ok();

    let live: Session = until(view, cx, WITHIN, &format!("{own} live"), |v| {
        v.sessions.iter().find(|s| s.name == own && fleet::is_alive(&s.health(now_ms()))).cloned()
    })
    .await?;
    let closed: Session = until(view, cx, WITHIN, &format!("{RECENT} among the closed sessions"), |v| v.closed.iter().find(|s| s.session_id == RECENT).cloned()).await?;
    let cwd = crate::selftest::home();
    let (shell, hand, work, attach, doomed) = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            let shell = v.open_shell_in(cwd.clone(), "reopen shell".into(), window, cx);
            let hand = v.resume(WORKSPACE, "reopen", &closed, window, cx);
            let work = v.open_work(crate::selftest_work::WORKSPACE, cx);
            let attach = v.attach(WORKSPACE, "reopen", &live, window, cx);
            let doomed = v.open_shell_in(gone.to_path_buf(), "reopen gone".into(), window, cx);
            // the order a drag leaves: Work first
            supraman_ui::tabs::reorder(&mut v.tabs, 2, 0);
            v.activate_tab(hand, cx);
            (shell, hand, work, attach, doomed)
        })
    })?;
    for id in [shell, hand, attach, doomed] {
        wait_for_tab_terminal(view, id, WITHIN, cx).await?;
    }
    fake_ran(home, cx).await?;
    let saved = until_app(view, cx, WITHIN, "the five tabs in tabs.json", |v, cx| {
        let now = v.saved_tabs(cx);
        (now.tabs.len() == 5 && saved_tabs::load(home).0 == now).then_some(now)
    })
    .await?;
    let order = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect::<Vec<_>>());
    if order != [work, shell, hand, attach, doomed] || kinds(&saved) != ["work", "shell", "claude", "attach", "shell"] || saved.active != Some(2) {
        bail!("before the restart: tabs {order:?}, saved {:?} active {:?}", kinds(&saved), saved.active);
    }
    match &saved.tabs[2] {
        SavedTab::Claude { session_id: Some(sid), .. } if sid == RECENT => {}
        other => bail!("the claude tab is saved as {other:?}, not with the full uuid {RECENT}"),
    }

    let took = restart(view, home, 4, Some(gone), cx).await?;
    let mut expected = saved.clone();
    expected.tabs.remove(4);
    let (now, notices) = view.read_with(cx, |v, cx| (v.saved_tabs(cx), v.tab_notices.clone()));
    if now != expected {
        bail!("after the restart {now:?}, expected {expected:?}");
    }
    if notices.len() != 1 || !notices[0].contains("reopen gone") || !notices[0].contains("is gone") {
        bail!("the tab whose folder went is not named in one notice: {notices:?}");
    }
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    let active = view.read_with(cx, |v, _| v.active);
    if active != Some(ids[2]) {
        bail!("active {active:?} after the restart, not the claude tab {}", ids[2]);
    }
    for &id in &ids[1..] {
        wait_for_tab_terminal(view, id, WITHIN, cx).await?;
    }
    let args = fake_ran(home, cx).await?;
    let attached = view.read_with(cx, |v, _| v.tab(ids[3]).map(|t| matches!(&t.kind, TabKind::Attach { session_id } if *session_id == live.id)));
    if attached != Some(true) || !view.read_with(cx, |v, _| v.tab(ids[2]).is_some_and(|t| t.hand)) {
        bail!("the reopened attach or claude tab is of another kind");
    }

    supraman_core::Settings::update(home, |s| s.sessions.resume_tabs_on_click = true).map_err(|e| anyhow!("settings.toml: {e}"))?;
    view.update(cx, |v, _| v.tab_notices.clear());
    restart(view, home, 4, None, cx).await?;
    let placeholder = view.read_with(cx, |v, _| v.tabs.get(2).filter(|t| matches!(t.kind, TabKind::Saved(_)) && t.terminal.is_none()).map(|t| t.id));
    let Some(placeholder) = placeholder else {
        bail!("with Resume on a click the claude tab is not a placeholder");
    };
    let others: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().filter(|t| matches!(t.kind, TabKind::Shell | TabKind::Attach { .. })).map(|t| t.id).collect());
    for id in others {
        let terminal = wait_for_tab_terminal(view, id, WITHIN, cx).await?;
        let started = Instant::now();
        while terminal.read_with(cx, |t, _| t.get_content()).trim().is_empty() {
            if started.elapsed() > WITHIN {
                bail!("the reopened tab {id} printed nothing within {} s", WITHIN.as_secs());
            }
            sleep(cx, Duration::from_millis(50)).await;
        }
    }
    let ran = view.read_with(cx, |v, _| v.tab(placeholder).is_none_or(|t| t.terminal.is_some()));
    if ran || seen_file(home).exists() {
        bail!("the placeholder ran claude before its Resume was clicked");
    }
    if view.read_with(cx, |v, cx| v.saved_tabs(cx)) != expected {
        bail!("the placeholder is not saved as the claude tab it stands for");
    }
    click(view, &format!("resume-tab-{placeholder}"), cx).await?;
    let resumed = until(view, cx, WITHIN, "Resume opens the claude tab in the placeholder's place", |v| {
        v.tabs.get(2).filter(|t| t.hand && t.id != placeholder && v.active == Some(t.id) && v.tabs.len() == 4).map(|t| t.id)
    })
    .await?;
    wait_for_tab_terminal(view, resumed, WITHIN, cx).await?;
    fake_ran(home, cx).await?;

    Ok(format!(
        "work, shell, claude ({RECENT}), attach ({}), shell in a folder then deleted, claude active, saved in order; \
         after a restart 4 came back in {} ms in the same order with the same active tab, the claude resumed (`{args}`), \
         the attach re-attached and one notice named the dropped tab; with Resume on a click the claude tab was a placeholder \
         that ran nothing until its Resume, which resumed it in place",
        live.id,
        took.as_millis()
    ))
}

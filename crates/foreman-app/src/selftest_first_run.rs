//! `--selftest` step 7d (m5/3): first run without WorkState. With no `workstate_env` and nothing
//! installed the notice above the tree names the setup skill and offers no `.env`; a fake install's
//! `.env` (under the scratch home through [`INSTALL_DIR_VAR`], never the real `~/.workstate`) is offered,
//! and one click writes it into `settings.toml`, after which the fake WorkState answers and the notice
//! goes. A closed port names the URL and the error with a Retry. The gate's settings are put back.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use foreman_core::Settings;
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{click, sleep},
    selftest_manage::{shot, shot_window},
    settings_window::Page,
    view::FleetView,
    workstate_setup::{INSTALL_DIR_VAR, SETUP_SKILL, WsStatus},
};

/// A recheck is picked up within 250 ms; `/health` gives up after 3 s.
const WITHIN: Duration = Duration::from_secs(10);

async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = view.read_with(cx, |v, _| f(v)) {
            return Ok(t);
        }
        if started.elapsed() > WITHIN {
            let now = view.read_with(cx, |v, _| format!("{:?}", v.workstate));
            bail!("{what}: not within {} s (status {now})", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.hits.borrow().contains_key(key)
}

fn save(home: &std::path::Path, edit: impl FnOnce(&mut Settings)) -> anyhow::Result<()> {
    let mut settings = Settings::load(home)?;
    edit(&mut settings);
    settings.save(home)?;
    Ok(())
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let home = crate::selftest::home();
    let saved = Settings::load(&home)?;
    let install = PathBuf::from(std::env::var_os(INSTALL_DIR_VAR).ok_or_else(|| anyhow!("{INSTALL_DIR_VAR} is not set"))?);
    if !install.starts_with(&home) {
        bail!("{} is not under the scratch home {}", install.display(), home.display());
    }
    let mut lines = Vec::new();

    // a. no workstate_env, nothing installed: the notice and the skill, no offer
    save(&home, |s| s.workstate_env = None)?;
    view.update(cx, |v, _| v.recheck_workstate());
    let problem = until(view, cx, "the not-set-up notice", |v| {
        (v.workstate == WsStatus::NoEnv { offer: None } && painted(v, "ws-setup-notice") && !painted(v, "ws-setup-use"))
            .then(|| v.workstate_notice())
            .flatten()
    })
    .await?;
    let fix = view.read_with(cx, |v, _| v.workstate.fix()).unwrap_or_default();
    if !fix.contains(SETUP_SKILL) {
        bail!("the fix does not name {SETUP_SKILL}: {fix:?}");
    }
    lines.push(format!("no workstate_env: notice \"{problem}\" with \"{fix}\", no offer"));

    // b. a fake install's .env: offered, and one click writes settings.toml
    let env = install.join(".env");
    std::fs::create_dir_all(&install)?;
    std::fs::write(&env, "# fake, selftest only\nWORKSTATE_CONTROLLER_TOKEN=fake-first-run-controller\nWORKSTATE_ORCHESTRATOR_TOKEN=fake-first-run-orchestrator\n")?;
    view.update(cx, |v, _| v.recheck_workstate());
    until(view, cx, "the offer of the installed .env", |v| (v.workstate.offer() == Some(env.as_path()) && painted(v, "ws-setup-use")).then_some(())).await?;
    shot("m5-3-first-run-offer", cx).await;
    // Settings → WorkState shows the same status, with the same offer
    click(view, "ws-setup-settings", cx).await?;
    let handle = until(view, cx, "Settings… opens the settings window", |v| v.settings_window).await?;
    let started = Instant::now();
    loop {
        let (page, shown) = handle.read_with(cx, |s, _| (s.page(), s.workstate().clone())).map_err(|e| anyhow!("settings window: {e}"))?;
        let fleet = view.read_with(cx, |v, _| v.workstate.clone());
        if page == Page::WorkState && shown == fleet {
            break;
        }
        if started.elapsed() > WITHIN {
            bail!("Settings shows {page:?} with {shown:?}, the notice {fleet:?}");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    shot_window("m5-3-settings-workstate", Some("Foreman Settings"), cx).await;
    view.update(cx, |v, cx| v.close_settings(cx));
    click(view, "ws-setup-use", cx).await?;
    let written = Settings::load(&home)?.workstate_env;
    if written.as_deref() != Some(env.as_path()) {
        bail!("Use wrote workstate_env = {written:?}, expected {}", env.display());
    }
    let url = until(view, cx, "Ready after Use, the notice gone", |v| match &v.workstate {
        WsStatus::Ready { url } if !painted(v, "ws-setup-notice") => Some(url.clone()),
        _ => None,
    })
    .await?;
    lines.push(format!("Settings → WorkState showed the same status; one click on Use wrote workstate_env = {}; {url} answers /health, notice gone", env.display()));

    // c. the server down: the URL, the error and Retry
    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let closed = format!("http://127.0.0.1:{port}/mcp");
    save(&home, |s| s.workstate_url = Some(closed.clone()))?;
    view.update(cx, |v, _| v.recheck_workstate());
    let down = until(view, cx, "the unreachable notice", |v| {
        (v.workstate.unreachable() && painted(v, "ws-setup-retry")).then(|| v.workstate_notice()).flatten()
    })
    .await?;
    if !down.contains(&closed) {
        bail!("the unreachable notice does not name {closed}: {down:?}");
    }
    let warning = view.read_with(cx, |v, _| v.workstate.start_warning("w"));
    if !warning.as_deref().is_some_and(|w| w.contains("unreachable")) {
        bail!("Start would not warn while unreachable: {warning:?}");
    }
    shot("m5-3-unreachable", cx).await;
    click(view, "ws-setup-dismiss", cx).await?;
    until(view, cx, "× hides the notice", |v| (!painted(v, "ws-setup-notice") && v.workstate.unreachable()).then_some(())).await?;
    lines.push(format!("closed port: \"{down}\", Retry shown, × hid it"));

    saved.save(&home)?;
    view.update(cx, |v, _| v.recheck_workstate());
    until(view, cx, "the gate's fake WorkState Ready again", |v| matches!(v.workstate, WsStatus::Ready { .. }).then_some(())).await?;
    Ok(lines.join("; "))
}

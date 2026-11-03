use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use supraman_core::Settings;
use supraman_workman::AuthStatus;
use supraman_workman::fake::{Answer, FakeWorkman};
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{click, sleep},
    selftest_manage::{shot, shot_window},
    settings_window::page,
    view::FleetView,
    workstate_setup::WsStatus,
};

/// A recheck is picked up within 250 ms; `/health` gives up after 3 s; a sign-in is a few requests.
const WITHIN: Duration = Duration::from_secs(15);

async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let result = crate::selftest::until(view, cx, WITHIN, what, f).await;
    result.map_err(|e| {
        let now = view.read_with(cx, |v, _| format!("status {:?}, sign-in error {:?}", v.workstate, v.sign_in_error));
        e.context(now)
    })
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let home = crate::selftest::home();
    let saved = Settings::load(&home)?;
    let (fake, oauth) = crate::selftest_work::fake().ok_or_else(|| anyhow!("step 1w started no fake Workman"))?;
    let base = fake.base().to_string();
    let auth = crate::signin::auth(&home);
    let mut lines = Vec::new();

    let who = until(view, cx, "Ready as the fake operator", |v| match &v.workstate {
        WsStatus::Ready { url, who } if *url == base => who.clone(),
        _ => None,
    })
    .await?;
    view.update(cx, |v, cx| v.open_settings_at(Some(page::WORKSTATE), cx));
    let handle = until(view, cx, "Settings opens on Workman", |v| v.settings_window).await?;
    let started = Instant::now();
    loop {
        let (page, shown) = handle.read_with(cx, |s, _| (s.page(), s.workstate().clone())).map_err(|e| anyhow!("settings window: {e}"))?;
        let fleet = view.read_with(cx, |v, _| v.workstate.clone());
        if page == page::WORKSTATE && shown == fleet {
            break;
        }
        if started.elapsed() > WITHIN {
            bail!("Settings shows {page:?} with {shown:?}, the fleet {fleet:?}");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    shot_window("m5-43-settings-signed-in", Some("Supraman Settings"), cx).await;
    lines.push(format!("Settings → Workman: signed in on {base} as {who}"));

    view.update(cx, |v, cx| v.sign_out(cx));
    let problem = until(view, cx, "the not-signed-in notice with Sign in", |v| {
        (matches!(v.workstate, WsStatus::NotSignedIn { .. }) && painted(v, "ws-setup-notice") && painted(v, "ws-setup-signin"))
            .then(|| v.workstate_notice())
            .flatten()
    })
    .await?;
    if auth.status(&base) != AuthStatus::NotSignedIn {
        bail!("after Sign out the sign-in still says {:?}", auth.status(&base));
    }
    shot("m5-43-not-signed-in", cx).await;
    lines.push(format!("Sign out: notice \"{problem}\" with Sign in"));

    let issued = oauth.issued().len();
    click(view, "ws-setup-signin", cx).await?;
    until(view, cx, "Ready after Sign in, the notice gone", |v| {
        (matches!(&v.workstate, WsStatus::Ready { url, .. } if *url == base) && !painted(v, "ws-setup-notice")).then_some(())
    })
    .await?;
    let fresh: Vec<String> = oauth.issued().into_iter().skip(issued).collect();
    if fresh.is_empty() {
        bail!("Sign in issued no token");
    }
    let rows = crate::selftest_work::inbox_keys();
    until(view, cx, "the tree's Workman rows", |v| rows.iter().all(|k| painted(v, k)).then_some(())).await?;
    let read = || {
        fake.seen().iter().any(|s| {
            s.target.starts_with("/api/v1/") && s.authorization.as_deref().and_then(|a| a.strip_prefix("Bearer ")).is_some_and(|t| fresh.iter().any(|f| f == t))
        })
    };
    let started = Instant::now();
    while !read() {
        if started.elapsed() > WITHIN {
            bail!("no read with the new sign-in's token");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    lines.push(format!("the notice's Sign in signed in again ({} new tokens), the Inbox paints {} Workman rows read with them", fresh.len(), rows.len()));

    let other = FakeWorkman::start(vec![("/health".into(), Answer::text("Healthy"))]);
    let other_oauth = other.enable_oauth();
    handle.update(cx, |s, _, cx| s.set_url_text(other.base(), cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    handle.update(cx, |s, _, cx| s.apply_url(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    until(view, cx, "Ready on the other server", |v| {
        matches!(&v.workstate, WsStatus::Ready { url, .. } if url == other.base()).then_some(())
    })
    .await?;
    if Settings::load(&home)?.workman.origin() != other.base() {
        bail!("the new URL was not saved after the sign-in there");
    }
    if other_oauth.issued().is_empty() {
        bail!("the other server issued nothing");
    }
    lines.push(format!("URL → {}: signed in there, saved", other.base()));

    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let closed = format!("http://127.0.0.1:{port}");
    handle.update(cx, |s, _, cx| s.set_url_text(&closed, cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    handle.update(cx, |s, _, cx| s.apply_url(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let error = until(view, cx, "the failed switch's error", |v| v.signing_in.is_none().then(|| v.sign_in_error.clone()).flatten()).await?;
    if !error.contains(&closed) || Settings::load(&home)?.workman.origin() != other.base() {
        bail!("a failed switch to {closed}: {error:?}, URL now {}", Settings::load(&home)?.workman.origin());
    }
    shot_window("m5-43-settings-switch-failed", Some("Supraman Settings"), cx).await;
    lines.push(format!("URL → {closed} (closed): \"{error}\", the URL stayed"));
    view.update(cx, |v, cx| v.close_settings(cx));

    Settings { workman: supraman_core::WorkmanSettings { url: Some(closed.clone()) }, ..saved.clone() }.save(&home)?;
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

    auth.sign_out(other.base()).map_err(|e| anyhow!("signing out of the other fake: {e}"))?;
    saved.save(&home)?;
    crate::signin::sign_in(&home, &base).map_err(|e| anyhow!("signing in to the gate's fake again: {e}"))?;
    view.update(cx, |v, cx| {
        v.recheck_workstate();
        crate::work_view::WorkHub::reconfigure(Some(home.clone()), cx);
    });
    until(view, cx, "the gate's fake Workman Ready again", |v| matches!(v.workstate, WsStatus::Ready { .. }).then_some(())).await?;
    Ok(lines.join("; "))
}

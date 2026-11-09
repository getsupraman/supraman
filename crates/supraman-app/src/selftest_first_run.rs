use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use supraman_workman::AuthStatus;
use supraman_workman::fake::{Answer, FakeWorkman};
use gpui::{AsyncWindowContext, Entity, WindowHandle};

use crate::{
    selftest::{click, sleep},
    selftest_manage::{shot, shot_window},
    settings_window::{SettingsView, page},
    view::FleetView,
};
use supraman_workman::status::WorkmanStatus;

/// A recheck is picked up within 250 ms; `/health` gives up after 3 s; a sign-in is a few requests.
const WITHIN: Duration = Duration::from_secs(15);

async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let result = crate::selftest::until(view, cx, WITHIN, what, f).await;
    result.map_err(|e| {
        let now = view.read_with(cx, |v, _| format!("status {:?}, sign-in error {:?}", v.workman, v.sign_in_error));
        e.context(now)
    })
}

/// The URL in the selftest's own server.json.
fn saved_url() -> anyhow::Result<String> {
    supraman_core::workman_server::load(&crate::selftest::workman_server_json()).map(|s| s.origin()).map_err(|e| anyhow!(e))
}

async fn settled<T>(
    handle: &WindowHandle<SettingsView>,
    cx: &mut AsyncWindowContext,
    what: &str,
    f: impl Fn(&SettingsView) -> Option<T>,
) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(found) = handle.read_with(cx, |s, _| f(s)).map_err(|e| anyhow!("settings window: {e}"))? {
            return Ok(found);
        }
        if started.elapsed() > WITHIN {
            let now = handle.read_with(cx, |s, _| format!("{:?}", s.server_check())).unwrap_or_default();
            bail!("Settings never showed {what}: {now}");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

/// Writes `<selftest home>/user/.claude.json` (the path `selftest::prepare` set in place of the user's): first
/// without a user-scope workman entry, then with one naming `base`'s /mcp and a header value nothing may read.
async fn claude_entry_checked(handle: &WindowHandle<SettingsView>, base: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let path = crate::selftest::claude_json();
    let project_only = format!(r#"{{"projects":{{"C:/x":{{"mcpServers":{{"workman":{{"type":"http","url":"{base}/mcp"}}}}}}}}}}"#);
    supraman_core::fs::write_atomic(&path, project_only).with_context(|| format!("cannot write {}", path.display()))?;
    handle.update(cx, |s, _, cx| s.check_server(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let missing = settled(handle, cx, "the warning of no user-scope workman entry", |s| {
        let c = s.server_check()?;
        (c.mcp_name.as_deref() == Some("workman")).then(|| c.warnings.iter().find(|w| w.contains("no user-scope MCP server named")).cloned()).flatten()
    })
    .await?;
    shot_window("workman-oauth-only-5-claude-entry-missing", Some("Supraman Settings"), cx).await;
    let user = format!(r#"{{"mcpServers":{{"workman":{{"type":"http","url":"{base}/mcp","headers":{{"Authorization":"Bearer selftest-never-read"}}}}}}}}"#);
    supraman_core::fs::write_atomic(&path, user).with_context(|| format!("cannot write {}", path.display()))?;
    handle.update(cx, |s, _, cx| s.check_server(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    settled(handle, cx, "no warning once claude's workman entry is this server's", |s| s.server_check().filter(|c| c.warnings.is_empty()).map(|_| ())).await?;
    Ok(format!("{}: \"{missing}\", then none for {base}/mcp", path.display()))
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let home = crate::selftest::home();
    let (fake, oauth) = crate::selftest_work::fake().ok_or_else(|| anyhow!("step 1w started no fake Workman"))?;
    let base = fake.base().to_string();
    let auth = crate::signin::auth(&home);
    let mut lines = Vec::new();

    let who = until(view, cx, "Ready as the fake operator", |v| match &v.workman {
        WorkmanStatus::Ready { url, who, .. } if *url == base => who.clone(),
        _ => None,
    })
    .await?;
    view.update(cx, |v, cx| v.open_settings_at(Some(page::WORKMAN), cx));
    let handle = until(view, cx, "Settings opens on Workman", |v| v.settings_window).await?;
    let started = Instant::now();
    loop {
        let (page, shown) = handle.read_with(cx, |s, _| (s.page(), s.workman().clone())).map_err(|e| anyhow!("settings window: {e}"))?;
        let fleet = view.read_with(cx, |v, _| v.workman.clone());
        if page == page::WORKMAN && shown == fleet {
            break;
        }
        if started.elapsed() > WITHIN {
            bail!("Settings shows {page:?} with {shown:?}, the fleet {fleet:?}");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    shot_window("m5-43-settings-signed-in", Some("Supraman Settings"), cx).await;
    lines.push(format!("Settings → Workman: signed in on {base} as {who}"));
    lines.push(claude_entry_checked(&handle, &base, cx).await?);

    view.update(cx, |v, cx| v.sign_out(cx));
    let problem = until(view, cx, "the not-signed-in notice with Sign in", |v| {
        (matches!(v.workman, WorkmanStatus::NotSignedIn { .. }) && painted(v, "ws-setup-notice") && painted(v, "ws-setup-signin"))
            .then(|| v.workman_notice())
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
        (matches!(&v.workman, WorkmanStatus::Ready { url, .. } if *url == base) && !painted(v, "ws-setup-notice")).then_some(())
    })
    .await?;
    let fresh: Vec<String> = oauth.issued().into_iter().skip(issued).collect();
    if fresh.is_empty() {
        bail!("Sign in issued no token");
    }
    let (blocked, housekeeping) = crate::selftest_work::inbox_keys();
    until(view, cx, "the Inbox's Workman cards", |v| (painted(v, &blocked) && housekeeping.iter().any(|k| painted(v, k))).then_some(())).await?;
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
    lines.push(format!("the notice's Sign in signed in again ({} new tokens), the Inbox paints its Workman cards read with them", fresh.len()));

    let other = FakeWorkman::start(vec![("/health".into(), Answer::text("Healthy"))]);
    let other_oauth = other.enable_oauth();
    handle.update(cx, |s, _, cx| s.set_url_text(other.base(), cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    handle.update(cx, |s, _, cx| s.apply_url(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    until(view, cx, "Ready on the other server", |v| {
        matches!(&v.workman, WorkmanStatus::Ready { url, .. } if url == other.base()).then_some(())
    })
    .await?;
    if saved_url()? != other.base() {
        bail!("the new URL was not saved after the sign-in there");
    }
    let named = supraman_core::workman_server::load(&crate::selftest::workman_server_json()).map_err(|e| anyhow!(e))?.mcp_name;
    if named != supraman_core::workman_server::MCP_NAME {
        bail!("Apply wrote mcpName {named:?}, not workman");
    }
    let mcp = format!("{}/mcp", other.base());
    settled(&handle, cx, "the warning that claude's workman entry is not the new server", |s| {
        s.server_check().filter(|c| c.warnings.iter().any(|w| w.contains(&mcp))).map(|_| ())
    })
    .await?;
    if other_oauth.issued().is_empty() {
        bail!("the other server issued nothing");
    }
    lines.push(format!("URL → {}: signed in there, saved", other.base()));

    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let closed = format!("http://127.0.0.1:{port}");
    handle.update(cx, |s, _, cx| s.set_url_text(&closed, cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    handle.update(cx, |s, _, cx| s.apply_url(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let error = until(view, cx, "the failed switch's error", |v| v.signing_in.is_none().then(|| v.sign_in_error.clone()).flatten()).await?;
    if !error.contains(&closed) || saved_url()? != other.base() {
        bail!("a failed switch to {closed}: {error:?}, URL now {}", saved_url()?);
    }
    shot_window("m5-43-settings-switch-failed", Some("Supraman Settings"), cx).await;
    lines.push(format!("URL → {closed} (closed): \"{error}\", the URL stayed"));
    view.update(cx, |v, cx| v.close_settings(cx));

    crate::selftest::set_workman(&closed)?;
    view.update(cx, |v, _| v.recheck_workman());
    let down = until(view, cx, "the unreachable notice", |v| {
        (v.workman.unreachable() && painted(v, "ws-setup-retry")).then(|| v.workman_notice()).flatten()
    })
    .await?;
    if !down.contains(&closed) {
        bail!("the unreachable notice does not name {closed}: {down:?}");
    }
    let warning = view.read_with(cx, |v, _| v.workman.start_warning("w"));
    if !warning.as_deref().is_some_and(|w| w.contains("unreachable")) {
        bail!("Start would not warn while unreachable: {warning:?}");
    }
    shot("m5-3-unreachable", cx).await;
    click(view, "ws-setup-dismiss", cx).await?;
    until(view, cx, "× hides the notice", |v| (!painted(v, "ws-setup-notice") && v.workman.unreachable()).then_some(())).await?;
    lines.push(format!("closed port: \"{down}\", Retry shown, × hid it"));

    auth.sign_out(other.base()).map_err(|e| anyhow!("signing out of the other fake: {e}"))?;
    crate::selftest::set_workman(&base)?;
    crate::signin::sign_in(&home, &base).map_err(|e| anyhow!("signing in to the gate's fake again: {e}"))?;
    view.update(cx, |v, cx| {
        v.recheck_workman();
        crate::work_view::WorkHub::reconfigure(Some(home.clone()), cx);
    });
    until(view, cx, "the gate's fake Workman Ready again", |v| matches!(v.workman, WorkmanStatus::Ready { .. }).then_some(())).await?;
    Ok(lines.join("; "))
}

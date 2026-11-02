use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail};
use gpui::{App, AsyncWindowContext, Entity};
use supraman_core::update::{Manifest, SigningKey, sign};
use supraman_workstate::fake::{Answer, FakeWorkState};
use supraman_workstate::updates::{self, Outcome, Source};

use crate::selftest::{click, sleep, until};
use crate::selftest_manage::shot_window;
use crate::selftest_schedules::themed_shots;
use crate::settings_window::page;
use crate::view::{FleetView, Link};

const WITHIN: Duration = Duration::from_secs(15);
const VERSION: &str = "99.0.0";
const LATEST: &str = "/repos/getsupraman/supraman/releases/latest";

static OPENED: Mutex<Vec<Link>> = Mutex::new(Vec::new());

fn recording_open(link: &Link, _: &mut App) {
    OPENED.lock().unwrap_or_else(|e| e.into_inner()).push(link.clone());
}

fn opened() -> Vec<Link> {
    OPENED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Release v99.0.0 on a fake releases endpoint, its manifest signed by a key made here.
fn serve(key: &SigningKey) -> FakeWorkState {
    let fake = FakeWorkState::start(Vec::new());
    let base = fake.base().to_string();
    let manifest = Manifest {
        version: VERSION.into(),
        commit: "c52996b".into(),
        zip: format!("Supraman-{VERSION}.zip"),
        sha256: "ab".repeat(32),
        size: 9,
        published: "2026-11-02T10:00:00Z".into(),
        notes_url: Some(format!("{base}/notes/{VERSION}")),
    };
    let bytes = manifest.to_bytes();
    let release = serde_json::json!({
        "tag_name": format!("v{VERSION}"),
        "html_url": format!("{base}/releases/v{VERSION}"),
        "body": "selftest notes",
        "draft": false,
        "prerelease": false,
        "assets": [
            {"name": "supraman-update.json", "browser_download_url": format!("{base}/download/m")},
            {"name": "supraman-update.json.sig", "browser_download_url": format!("{base}/download/s")},
        ],
    });
    fake.set(LATEST, Answer::json(release.to_string()).api(None).header("ETag", "\"selftest\""));
    fake.set("/download/m", Answer::text(String::from_utf8(bytes.clone()).unwrap_or_default()).api(None));
    fake.set("/download/s", Answer::text(String::from_utf8(sign(&bytes, key)).unwrap_or_default()).api(None));
    fake
}

fn painted(v: &FleetView) -> bool {
    v.hits.borrow().contains_key("update-notice")
}

/// public/8: a newer signed release on a fake endpoint shows the banner; What's new and Update open its pages; × hides
/// it and a later check keeps it hidden; Settings → About shows the last check; checking off sends nothing.
pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    OPENED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let key = SigningKey::from_bytes(&[8; 32]);
    let fake = serve(&key);
    let (real_open, real_source) = view.update(cx, |v, _| {
        let real_open = std::mem::replace(&mut v.open_link_with, recording_open);
        let mut source = v.updates.source.lock().unwrap_or_else(|e| e.into_inner());
        let real = source.clone();
        *source = Source { api: fake.base().to_string(), keys: Ok(vec![key.verifying_key()]), ..real.clone() };
        (real_open, real)
    });
    let result = steps(view, home, &fake, cx).await;
    view.update(cx, |v, cx| {
        v.open_link_with = real_open;
        *v.updates.source.lock().unwrap_or_else(|e| e.into_inner()) = real_source;
        v.close_settings(cx);
    });
    result
}

async fn checked(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let before = view.read_with(cx, |v, _| v.updates.checks);
    view.update(cx, |v, _| v.check_updates_now());
    until(view, cx, WITHIN, "the update check answered", |v| (v.updates.checks > before && !v.updates.checking).then_some(())).await
}

async fn steps(view: &Entity<FleetView>, home: &Path, fake: &FakeWorkState, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    checked(view, cx).await?;
    until(view, cx, WITHIN, "the update banner painted", |v| painted(v).then_some(())).await?;
    let offered = view.read_with(cx, |v, _| v.updates.offered().map(|a| a.version().to_string()));
    if offered.as_deref() != Some(VERSION) {
        bail!("the banner offers {offered:?}, not {VERSION}; last check {:?}", view.read_with(cx, |v, _| v.updates.last.clone()));
    }
    let sent = fake.seen().len();
    if sent != 3 {
        bail!("the check sent {sent} requests, not 3 (latest, manifest, signature): {:?}", fake.seen());
    }
    themed_shots(home, "public-8-update-banner", cx).await;

    click(view, "update-notes", cx).await?;
    click(view, "update-install", cx).await?;
    let base = fake.base();
    let want = vec![Link::Url(format!("{base}/notes/{VERSION}")), Link::Url(format!("{base}/releases/v{VERSION}"))];
    for _ in 0..50 {
        if opened().len() >= 2 {
            break;
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
    if opened() != want {
        bail!("What's new and Update opened {:?}, not {want:?}", opened());
    }

    view.update(cx, |v, cx| v.open_settings_at(Some(page::ABOUT), cx));
    let mut handle = None;
    for _ in 0..100 {
        handle = view.read_with(cx, |v, _| v.settings_window).filter(|h| h.read_with(cx, |_, _| ()).is_ok());
        if handle.is_some() {
            break;
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let handle = handle.ok_or_else(|| anyhow!("Settings did not open within 5 s"))?;
    let line = handle.read_with(cx, |s, cx| s.updates_line(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    if line != format!("Supraman {VERSION} is available") {
        bail!("Settings → About says {line:?}");
    }
    shot_window("public-8-settings-updates", Some("Supraman Settings"), cx).await;

    click(view, "update-dismiss", cx).await?;
    until(view, cx, WITHIN, "the update banner gone after ×", |v| (!painted(v)).then_some(())).await?;
    if updates::dismissed(home).map(|v| v.to_string()).as_deref() != Some(VERSION) {
        bail!("the home keeps dismissed {:?}, not {VERSION}", updates::dismissed(home));
    }
    fake.set(LATEST, Answer::status(304, "").api(None));
    checked(view, cx).await?;
    sleep(cx, Duration::from_millis(500)).await;
    let (still, outcome) = view.read_with(cx, |v, _| (painted(v), v.updates.last.as_ref().map(|c| c.outcome.clone())));
    if still || !matches!(outcome, Some(Outcome::Available(_))) {
        bail!("after × and another check: banner painted {still}, last outcome {outcome:?}");
    }
    let unchanged = fake.seen().last().and_then(|s| s.header("if-none-match").map(str::to_string));
    if unchanged.as_deref() != Some("\"selftest\"") {
        bail!("the second check sent If-None-Match {unchanged:?}");
    }

    handle.update(cx, |s, _, cx| s.set_update_check(false, cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let before = fake.seen().len();
    checked(view, cx).await?;
    let line = handle.read_with(cx, |s, cx| s.updates_line(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let after = fake.seen().len();
    handle.update(cx, |s, _, cx| s.set_update_check(true, cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    checked(view, cx).await?;
    if after != before || line != "Checking is off." {
        bail!("with checking off a check sent {} request(s) and About says {line:?}", after - before);
    }
    Ok(format!(
        "v{VERSION} on a fake endpoint: banner painted, What's new and Update opened its notes and release page, About said \
         'Supraman {VERSION} is available'; × kept it hidden through a 304 check; checking off sent no request"
    ))
}

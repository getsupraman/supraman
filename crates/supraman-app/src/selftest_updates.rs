//! `--selftest` step 7+u: the update banner and Settings → About against a fake releases endpoint, and `sm update apply` swapping a temp install and starting its stub: real processes and windows.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, bail};
use gpui::{App, AsyncWindowContext, Entity};
use supraman_core::update::{Manifest, SigningKey, sign};
use supraman_workman::fake::{Answer, FakeWorkman};
use supraman_workman::updates::{self, Source};

use crate::selftest::{click, sleep, until, until_with};
use crate::selftest_manage::shot_window;
use crate::selftest_schedules::themed_shots;
use crate::settings_window::page;
use crate::updates::{Stage, Target};
use crate::view::FleetView;
use supraman_ui::open::Link;

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
fn serve(key: &SigningKey) -> FakeWorkman {
    let fake = FakeWorkman::start(Vec::new());
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

/// A newer signed release on a fake endpoint shows the banner; What's new and Update open its pages; × hides
/// it and the home keeps it dismissed; Settings → About shows the last check.
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

const NEXT: &str = "99.1.0";

fn stored_zip(entries: &[(&str, &[u8])]) -> anyhow::Result<Vec<u8>> {
    use std::io::Write as _;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        w.start_file(*name, options)?;
        w.write_all(bytes)?;
    }
    Ok(w.finish()?.into_inner())
}

/// Release v99.1.0 of `zip` (served slowly) with a manifest of `sha256` signed by `key`.
fn serve_next(fake: &FakeWorkman, key: &SigningKey, zip: &[u8], sha256: &str) {
    let base = fake.base().to_string();
    let manifest = Manifest {
        version: NEXT.into(),
        commit: "c52996b".into(),
        zip: format!("Supraman-{NEXT}.zip"),
        sha256: sha256.into(),
        size: zip.len() as u64,
        published: "2026-11-02T10:00:00Z".into(),
        notes_url: None,
    };
    let bytes = manifest.to_bytes();
    let release = serde_json::json!({
        "tag_name": format!("v{NEXT}"),
        "html_url": format!("{base}/releases/v{NEXT}"),
        "draft": false,
        "prerelease": false,
        "assets": [
            {"name": "supraman-update.json", "browser_download_url": format!("{base}/download/m")},
            {"name": "supraman-update.json.sig", "browser_download_url": format!("{base}/download/s")},
            {"name": format!("Supraman-{NEXT}.zip"), "browser_download_url": format!("{base}/download/zip")},
        ],
    });
    fake.set(LATEST, Answer::json(release.to_string()).api(None));
    fake.set("/download/m", Answer::text(String::from_utf8(bytes.clone()).unwrap_or_default()).api(None));
    fake.set("/download/s", Answer::text(String::from_utf8(sign(&bytes, key)).unwrap_or_default()).api(None));
    fake.set("/download/zip", Answer::bytes(zip.to_vec()).paced(256 * 1024, Duration::from_millis(40)));
}

fn stage_painted(v: &FleetView) -> bool {
    v.hits.borrow().contains_key("update-stage")
}

/// Update on a fake endpoint over a temp install, never this build's folder; the release's supraman.exe is a stub sm.exe.
pub async fn install(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let sm_exe = supraman_core::install::sm_exe()?;
    let sm = std::fs::read(&sm_exe).map_err(|e| anyhow!("{}: {e}", sm_exe.display()))?;
    let root = home.join("update-test");
    let _ = std::fs::remove_dir_all(&root);
    let install = root.join("install");
    let marker = root.join("marker");
    for (path, bytes) in [
        (install.join("VERSION"), &b"version 0.1.0\n"[..]),
        (install.join("supraman.exe"), b"the old app"),
        (install.join("plugins/supraman/bin/sm.exe"), b"the old hooks copy"),
        (install.join("gone.txt"), b"only the old release ships me"),
        (install.join("FILES"), b"FILES\nVERSION\ngone.txt\nplugins/supraman/bin/sm.exe\nsupraman.exe\n"),
        (marker.join("hooks/hooks.json"), b"{}"),
    ] {
        std::fs::create_dir_all(path.parent().unwrap_or(&root))?;
        std::fs::write(&path, bytes)?;
    }
    let mut pad = vec![0u8; 640 * 1024];
    let mut seed = 0x2545_f491_u32;
    for b in &mut pad {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        *b = (seed >> 16) as u8;
    }
    let version = format!("version {NEXT}\n");
    let zip = stored_zip(&[
        ("Supraman/VERSION", version.as_bytes()),
        ("Supraman/sm.exe", &sm),
        ("Supraman/supraman.exe", &sm),
        ("Supraman/plugins/supraman/bin/sm.exe", b"the new hooks copy"),
        ("Supraman/pad.bin", &pad),
    ])?;
    let scratch = root.join("zip");
    std::fs::write(&scratch, &zip)?;
    let (sha256, _) = supraman_core::update::sha256_file(&scratch)?;

    let key = SigningKey::from_bytes(&[9; 32]);
    let fake = FakeWorkman::start(Vec::new());
    serve_next(&fake, &key, &zip, &sha256);
    let restart_args = vec!["plugin".to_string(), "install".to_string(), marker.display().to_string()];
    let (real_source, real_target) = view.update(cx, |v, _| {
        let mut source = v.updates.source.lock().unwrap_or_else(|e| e.into_inner());
        let real = source.clone();
        *source = Source { api: fake.base().to_string(), keys: Ok(vec![key.verifying_key()]), ..real.clone() };
        drop(source);
        let target = Target { install: install.clone(), wait_pid: None, quit: false, restart_args };
        (real, std::mem::replace(&mut v.updates.target, target))
    });
    let result = install_steps(view, home, &install, &marker, &sm, cx).await;
    view.update(cx, |v, cx| {
        if v.updates.offered().is_some() {
            v.dismiss_update(cx);
        }
        *v.updates.source.lock().unwrap_or_else(|e| e.into_inner()) = real_source;
        v.updates.target = real_target;
        v.updates.stage = Stage::Idle;
    });
    result
}

async fn install_steps(view: &Entity<FleetView>, home: &Path, install: &Path, marker: &Path, sm: &[u8], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    checked(view, cx).await?;
    until(view, cx, WITHIN, "the 99.1.0 banner painted", |v| (painted(v) && v.updates.offered().is_some_and(|a| a.version() == NEXT)).then_some(())).await?;
    click(view, "update-install", cx).await?;
    let line = until(view, cx, WITHIN, "the download's progress painted", |v| match v.updates.stage {
        Stage::Downloading { done, total } if done > 0 && done < total && stage_painted(v) => v.updates.stage_line(),
        _ => None,
    })
    .await?;
    if !line.starts_with("Downloading ") || !line.ends_with(" MB") {
        bail!("the banner says {line:?} while downloading");
    }
    shot_window("public-9-update-progress", None, cx).await;
    until(view, cx, WITHIN, "the hand-over to sm update apply", |v| match &v.updates.stage {
        Stage::Applying => Some(Ok(())),
        Stage::Failed(why) => Some(Err(why.clone())),
        _ => None,
    })
    .await?
    .map_err(|why| anyhow!("the update failed: {why}"))?;
    let started = marker.join("bin").join("sm.exe");
    let log = updates::version_dir(home, NEXT).join(updates::APPLY_LOG);
    until_with(cx, Duration::from_secs(30), "the stub supraman.exe started", |_| started.exists().then_some(())).await.map_err(|e| {
        anyhow!("{e}; apply.log: {}", std::fs::read_to_string(&log).unwrap_or_default())
    })?;
    let applied = std::fs::read_to_string(&log)?;
    let version = std::fs::read_to_string(install.join("VERSION"))?;
    if !version.starts_with(&format!("version {NEXT}")) || std::fs::read(install.join("supraman.exe"))? != sm || install.join("gone.txt").exists() {
        bail!("the install after the apply: VERSION {version:?}, gone.txt there {}; apply.log: {applied}", install.join("gone.txt").exists());
    }
    if !applied.contains(" done\n") || !applied.contains(" started ") {
        bail!("apply.log: {applied}");
    }
    Ok(format!(
        "v{NEXT} over a temp install: the zip showed '{line}', sm update apply swapped it in (VERSION {NEXT}, gone.txt removed) and started the stub supraman.exe"
    ))
}

async fn checked(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let before = view.read_with(cx, |v, _| v.updates.checks);
    view.update(cx, |v, _| v.check_updates_now());
    until(view, cx, WITHIN, "the update check answered", |v| (v.updates.checks > before && !v.updates.checking).then_some(())).await
}

async fn steps(view: &Entity<FleetView>, home: &Path, fake: &FakeWorkman, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    checked(view, cx).await?;
    until(view, cx, WITHIN, "the update banner painted", |v| painted(v).then_some(())).await?;
    let offered = view.read_with(cx, |v, _| v.updates.offered().map(|a| a.version().to_string()));
    if offered.as_deref() != Some(VERSION) {
        bail!("the banner offers {offered:?}, not {VERSION}; last check {:?}", view.read_with(cx, |v, _| v.updates.last.clone()));
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
    for _ in 0..100 {
        if handle.read_with(cx, |s, _| s.loaded()).map_err(|e| anyhow!("settings window: {e}"))? {
            break;
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let line = handle.read_with(cx, |s, cx| s.updates_line(cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    if line != format!("Supraman {VERSION} is available") {
        bail!("Settings → About says {line:?}");
    }
    shot_window("public-8-settings-updates", Some("Supraman Settings"), cx).await;

    click(view, "update-dismiss", cx).await?;
    until(view, cx, WITHIN, "the update banner gone after ×", |v| (!painted(v)).then_some(())).await?;
    let kept = |_: &FleetView| (updates::dismissed(home).map(|v| v.to_string()).as_deref() == Some(VERSION)).then_some(());
    if until(view, cx, WITHIN, "the home keeps the dismissed version", kept).await.is_err() {
        bail!("the home keeps dismissed {:?}, not {VERSION}", updates::dismissed(home));
    }
    Ok(format!(
        "v{VERSION} on a fake endpoint: banner painted, What's new and Update opened its notes and release page, About said \
         'Supraman {VERSION} is available'; × hid it and the home keeps it dismissed"
    ))
}

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{until, until_with},
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);
const ID: &str = "57a27000-0000-0000-0000-000000000062";
const BEFORE: &str = "selftest startup before";
const AFTER: &str = "selftest startup after";

fn title(v: &FleetView) -> Option<String> {
    v.closed.iter().find(|s| s.session_id == ID).map(|s| s.name.clone())
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
}

pub async fn run(view: &Entity<FleetView>, home: &Path, main_repo: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let path = crate::selftest_closed::claude_dir(home)
        .join("projects")
        .join(supraman_claude::transcript::slug(main_repo))
        .join(format!("{ID}.jsonl"));
    std::fs::write(&path, crate::selftest_closed::transcript(ID, main_repo, BEFORE))?;
    let result = check(view, home, &path, cx).await;
    let hold = view.read_with(cx, |v, _| v.transcripts_hold.clone());
    hold.armed.store(false, Ordering::SeqCst);
    hold.holding.store(false, Ordering::SeqCst);
    let _ = std::fs::remove_file(&path);
    view.update(cx, |v, _| v.transcripts_nudge.wake());
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, path: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let cache = home.join("cache").join("transcripts.json");
    view.update(cx, |v, _| v.transcripts_nudge.wake());
    until(view, cx, WITHIN, "the new transcript listed and saved in the cache", |v| {
        (title(v).as_deref() == Some(BEFORE) && std::fs::read_to_string(&cache).is_ok_and(|c| c.contains(ID))).then_some(())
    })
    .await?;

    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    writeln!(file, "{}", serde_json::json!({"type":"custom-title","customTitle":AFTER,"sessionId":ID}))?;
    drop(file);
    let hold = view.read_with(cx, |v, _| v.transcripts_hold.clone());
    hold.armed.store(true, Ordering::SeqCst);
    let scans = view.update(cx, |v, cx| {
        v.restart_transcripts(true, cx);
        v.transcript_scans
    });
    let warm = Instant::now();
    let line = until_with(cx, WITHIN, "a warm start: the cached row shown, every source loaded, while the scan is held", |cx| {
        view.read_with(cx, |v, _| {
            if painted(v, "loading-sessions") {
                return Some(Err("the loading line painted on a start with a saved cache".to_string()));
            }
            (hold.holding.load(Ordering::SeqCst) && v.loaded && v.transcripts_read).then(|| match title(v) {
                Some(t) if t == BEFORE => Ok(()),
                t => Err(format!("the cached row reads {t:?}, not {BEFORE:?}")),
            })
        })
    })
    .await?;
    line.map_err(|e| anyhow::anyhow!(e))?;
    let shown = warm.elapsed();
    if view.read_with(cx, |v, _| (v.loaded_at_scan, v.transcript_scans)) != (scans, scans) {
        bail!("a warm start waited for the transcript scan");
    }
    hold.holding.store(false, Ordering::SeqCst);
    until(view, cx, WITHIN, "the changed transcript renamed by the background scan", |v| {
        (v.transcript_scans > scans && title(v).as_deref() == Some(AFTER)).then_some(())
    })
    .await?;
    let renamed = warm.elapsed();

    std::fs::remove_file(&cache)?;
    hold.armed.store(true, Ordering::SeqCst);
    let scans = view.update(cx, |v, cx| {
        v.restart_transcripts(false, cx);
        v.transcript_scans
    });
    let cold = Instant::now();
    until_with(cx, WITHIN, "a cold start: the loading line naming the transcripts while the scan is held", |cx| {
        view.read_with(cx, |v, cx| {
            (hold.holding.load(Ordering::SeqCst) && painted(v, "loading-sessions") && v.loading(cx).iter().any(|p| p.starts_with("transcripts")))
                .then_some(())
        })
    })
    .await?;
    hold.holding.store(false, Ordering::SeqCst);
    until(view, cx, WITHIN, "the loading line gone once the scan is applied", |v| {
        (v.loaded && !painted(v, "loading-sessions") && title(v).as_deref() == Some(AFTER)).then_some(())
    })
    .await?;
    if view.read_with(cx, |v, _| v.loaded_at_scan) != scans + 1 {
        bail!("a cold start loaded before its transcript scan");
    }
    Ok(format!(
        "warm start: cached row shown with no loading line {} ms after the restart, renamed by the background scan at {} ms; cold start: loading line until the scan, gone after {} ms",
        shown.as_millis(),
        renamed.as_millis(),
        cold.elapsed().as_millis()
    ))
}

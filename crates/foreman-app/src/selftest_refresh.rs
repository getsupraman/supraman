//! `--selftest` step 7j (m5/28): a closed interactive session offers resume at once, and Refresh (the action bar's
//! button and F5) rescans now. [`ENDING`] is injected as a live interactive session in the gate's main repo, its pid
//! a windowless `ping` of the selftest's own. Right after a poll the ping is killed and the row taken out of the
//! `claude agents` snapshot with its transcript written to the fake claude dir (as claude leaves it when its terminal
//! closes): the time until its closed row paints resume is measured, and must stay under [`CLOSE_TO_RESUME`]. Then,
//! each right after a poll and a scan finished, a transcript appears for [`FRESH`]: the Refresh button, and F5 with a
//! Work tab active for a second one ([`FRESH_KEY`]), must poll and scan again within [`REFRESHED`], well before
//! [`REFRESH_EVERY`] / [`TRANSCRIPTS_EVERY`] would. Last, F5 with a shell tab active reaches its program (a pwsh key
//! reader prints `F5 0`) and refreshes nothing.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use gpui::{AsyncWindowContext, Entity, Modifiers};

use crate::{
    selftest::{click, hex6, until},
    selftest_closed::{claude_dir, transcript},
    view::{FleetView, REFRESH_EVERY, TRANSCRIPTS_EVERY},
};

pub const ENDING: &str = "e2d1c105-0000-4000-8000-00000000e2d1";
pub const FRESH: &str = "f2e5a7b0-0000-4000-8000-00000000f2e5";
pub const FRESH_KEY: &str = "f5f5ab0c-0000-4000-8000-00000000f5f5";
const ENDING_TITLE: &str = "selftest ending session";
/// From the process gone to resume painted: the pid watch (250 ms), one `claude agents`, one scan (0.9–1.7 s
/// measured); a poll at the end of the wait cannot pass, it takes [`REFRESH_EVERY`] plus `claude agents` (≥ 2.5 s).
const CLOSE_TO_RESUME: Duration = Duration::from_millis(2300);
const REFRESHED: Duration = Duration::from_millis(1500);
const WITHIN: Duration = Duration::from_secs(45);

/// The tree row of `id`: (closed, resume offered and painted).
fn row(v: &FleetView, id: &str) -> Option<(bool, bool)> {
    let tree = v.tree();
    let row = tree.workspaces.iter().flat_map(|w| w.repos.iter().flat_map(|r| &r.sessions)).find(|r| r.session.session_id == id)?;
    let painted = v.hits.borrow().contains_key(&format!("resume-{}", &id[..8]));
    Some((row.closed, v.resumable(row) && painted))
}

fn write_transcript(home: &Path, main: &Path, id: &str, title: &str) -> anyhow::Result<()> {
    let dir = claude_dir(home).join("projects").join(foreman_claude::transcript::slug(main));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{id}.jsonl")), transcript(id, main, title))?;
    Ok(())
}

pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let shown = view.update(cx, |v, cx| {
        cx.notify();
        std::mem::replace(&mut v.show_stopped, true)
    });
    // the ended session's process: a windowless ping of this run's own, killed by its handle
    let mut process = foreman_core::process::command("ping")
        .args(["-n", "120", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()?;
    let earlier: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| earlier.into_iter().for_each(|id| v.close_tab(id, cx)));
    let result = check(view, home, &mut process, cx).await;
    process.kill().ok();
    process.wait().ok();
    let mine: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| {
        mine.into_iter().for_each(|id| v.close_tab(id, cx));
        v.injected.lock().unwrap_or_else(|e| e.into_inner()).clear();
        v.show_stopped = shown;
        cx.notify();
    });
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, process: &mut std::process::Child, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let main = crate::selftest::main_repo_of_gate()?;
    let live = serde_json::json!([{
        "sessionId": ENDING,
        "name": ENDING_TITLE,
        "cwd": main.display().to_string(),
        "kind": "interactive",
        "startedAt": foreman_claude::now_ms() - 60_000,
        "status": "idle",
        "pid": process.id(),
    }]);
    let rows = foreman_claude::parse(&live.to_string())?;
    view.read_with(cx, |v, _| v.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(rows));
    until(view, cx, WITHIN, "the injected interactive session running in the tree", |v| row(v, ENDING).filter(|r| !r.0)).await?;

    // right after a poll, its terminal closes: the process ends, claude drops it from `claude agents`, its transcript stays
    let polls = view.read_with(cx, |v, _| v.refreshes);
    until(view, cx, WITHIN, "a poll", |v| (v.refreshes > polls).then_some(())).await?;
    write_transcript(home, &main, ENDING, ENDING_TITLE)?;
    process.kill()?;
    process.wait()?;
    view.read_with(cx, |v, _| v.injected.lock().unwrap_or_else(|e| e.into_inner()).clear());
    let gone = Instant::now();
    until(view, cx, WITHIN, "the ended session out of the snapshot", |v| (!v.sessions.iter().any(|s| s.session_id == ENDING)).then_some(())).await?;
    let left = gone.elapsed();
    until(view, cx, WITHIN, "the ended session as a closed row", |v| row(v, ENDING).filter(|r| r.0)).await?;
    let closed = gone.elapsed();
    until(view, cx, WITHIN, "resume on the closed row", |v| row(v, ENDING).filter(|r| r.1)).await?;
    let resume = gone.elapsed();
    println!(
        "step 7j: close → resume {} ms (left the snapshot {} ms, closed row {} ms)",
        resume.as_millis(),
        left.as_millis(),
        closed.as_millis()
    );
    if resume > CLOSE_TO_RESUME {
        bail!("resume showed {} ms after the session ended, over {} ms", resume.as_millis(), CLOSE_TO_RESUME.as_millis());
    }

    let button = refreshed(view, home, &main, FRESH, cx, |view, cx| Box::pin(click(view, "refresh", cx))).await?;
    let work = view.update(cx, |v, cx| v.open_work(crate::selftest_work::WORKSPACE, cx));
    click(view, &format!("tab-{work}"), cx).await?;
    let key = refreshed(view, home, &main, FRESH_KEY, cx, |_, cx| Box::pin(async move { crate::selftest_tabs::press(cx, "f5", Modifiers::default()) })).await?;
    let in_terminal = f5_in_a_terminal(view, cx).await?;
    Ok(format!(
        "{ENDING} ended (out of the snapshot after {} ms): closed row at {} ms, resume at {} ms; \
         Refresh button polled and scanned in {} ms, F5 on a Work tab in {} ms (every {} s / {} s otherwise); {in_terminal}",
        left.as_millis(),
        closed.as_millis(),
        resume.as_millis(),
        button.as_millis(),
        key.as_millis(),
        REFRESH_EVERY.as_secs(),
        TRANSCRIPTS_EVERY.as_secs()
    ))
}

type Trigger = for<'a> fn(&'a Entity<FleetView>, &'a mut AsyncWindowContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>>;

/// Right after a poll and a scan, `id`'s transcript appears and `trigger` runs: both must come again, and the
/// new closed row show, within [`REFRESHED`].
async fn refreshed(view: &Entity<FleetView>, home: &Path, main: &Path, id: &str, cx: &mut AsyncWindowContext, trigger: Trigger) -> anyhow::Result<Duration> {
    let scans = view.read_with(cx, |v, _| v.transcript_scans);
    until(view, cx, WITHIN, "a scan", |v| (v.transcript_scans > scans).then_some(())).await?;
    let polls = view.read_with(cx, |v, _| v.refreshes);
    until(view, cx, WITHIN, "a poll", |v| (v.refreshes > polls).then_some(())).await?;
    let (polls, scans) = view.read_with(cx, |v, _| (v.refreshes, v.transcript_scans));
    write_transcript(home, main, id, &format!("selftest refreshed {}", &id[..8]))?;
    let asked = Instant::now();
    trigger(view, cx).await?;
    until(view, cx, REFRESHED, &format!("a poll, a scan and {id} after Refresh"), |v| {
        (v.refreshes > polls && v.transcript_scans > scans && row(v, id).is_some_and(|r| r.0)).then_some(())
    })
    .await?;
    Ok(asked.elapsed())
}

/// F5 with a shell tab active goes to its program, and [`FleetView::refresh_now`] is not called.
async fn f5_in_a_terminal(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let script = std::env::temp_dir().join(format!("fm-selftest-f5-{}.ps1", hex6()));
    std::fs::write(&script, crate::selftest_tabs::READER).context("the key reader script")?;
    let result = f5_reaches(view, &script, cx).await;
    std::fs::remove_file(&script).ok();
    result
}

async fn f5_reaches(view: &Entity<FleetView>, script: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let args = ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"].map(String::from).into_iter();
    let args: Vec<String> = args.chain([script.display().to_string()]).collect();
    let tab = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            let saved = std::mem::replace(&mut v.shell_args, args);
            let tab = v.open_shell_in(std::env::temp_dir(), "keys F5".into(), window, cx);
            v.shell_args = saved;
            tab
        })
    })?;
    let terminal = crate::selftest_tabs::reader(view, tab, cx).await?;
    click(view, &format!("tab-{tab}"), cx).await?;
    let asks = view.read_with(cx, |v, _| v.refresh_asks);
    crate::selftest_tabs::press(cx, "f5", Modifiers::default())?;
    let got = crate::selftest_tabs::keys(&terminal, 1, cx).await.context("the key reader")?;
    if got != ["fm-key F5 0"] {
        bail!("F5 in the shell tab: the reader got {got:?}, expected only F5");
    }
    let after = view.read_with(cx, |v, _| v.refresh_asks);
    if after != asks {
        bail!("F5 in the shell tab also refreshed ({asks} → {after} refresh_now calls)");
    }
    Ok(format!("F5 in a shell tab reached its program ({got:?}) and refreshed nothing"))
}

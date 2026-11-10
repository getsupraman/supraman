//! Keystroke-to-paint with René's load (460 live sessions, 2800 closed ones): characters typed at a typist's pace
//! into New task…'s prompt and into a shell tab, each timed from when it was due to the first frame painted with it,
//! and every UI-thread stall over 100 ms (supraman_ui::stall) with what ran.

use std::time::{Duration, Instant};

use anyhow::{bail, Context as _};
use gpui::{AsyncWindowContext, Entity, Keystroke, Modifiers};
use supraman_claude::Session;
use supraman_term::Terminal;
use supraman_ui::{percentile, stall};

use crate::selftest::{WORKSPACE, frames, sleep, until, until_with, wait_for_tab_terminal};
use crate::view::FleetView;

const LIVE: usize = 460;
const CLOSED: usize = 2800;
const WITHIN: Duration = Duration::from_secs(20);
/// A fast typist: 12 characters a second.
const PACE: Duration = Duration::from_millis(80);
const PROMPT_KEYS: usize = 60;
const LONG_KEYS: usize = 240;
const LINE: usize = 60;
const LONG_PACE: Duration = Duration::from_millis(50);
const PASTE_LINES: usize = 30;
const TERMINAL_KEYS: usize = 30;
pub(crate) const PROMPT_P95_MS: f64 = 100.0;
pub(crate) const FRAME_P95_MS: f64 = 50.0;
pub(crate) const TERMINAL_P95_MS: f64 = 100.0;
/// A key this late fails the step at once, inside the run's watchdog.
const GIVE_UP: Duration = Duration::from_secs(2);
const KEY_WITHIN: Duration = Duration::from_secs(5);
const SEEN_POLL: Duration = Duration::from_millis(1);

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    // the check's fresh build on every cached tree is the selftest's own load, not the app's
    let checking = crate::view::tree_check::set(false);
    let result = check(view, cx).await;
    crate::view::tree_check::set(checking);
    view.update(cx, |v, cx| {
        v.shell_args = saved;
        v.polls.injected.lock().unwrap_or_else(|e| e.into_inner()).clear();
        v.listing.injected_closed.clear();
        v.listing.closed.retain(|s| !s.session_id.starts_with(FIXTURE));
        v.invalidate_tree();
        let mine: Vec<u64> = v.tabs.iter().map(|t| t.id).filter(|id| !before.contains(id)).collect();
        mine.into_iter().for_each(|id| v.close_tab(id, cx));
        cx.notify();
    });
    result
}

const FIXTURE: &str = "f1c7";

fn fixture(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<(Vec<Session>, Vec<Session>)> {
    let folders: Vec<std::path::PathBuf> = view.read_with(cx, |v, _| v.workspaces.iter().flat_map(|w| w.folders.iter().map(|f| f.path.clone())).collect());
    if folders.is_empty() {
        bail!("the registry has no folders for the fixture sessions");
    }
    let now = supraman_claude::now_ms();
    let live: Vec<serde_json::Value> = (0..LIVE)
        .map(|i| {
            let (state, status) = [("running", "busy"), ("idle", "idle"), ("blocked", "waiting")][i % 3];
            serde_json::json!({
                "id": format!("{FIXTURE}{i:04x}"),
                "sessionId": format!("{FIXTURE}{i:04x}-0000-4000-8000-{i:012x}"),
                "name": format!("fixture-{i}"),
                "cwd": folders[i % folders.len()],
                "kind": if i % 2 == 0 { "background" } else { "interactive" },
                "startedAt": now - (i as i64) * 60_000,
                "state": state,
                "status": status,
                "pid": std::process::id(),
            })
        })
        .collect();
    let live = supraman_claude::parse(&serde_json::Value::Array(live).to_string())?;
    let mut closed = live.first().cloned().into_iter().cycle().take(CLOSED).collect::<Vec<_>>();
    for (i, s) in closed.iter_mut().enumerate() {
        s.session_id = format!("{FIXTURE}c{i:03x}-0000-4000-8000-{i:012x}");
        s.id = s.session_id[..8].to_string();
        s.name = format!("closed fixture {i}");
        s.cwd = folders[i % folders.len()].clone();
        s.kind = supraman_claude::Kind::Interactive;
        s.state = Some(supraman_claude::SessionState::Stopped);
        s.status = None;
        s.pid = None;
        s.started_at = Some(now - (i as i64) * 7 * 60_000);
    }
    Ok((live, closed))
}

#[derive(Debug)]
struct Latency {
    p50: f64,
    p95: f64,
    max: f64,
    keys: usize,
}

impl Latency {
    fn of(mut ms: Vec<f64>) -> Self {
        ms.sort_by(|a, b| a.total_cmp(b));
        Self { p50: percentile(&ms, 50.0), p95: percentile(&ms, 95.0), max: ms.last().copied().unwrap_or(f64::NAN), keys: ms.len() }
    }
}

impl std::fmt::Display for Latency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms over {} keys", self.p50, self.p95, self.max, self.keys)
    }
}

/// Each key is due `PACE` after the last; `seen` says when its character is in the view's state, the first frame
/// painted after that shows it.
async fn timed(
    view: &Entity<FleetView>,
    keys: &[char],
    pace: Duration,
    cx: &mut AsyncWindowContext,
    mut seen: impl FnMut(usize, &mut AsyncWindowContext) -> bool,
) -> anyhow::Result<(Latency, frames::Stretch)> {
    let counted = crate::view::tree_check::counts();
    let recording = frames::start(view, cx);
    let typed = typed_at(view, keys, pace, cx, &mut seen).await;
    let painted = view.read_with(cx, |v, _| v.probe.painted());
    let stretch = recording.stop(view, cx);
    let (builds, renders) = crate::view::tree_check::counts();
    println!("step 7+r: {} trees built over {} renders while typing {} keys", builds - counted.0, renders - counted.1, keys.len());
    let shown = typed?;
    let mut ms = Vec::new();
    for (i, (due, at)) in shown.iter().enumerate() {
        let frame = painted.iter().find(|p| *p >= at).with_context(|| format!("no frame painted after key {i}"))?;
        ms.push(frame.saturating_duration_since(*due).as_secs_f64() * 1000.0);
    }
    Ok((Latency::of(ms), stretch))
}

async fn typed_at(
    view: &Entity<FleetView>,
    keys: &[char],
    pace: Duration,
    cx: &mut AsyncWindowContext,
    seen: &mut impl FnMut(usize, &mut AsyncWindowContext) -> bool,
) -> anyhow::Result<Vec<(Instant, Instant)>> {
    let mut due = Instant::now() + pace;
    let mut shown = Vec::new();
    for (i, c) in keys.iter().enumerate() {
        let wait = due.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            sleep(cx, wait).await;
        }
        let keystroke = match c {
            '\n' => Keystroke { modifiers: Modifiers { shift: true, ..Default::default() }, key: "enter".into(), key_char: None },
            ' ' => Keystroke { modifiers: Modifiers::default(), key: "space".into(), key_char: Some(" ".into()) },
            _ => Keystroke { modifiers: Modifiers::default(), key: c.to_string(), key_char: Some(c.to_string()) },
        };
        cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx))?;
        let deadline = Instant::now() + KEY_WITHIN;
        while !seen(i, cx) {
            if Instant::now() > deadline {
                let frames = view.read_with(cx, |v, _| v.probe.painted().len());
                let state = cx.update(|window, cx| tabs_now(view, window, cx))?;
                bail!("key {i} ({c:?}) not shown within {} s; {state}; {frames} frames painted meanwhile; stalls: {}", KEY_WITHIN.as_secs(), listed(&stall::take()));
            }
            sleep(cx, SEEN_POLL).await;
        }
        let late = due.elapsed();
        if late > GIVE_UP {
            bail!("key {i} ({c:?}) shown {} ms after it was due, over {} ms: the keys queue behind the frames; stalls: {}", late.as_millis(), GIVE_UP.as_millis(), listed(&stall::take()));
        }
        shown.push((due, Instant::now()));
        due += pace;
    }
    sleep(cx, Duration::from_millis(200)).await;
    Ok(shown)
}

fn tabs_now(view: &Entity<FleetView>, window: &gpui::Window, cx: &gpui::App) -> String {
    let v = view.read(cx);
    let tabs: Vec<String> = v
        .tabs
        .iter()
        .map(|t| {
            let kind = match &t.kind {
                crate::view::TabKind::Attach { .. } => "attach".to_string(),
                crate::view::TabKind::Shell => "shell".to_string(),
                crate::view::TabKind::Work(_) => "work".to_string(),
                crate::view::TabKind::Git(_) => "git".to_string(),
                crate::view::TabKind::Schedule(_) => "schedule".to_string(),
                crate::view::TabKind::Details(_) => "details".to_string(),
                crate::view::TabKind::Saved(_) => "saved".to_string(),
                crate::view::TabKind::Start(p) => format!("start {:?} focused {}", p.input.read(cx).text(), p.input.read(cx).is_focused(window)),
            };
            format!("{} {kind} {:?}", t.id, t.title)
        })
        .collect();
    format!("active {:?}; tabs [{}]; root focused {}", v.active, tabs.join(", "), window.focused(cx).is_some())
}

fn listed(s: &[stall::Stall]) -> String {
    if s.is_empty() { "none".to_string() } else { s.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ") }
}

fn keys(n: usize) -> Vec<char> {
    "the quick brown fox jumps over the lazy dog ".chars().cycle().take(n).collect()
}

async fn check(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (live, closed) = fixture(view, cx)?;
    view.update(cx, |v, cx| {
        v.polls.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(live);
        v.listing.closed.extend(closed.iter().cloned());
        v.listing.injected_closed = closed;
        v.invalidate_tree();
        v.polls.poll_nudge.wake();
        cx.notify();
    });
    until(view, cx, WITHIN, "the fixture sessions in the list", |v| (v.sessions.iter().filter(|s| s.session_id.starts_with(FIXTURE)).count() == LIVE).then_some(())).await?;
    stall::take();

    let tab = cx.update(|window, cx| view.update(cx, |v, cx| v.open_start(WORKSPACE, None, false, window, cx)))?.context("New task opened no tab")?;
    let found = crate::selftest_start::start_tab(view, cx).await?;
    println!("step 7+r: New task opened tab {tab}, the focused prompt is tab {found}; {}", cx.update(|window, cx| tabs_now(view, window, cx))?);
    println!("step 7+r: fixture in the list; stalls while it settled: {}", listed(&stall::take()));
    let typed = keys(PROMPT_KEYS);
    let (prompt, prompt_frames) = timed(view, &typed, PACE, cx, |i, cx| {
        view.read_with(cx, |v, cx| v.start_page(tab).is_some_and(|p| p.input.read(cx).text().chars().count() > i))
    })
    .await?;
    let prompt_stalls = stall::take();
    println!("step 7+r: prompt keystroke-to-paint {prompt}; frames {}; stalls: {}", prompt_frames.stats, listed(&prompt_stalls));
    view.update(cx, |v, cx| v.close_tab(tab, cx));

    let (long, long_frames, pasted) = long_scheduled(view, cx).await?;
    let long_stalls = stall::take();
    println!("step 7+r: long scheduled prompt keystroke-to-paint {long}; paste of {PASTE_LINES} lines to paint {pasted:.1} ms; frames {}; stalls: {}", long_frames.stats, listed(&long_stalls));

    let shell = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let terminal = wait_for_tab_terminal(view, shell, WITHIN, cx).await?;
    until_with(cx, WITHIN, "the shell's prompt", |cx| terminal.read_with(cx, |t, _| t.get_content()).contains("PS ").then_some(())).await?;
    stall::take();
    let typed: Vec<char> = keys(TERMINAL_KEYS * 2).into_iter().filter(|c| *c != ' ').take(TERMINAL_KEYS).collect();
    let start = prompt_line(&terminal, cx);
    let (term, term_frames) = timed(view, &typed, PACE, cx, |i, cx| {
        let want: String = typed[..=i].iter().collect();
        prompt_line(&terminal, cx).strip_prefix(start.as_str()).is_some_and(|rest| rest.trim_start().starts_with(want.as_str()))
    })
    .await?;
    let term_stalls = stall::take();

    let report = format!(
        "with {LIVE} live and {CLOSED} closed fixture sessions: prompt keystroke-to-paint {prompt} (budget p95 {PROMPT_P95_MS} ms), frames {} (budget p95 {FRAME_P95_MS} ms); stalls over 100 ms: {}; \
         New scheduled task, {LONG_KEYS} keys over lines then a {PASTE_LINES}-line paste: {long}, paste to paint {pasted:.1} ms; stalls: {}; \
         terminal keystroke-to-paint {term} (budget p95 {TERMINAL_P95_MS} ms), frames {}; stalls: {}",
        prompt_frames.stats,
        listed(&prompt_stalls),
        listed(&long_stalls),
        term_frames.stats,
        listed(&term_stalls),
    );
    println!("step 7+r measured: {report}");
    let busy = prompt_frames.others_cpu.max(long_frames.others_cpu).max(term_frames.others_cpu);
    if busy >= frames::BUSY_CPU {
        return Ok(format!("{report}; machine busy ({:.0} % CPU by others): budgets not judged", busy * 100.0));
    }
    if prompt.p95 > PROMPT_P95_MS || long.p95 > PROMPT_P95_MS || pasted > PROMPT_P95_MS || term.p95 > TERMINAL_P95_MS {
        bail!("keystroke-to-paint over budget: {report}");
    }
    if prompt_frames.stats.p95 > FRAME_P95_MS || long_frames.stats.p95 > FRAME_P95_MS {
        bail!("frames over the p95 budget of {FRAME_P95_MS} ms while typing: {report}");
    }
    Ok(report)
}

/// René's case: New scheduled task…, a long prompt over several lines at 20 keys a second, a pasted block, more keys after it.
async fn long_scheduled(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<(Latency, frames::Stretch, f64)> {
    use gpui::EntityInputHandler as _;
    let tab = cx.update(|window, cx| view.update(cx, |v, cx| v.open_start(WORKSPACE, None, true, window, cx)))?.context("New scheduled task opened no tab")?;
    crate::selftest_start::start_tab(view, cx).await?;
    let count = |cx: &mut AsyncWindowContext| view.read_with(cx, |v, cx| v.start_page(tab).map_or(0, |p| p.input.read(cx).text().chars().count()));
    let mut typed: Vec<char> = Vec::new();
    for line in keys(LONG_KEYS).chunks(LINE) {
        typed.extend(line);
        typed.push('\n');
    }
    let (long, frames) = timed(view, &typed, LONG_PACE, cx, |i, cx| count(cx) > i).await?;
    let block: String = (0..PASTE_LINES).map(|i| format!("{i:02} every weekday at 07:30 replay the failed runs of the night and report them\n")).collect();
    let input = view.read_with(cx, |v, _| v.start_page(tab).map(|p| p.input.clone())).context("the scheduled prompt tab is gone")?;
    let recording = frames::start(view, cx);
    let before = count(cx);
    let at = Instant::now();
    cx.update(|window, cx| input.update(cx, |i, cx| i.replace_text_in_range(None, &block, window, cx)))?;
    until_with(cx, WITHIN, "the pasted block in the prompt", |cx| (count(cx) > before).then_some(())).await?;
    let seen = Instant::now();
    sleep(cx, Duration::from_millis(200)).await;
    let painted = view.read_with(cx, |v, _| v.probe.painted());
    recording.stop(view, cx);
    let pasted = painted.iter().find(|p| **p >= seen).map_or(f64::NAN, |p| p.saturating_duration_since(at).as_secs_f64() * 1000.0);
    let more = keys(20);
    let start = count(cx);
    let (after, frames_after) = timed(view, &more, LONG_PACE, cx, |i, cx| count(cx) > start + i).await?;
    view.update(cx, |v, cx| v.close_tab(tab, cx));
    let worst = if after.p95 > long.p95 { after } else { long };
    let frames = if frames_after.stats.p95 > frames.stats.p95 { frames_after } else { frames };
    Ok((worst, frames, pasted))
}

/// The cursor's line, trailing blanks dropped.
fn prompt_line(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> String {
    let content = terminal.read_with(cx, |t, _| t.get_content());
    content.lines().rev().find(|l| l.trim_start().starts_with("PS ")).unwrap_or_default().trim_end().to_string()
}

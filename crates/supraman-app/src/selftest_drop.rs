use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::bail;
use supraman_term::Terminal;
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{click, scrolled_into_view, sleep, until, wait_for_tab_terminal},
    selftest_clipboard::{put_text, sequence, text},
    selftest_tabs::drag_from,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(10);
const PROMPT: &str = "selftest-cwd>";
const SETTLE: Duration = Duration::from_millis(500);
const SENTINEL: &str = "not dropped";

/// The prompts printed so far and the last one, trimmed.
fn prompts(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> (usize, String) {
    let content = terminal.read_with(cx, |t, _| t.get_content());
    let lines: Vec<&str> = content.lines().filter(|l| l.contains(PROMPT)).collect();
    (lines.len(), lines.last().map(|l| l.trim_end().to_string()).unwrap_or_default())
}

async fn prompt_is(terminal: &Entity<Terminal>, what: &str, ends: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<(usize, String)> {
    let started = Instant::now();
    loop {
        let now = prompts(terminal, cx);
        if now.1.ends_with(ends) {
            return Ok(now);
        }
        if started.elapsed() > WITHIN {
            bail!("{what}: the prompt line reads {:?}, expected it to end with {ends:?}", now.1);
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

/// `claude` (tab `tab`, session `sid`, SendMessage name `name`) runs; `plain` is a pwsh tab in `cwd`.
pub(crate) async fn run(
    view: &Entity<FleetView>,
    [tab, plain]: [u64; 2],
    sid: &str,
    name: &str,
    cwd: &Path,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    click(view, &format!("tab-{plain}"), cx).await?;
    until(view, cx, WITHIN, "the plain tab active", |v| (v.active == Some(plain)).then_some(())).await?;
    let terminal = wait_for_tab_terminal(view, plain, WITHIN, cx).await?;
    let (before, _) = prompt_is(&terminal, "the plain shell", PROMPT, cx).await?;
    put_text(SENTINEL, cx)?;
    let clipboard = sequence();
    let pane = format!("terminal-{plain}");
    let target = scrolled_into_view(view, &pane, cx).await?.center();

    drag_from(view, &format!("tab-{tab}"), target, Some("m5-54-tab-drop"), cx).await?;
    let (after, line) = prompt_is(&terminal, "the claude tab dropped on the plain terminal", name, cx).await?;
    sleep(cx, SETTLE).await;
    if prompts(&terminal, cx) != (after, line.clone()) || after != before {
        bail!("the dropped name ran: {before} prompt(s) before, {:?} after the drop", prompts(&terminal, cx));
    }
    terminal.update(cx, |t, _| t.input(b"\x1b".to_vec()));
    prompt_is(&terminal, "Esc after the tab drop", PROMPT, cx).await?;

    drag_from(view, &format!("session-row-{sid}"), target, None, cx).await?;
    let (rowed, row_line) = prompt_is(&terminal, "the session row dropped on the plain terminal", name, cx).await?;
    sleep(cx, SETTLE).await;
    if prompts(&terminal, cx) != (rowed, row_line) || rowed != before {
        bail!("the row's name ran: {before} prompt(s) before, {:?} after the drop", prompts(&terminal, cx));
    }
    terminal.update(cx, |t, _| t.input(b"\x1b".to_vec()));
    let cleared = prompt_is(&terminal, "Esc after the row drop", PROMPT, cx).await?;

    drag_from(view, &format!("tab-{plain}"), target, None, cx).await?;
    sleep(cx, SETTLE).await;
    let toast = view.read_with(cx, |v, _| v.pane_toast.text().map(String::from));
    if prompts(&terminal, cx) != cleared || toast.is_some() {
        bail!("the plain tab dropped on its own terminal: prompt {:?}, toast {toast:?}; expected nothing", prompts(&terminal, cx).1);
    }

    let bare = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(cwd.to_path_buf(), "bare".into(), window, cx)))?;
    click(view, &format!("tab-{plain}"), cx).await?;
    until(view, cx, WITHIN, "the plain tab active again", |v| (v.active == Some(plain)).then_some(())).await?;
    let target = scrolled_into_view(view, &pane, cx).await?.center();
    drag_from(view, &format!("tab-{bare}"), target, None, cx).await?;
    let why = until(view, cx, WITHIN, "the toast for a tab without claude", |v| {
        let text = v.pane_toast.text().filter(|t| t.contains("runs no claude session"))?;
        v.hits.borrow().contains_key("pane-toast").then(|| text.to_string())
    })
    .await?;
    sleep(cx, SETTLE).await;
    if prompts(&terminal, cx) != cleared {
        bail!("a tab without claude pasted something: prompt {:?}", prompts(&terminal, cx).1);
    }
    view.update(cx, |v, cx| v.close_tab(bare, cx));

    let held = text(cx)?;
    if sequence() != clipboard || held.as_deref() != Some(SENTINEL) {
        bail!("the drops touched the clipboard: it holds {held:?}, sequence {clipboard} → {}", sequence());
    }
    Ok(format!(
        "tab and session row dropped on a pwsh terminal pasted {name:?} without running it; its own tab pasted nothing; \
         a tab without claude pasted nothing and toasted {why:?}; the clipboard was not touched"
    ))
}

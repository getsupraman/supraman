//! `--selftest` step 7l (m5/33, m5/34): a shell tab of this Foreman, in the gate's session cwd, where
//! `claude --name fm-selftest-<hex>-mark` is typed. Its row must be marked (the time from Enter to the
//! painted mark is measured); a plain shell tab marks nothing and shows the row's `tab` link, which brings
//! the claude tab back. On that row, Copy SendMessage name puts the name `~/.claude/sessions` registered on
//! the clipboard, a toast says so, and the row's y is the same before and after. `/exit` clears the mark.
//! The claude's pid is recorded for [`crate::selftest::cleanup`], which kills it if the step does not end it.

use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use foreman_claude::peer;
use gpui::{AsyncWindowContext, ClipboardItem, Entity, MouseButton};

use crate::{
    selftest::{CLIPBOARD_TRIES, cargo_root, click, hex6, lock, press, read_own, sleep, wait_for_tab_terminal},
    view::{FleetView, REFRESH_EVERY},
};

const STARTED_WITHIN: Duration = Duration::from_secs(40);
const WITHIN: Duration = Duration::from_secs(10);

async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, within: Duration, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = view.read_with(cx, |v, _| f(v)) {
            return Ok(t);
        }
        if started.elapsed() > within {
            bail!("{what}: not within {} ms", within.as_millis());
        }
        sleep(cx, Duration::from_millis(25)).await;
    }
}

/// The session ids whose rows were painted marked in the last frame.
fn marked(v: &FleetView) -> Vec<String> {
    v.hits.borrow().keys().filter_map(|k| k.strip_prefix("marked-")).map(String::from).collect()
}

/// Kills `pid` only while it is still the process created at `start`.
pub(crate) fn kill_own(pid: u32, start: u64) {
    if peer::started(pid) != Some(start) {
        return;
    }
    #[cfg(windows)]
    // SAFETY: a handle this function opens and closes, to a process this run started.
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
        if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
        }
    }
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let earlier: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| earlier.into_iter().for_each(|id| v.close_tab(id, cx)));
    let saved = cx.update(|_, cx| cx.read_from_clipboard())?;
    let name = format!("fm-selftest-{}-mark", hex6());
    let shell = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, &name, cx).await;
    view.update(cx, |v, _| v.shell_args = shell);
    if let Some(saved) = saved {
        cx.update(|_, cx| cx.write_to_clipboard(saved)).ok();
    }
    let killed = lock().as_mut().map(|s| std::mem::take(&mut s.kill)).unwrap_or_default();
    for (pid, start) in killed {
        kill_own(pid, start);
    }
    let mine: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| mine.into_iter().for_each(|id| v.close_tab(id, cx)));
    result
}

async fn check(view: &Entity<FleetView>, name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let cwd = cargo_root().join("target").join("selftest-cwd");
    let tab = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(cwd.clone(), "mark".into(), window, cx)))?;
    let terminal = wait_for_tab_terminal(view, tab, WITHIN, cx).await?;
    // PSReadLine is up once the prompt shows the folder
    let started = Instant::now();
    while !terminal.read_with(cx, |t, _| t.get_content()).contains("selftest-cwd>") {
        if started.elapsed() > WITHIN {
            bail!("the shell tab showed no prompt in {}", cwd.display());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    if !view.read_with(cx, |v, _| marked(v)).is_empty() {
        bail!("a shell tab without claude marks a row");
    }

    terminal.update(cx, |t, _| t.input(format!("claude --name {name} --permission-mode default\r").into_bytes()));
    let entered = Instant::now();
    let dir = peer::sessions_dir().ok_or_else(|| anyhow!("no home directory for ~/.claude/sessions"))?;
    let entry = loop {
        let found = peer::live_entries(&dir, &peer::System).into_iter().find(|e| e.name == name);
        if let Some(e) = found.filter(|e| e.session_id.is_some()) {
            break e;
        }
        if entered.elapsed() > STARTED_WITHIN {
            bail!("{name} never registered in {}", dir.display());
        }
        sleep(cx, Duration::from_millis(50)).await;
    };
    if let (Some(scratch), Some(start)) = (lock().as_mut(), entry.proc_start) {
        scratch.kill.push((entry.pid, start));
    }
    let registered = entered.elapsed();
    let sid = entry.session_id.clone().unwrap_or_default();
    crate::selftest::forget_own_transcript(&sid, &cwd);
    until(view, cx, STARTED_WITHIN, &format!("{name}'s row marked"), |v| marked(v).contains(&sid).then_some(())).await?;
    let to_mark = entered.elapsed();
    println!("step 7l: claude+Enter → registered {} ms, row marked {} ms", registered.as_millis(), to_mark.as_millis());
    let others = view.read_with(cx, |v, _| marked(v));
    if others.len() != 1 {
        bail!("{} rows marked, expected only {name}'s: {others:?}", others.len());
    }

    // another tab: nothing marked, the row links back to its tab
    let plain = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(cwd.clone(), "plain".into(), window, cx)))?;
    wait_for_tab_terminal(view, plain, WITHIN, cx).await?;
    until(view, cx, WITHIN, "no row marked with a plain shell tab active", |v| marked(v).is_empty().then_some(())).await?;
    let link = format!("tab-link-{}", &sid[..8]);
    until(view, cx, WITHIN, "the row's tab link", |v| v.hits.borrow().contains_key(&link).then_some(())).await?;
    click(view, &link, cx).await?;
    until(view, cx, WITHIN, "the claude tab active again from the row's tab link", |v| (v.active == Some(tab)).then_some(())).await?;
    until(view, cx, WITHIN, "the mark back on the row", |v| marked(v).contains(&sid).then_some(())).await?;

    let copied = copy_send_name(view, &sid, name, cx).await.context("Copy SendMessage name")?;

    terminal.update(cx, |t, _| t.input(b"/exit\r".to_vec()));
    let asked = Instant::now();
    until(view, cx, REFRESH_EVERY + WITHIN, "the mark cleared after /exit", |v| {
        (marked(v).is_empty() && v.tab(tab).is_some_and(|t| t.claude.is_none())).then_some(())
    })
    .await?;
    let cleared = asked.elapsed();
    // claude takes its registry entry away before its process ends
    let runs = || peer::started(entry.pid).is_some_and(|s| Some(s) == entry.proc_start);
    while runs() {
        if asked.elapsed() > REFRESH_EVERY + WITHIN {
            bail!("the mark cleared but {name} (pid {}) still runs {} ms after /exit", entry.pid, asked.elapsed().as_millis());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    Ok(format!(
        "{name} typed in a shell tab: registered after {} ms, row marked after {} ms; a plain tab marks nothing and its \
         tab link brings the claude tab back; {copied}; /exit cleared the mark after {} ms",
        registered.as_millis(),
        to_mark.as_millis(),
        cleared.as_millis()
    ))
}

/// Right-click → Copy SendMessage name: the clipboard holds `name` (unique, so no `[ref]`), a toast says so
/// and the row has not moved.
async fn copy_send_name(view: &Entity<FleetView>, sid: &str, name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let row = format!("session-row-{sid}");
    let top = |v: &FleetView| v.hits.borrow().get(&row).map(|h| h.bounds.top());
    let before = until(view, cx, WITHIN, "the row painted", top).await?;
    let read = |cx: &mut AsyncWindowContext| cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    let mut copied = None;
    for _ in 0..CLIPBOARD_TRIES {
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("not copied".into())))?;
        press(view, &row, MouseButton::Right, cx).await?;
        let shown = until(view, cx, WITHIN, "Copy SendMessage name in the menu", |v| v.hits.borrow().contains_key("menu-copy-send-name").then_some(())).await;
        if let Err(e) = shown {
            let menu = view.read_with(cx, |v, _| v.menu.as_ref().map(|m| m.items.iter().map(|i| i.key).collect::<Vec<_>>()));
            let dir = peer::sessions_dir().unwrap_or_default();
            let live: Vec<String> = peer::live_entries(&dir, &peer::System).iter().filter(|e| e.name == name).map(|e| format!("{:?}", e)).collect();
            bail!("{e}: menu {menu:?}; live entries named {name}: {live:?}");
        }
        click(view, "menu-copy-send-name", cx).await?;
        copied = read_own(cx, &read, |c| c == name).await?;
        if copied.as_deref() == Some(name) {
            break;
        }
    }
    if copied.as_deref() != Some(name) {
        bail!("the clipboard holds {copied:?}, not {name:?} ({CLIPBOARD_TRIES} tries)");
    }
    until(view, cx, WITHIN, "the copy toast", |v| {
        (v.toast.as_ref().is_some_and(|t| t.text == format!("Copied {name}")) && v.hits.borrow().contains_key("toast")).then_some(())
    })
    .await?;
    // two frames later, with the toast painted
    sleep(cx, Duration::from_millis(100)).await;
    let after = until(view, cx, WITHIN, "the row painted after the copy", top).await?;
    if after != before {
        bail!("the row moved from y {:.1} to {:.1} after Copy", f32::from(before), f32::from(after));
    }
    Ok(format!("Copy SendMessage name copied {name:?}, toast shown, the row stayed at y {:.1}", f32::from(after)))
}

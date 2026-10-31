use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::peer;
use gpui::{AsyncWindowContext, Bounds, Entity, MouseButton, Pixels};

use crate::{
    selftest::{CLIPBOARD_TRIES, cargo_root, click, double_click, lock, press, read_own, run_hex, sleep, until, wait_for_tab_terminal},
    view::{FleetView, REFRESH_EVERY},
};

const STARTED_WITHIN: Duration = Duration::from_secs(40);
const WITHIN: Duration = Duration::from_secs(10);

fn marked(v: &FleetView) -> Vec<String> {
    v.hits.borrow().keys().filter_map(|k| k.strip_prefix("marked-")).map(String::from).collect()
}

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
    let _clipboard = crate::selftest_clipboard::borrow("7l", cx).await?;
    let saved = crate::selftest_clipboard::Snapshot::take()?;
    let name = format!("fm-selftest-{}-mark", run_hex());
    let shell = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, &name, cx).await;
    view.update(cx, |v, _| v.shell_args = shell);
    saved.restore().ok();
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

    let plain = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(cwd.clone(), "plain".into(), window, cx)))?;
    wait_for_tab_terminal(view, plain, WITHIN, cx).await?;
    until(view, cx, WITHIN, "no row marked with a plain shell tab active", |v| marked(v).is_empty().then_some(())).await?;
    let link = format!("tab-link-{}", &sid[..8]);
    until(view, cx, WITHIN, "the row's tab link", |v| v.hits.borrow().contains_key(&link).then_some(())).await?;
    click(view, &link, cx).await?;
    until(view, cx, WITHIN, "the claude tab active again from the row's tab link", |v| (v.active == Some(tab)).then_some(())).await?;
    until(view, cx, WITHIN, "the mark back on the row", |v| marked(v).contains(&sid).then_some(())).await?;
    view.update(cx, |v, cx| v.activate_tab(plain, cx));
    until(view, cx, WITHIN, "the plain tab active again", |v| (v.active == Some(plain) && marked(v).is_empty()).then_some(())).await?;
    let tabs = view.read_with(cx, |v, _| v.tabs.len());
    double_click(view, &format!("session-row-{sid}"), cx).await?;
    until(view, cx, WITHIN, "the claude tab active again from a double-click on its row", |v| (v.active == Some(tab)).then_some(())).await?;
    if view.read_with(cx, |v, _| v.tabs.len()) != tabs {
        bail!("a double-click on the row of a session open in a tab opened another tab");
    }

    let copied = copy_send_name(view, &sid, name, cx).await.context("Copy SendMessage name")?;
    let dropped = crate::selftest_drop::run(view, [tab, plain], &sid, name, &cwd, cx).await.context("drop on a terminal")?;
    println!("step 7l: {dropped}");

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
         tab link and a double-click on its row each bring the claude tab back; {copied}; /exit cleared the mark after {} ms",
        registered.as_millis(),
        to_mark.as_millis(),
        cleared.as_millis()
    ))
}

/// Rows re-sort and notices come and go between polls, so a row's own y proves nothing.
async fn copy_send_name(view: &Entity<FleetView>, sid: &str, name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let row = format!("session-row-{sid}");
    let tree = |v: &FleetView| v.hits.borrow().get("tree-area").map(|h| h.bounds);
    until(view, cx, WITHIN, "the row painted", |v| v.hits.borrow().contains_key(&row).then_some(())).await?;
    let before = until(view, cx, WITHIN, "the tree area painted", tree).await?;
    let mut copied = None;
    for _ in 0..CLIPBOARD_TRIES {
        crate::selftest_clipboard::put_text("not copied", cx)?;
        press(view, &row, MouseButton::Right, cx).await?;
        let shown = until(view, cx, WITHIN, "Copy SendMessage name in the menu", |v| v.hits.borrow().contains_key("menu-copy-send-name").then_some(())).await;
        if let Err(e) = shown {
            let menu = view.read_with(cx, |v, _| v.menu.as_ref().map(|m| m.items.iter().map(|i| i.key).collect::<Vec<_>>()));
            let dir = peer::sessions_dir().unwrap_or_default();
            let live: Vec<String> = peer::live_entries(&dir, &peer::System).iter().filter(|e| e.name == name).map(|e| format!("{:?}", e)).collect();
            bail!("{e}: menu {menu:?}; live entries named {name}: {live:?}");
        }
        click(view, "menu-copy-send-name", cx).await?;
        copied = read_own(cx, |c| c == name).await?;
        if copied.as_deref() == Some(name) {
            break;
        }
    }
    if copied.as_deref() != Some(name) {
        bail!("the clipboard holds {copied:?}, not {name:?} ({CLIPBOARD_TRIES} tries)");
    }
    let after = until(view, cx, WITHIN, "the copy toast painted", |v| {
        let shown = v.toast.text() == Some(format!("Copied {name}").as_str()) && v.hits.borrow().contains_key("toast");
        shown.then(|| tree(v)).flatten()
    })
    .await?;
    let edges = |b: Bounds<Pixels>| (f32::from(b.top()), f32::from(b.bottom()), f32::from(b.size.height));
    let ((top0, bottom0, height0), (top1, bottom1, height1)) = (edges(before), edges(after));
    // a notice or Inbox row above may push the top down, and the height with it; a toast in the flow lifts the bottom
    if (bottom1 - bottom0).abs() > 0.5 {
        bail!("the toast took room from the tree: tree area top {top0:.1} → {top1:.1}, bottom {bottom0:.1} → {bottom1:.1}, height {height0:.1} → {height1:.1}");
    }
    Ok(format!(
        "Copy SendMessage name copied {name:?}, toast shown over the tree: tree area bottom {bottom1:.1}, height {height0:.1} → {height1:.1}"
    ))
}

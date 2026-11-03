use std::time::{Duration, Instant};

use anyhow::bail;
use supraman_term::Terminal;
use gpui::{
    AsyncWindowContext, Entity, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, PlatformInput,
};

use crate::{
    selftest::{CLIPBOARD_TRIES, hex6, key, read_own, select_wheel_line, sleep},
    selftest_clipboard::put_text,
};

pub(crate) async fn run(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let _clipboard = crate::selftest_clipboard::borrow("6b", cx).await?;
    let saved = crate::selftest_clipboard::Snapshot::take()?;
    let result = steps(terminal, cx).await;
    saved.restore()?;
    result
}

fn last_line(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> String {
    let content = terminal.read_with(cx, |t, _| t.get_content());
    content.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().trim_end().to_string()
}

fn right_click(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let position = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds.bounds.center());
    cx.update(|window, cx| {
        let none = Modifiers::default();
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position, modifiers: none, click_count: 1, first_mouse: false }), cx);
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position, modifiers: none, click_count: 1 }), cx);
    })
}

async fn until(what: &str, within: Duration, cx: &mut AsyncWindowContext, mut done: impl FnMut(&mut AsyncWindowContext) -> bool) -> anyhow::Result<Duration> {
    let started = Instant::now();
    crate::selftest::until_with(cx, within, what, |cx| done(cx).then_some(())).await?;
    Ok(started.elapsed())
}

/// Windows Terminal's copy keys: Ctrl+C with a selection copies and never reaches the shell; right-click copies a
/// selection, or pastes without one.
async fn steps(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let ctrl = Modifiers::control();
    let mut report = Vec::new();

    let prompt = last_line(terminal, cx);
    let mut copied = None;
    let mut text = String::new();
    for _ in 0..CLIPBOARD_TRIES {
        (_, text) = select_wheel_line(terminal, cx).await?;
        put_text("not copied", cx)?;
        key(cx, "c", ctrl)?;
        copied = read_own(cx, |c| c == text).await?;
        if copied.as_deref() == Some(text.as_str()) {
            break;
        }
    }
    if copied.as_deref() != Some(text.as_str()) {
        bail!("Ctrl+C with {text:?} selected left the clipboard at {copied:?} ({CLIPBOARD_TRIES} tries)");
    }
    sleep(cx, Duration::from_millis(400)).await;
    let selection = terminal.read_with(cx, |t, _| t.selection_text());
    if selection.is_some() {
        bail!("Ctrl+C copied {text:?} but left the selection {selection:?}");
    }
    let after = last_line(terminal, cx);
    if after != prompt {
        bail!("Ctrl+C with a selection reached the shell: the prompt line {prompt:?} became {after:?}");
    }
    report.push(format!("Ctrl+C with {text:?} selected → clipboard, selection cleared, prompt line unchanged"));

    let mut copied = None;
    for _ in 0..CLIPBOARD_TRIES {
        (_, text) = select_wheel_line(terminal, cx).await?;
        put_text("not copied", cx)?;
        right_click(terminal, cx)?;
        copied = read_own(cx, |c| c == text).await?;
        if copied.as_deref() == Some(text.as_str()) {
            break;
        }
    }
    if copied.as_deref() != Some(text.as_str()) {
        bail!("right-click with {text:?} selected left the clipboard at {copied:?} ({CLIPBOARD_TRIES} tries)");
    }
    sleep(cx, Duration::from_millis(200)).await;
    if terminal.read_with(cx, |t, _| t.last_content().selection.is_some()) {
        bail!("right-click copied {text:?} but kept the selection");
    }

    let marker = format!("fm-rclick-{}", hex6());
    let mut pasted = false;
    for _ in 0..CLIPBOARD_TRIES {
        put_text(&marker, cx)?;
        right_click(terminal, cx)?;
        if until("right-click paste echoed", Duration::from_secs(2), cx, |cx| last_line(terminal, cx).contains(&marker)).await.is_ok() {
            pasted = true;
            break;
        }
    }
    if !pasted {
        bail!("right-click: {marker} not echoed at the prompt in {CLIPBOARD_TRIES} tries of 2 s");
    }
    report.push(format!("right-click with a selection → clipboard, cleared; without → pasted {marker}"));

    Ok(report.join("; "))
}

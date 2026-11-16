//! `--selftest` step 6b: a real pwsh in a terminal tab: wheel scroll, and Ctrl+C and right-click copy and paste through the clipboard.

use std::time::{Duration, Instant};

use anyhow::bail;
use supraman_term::Terminal;
use gpui::{
    AsyncWindowContext, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformInput, Point, ScrollDelta, ScrollEvent, point,
};

use crate::selftest::{copied, drawn, hex6, key, put_clipboard, select_wheel_line, sleep, until_with};


pub(crate) async fn wheel(terminal: &Entity<Terminal>, position: Point<Pixels>, notches: f32, cx: &mut AsyncWindowContext) -> anyhow::Result<usize> {
    let before = terminal.read_with(cx, |t, _| t.grid_extent().0);
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
        // a Windows wheel notch: 3 lines, positive is up (gpui_windows events.rs)
        let event = ScrollEvent { position, delta: ScrollDelta::Lines(point(0., 3. * notches)), ..Default::default() };
        window.dispatch_event(PlatformInput::Scroll(event), cx);
    })?;
    drawn(cx).await?;
    let mut seen = (before, before);
    let moved = until_with(cx, Duration::from_secs(3), "the wheel's scroll", |cx| {
        seen = terminal.read_with(cx, |t, _| (t.grid_extent().0, t.last_content().display_offset));
        (seen.0 != before && seen.0 == seen.1).then_some(seen.0)
    })
    .await;
    moved.map_err(|e| anyhow::anyhow!("{notches} notches from display_offset {before}: {e}; the terminal at {}, the last frame painted {}", seen.0, seen.1))
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
pub(crate) async fn run(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let ctrl = Modifiers::control();
    let mut report = Vec::new();

    let prompt = last_line(terminal, cx);
    let (_, text) = select_wheel_line(terminal, cx).await?;
    put_clipboard("not copied", cx)?;
    key(cx, "c", ctrl)?;
    let held = copied(&text, cx).await?;
    if held.as_deref() != Some(text.as_str()) {
        bail!("Ctrl+C with {text:?} selected left the clipboard at {held:?}");
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

    let (_, text) = select_wheel_line(terminal, cx).await?;
    put_clipboard("not copied", cx)?;
    right_click(terminal, cx)?;
    let held = copied(&text, cx).await?;
    if held.as_deref() != Some(text.as_str()) {
        bail!("right-click with {text:?} selected left the clipboard at {held:?}");
    }
    crate::selftest::drawn(cx).await?;
    if terminal.read_with(cx, |t, _| t.last_content().selection.is_some()) {
        bail!("right-click copied {text:?} but kept the selection");
    }

    let marker = format!("fm-rclick-{}", hex6());
    put_clipboard(&marker, cx)?;
    right_click(terminal, cx)?;
    until("right-click paste echoed", Duration::from_secs(5), cx, |cx| last_line(terminal, cx).contains(&marker))
        .await
        .map_err(|e| anyhow::anyhow!("right-click: {marker} not echoed at the prompt: {e}"))?;
    report.push(format!("right-click with a selection → clipboard, cleared; without → pasted {marker}"));

    Ok(report.join("; "))
}

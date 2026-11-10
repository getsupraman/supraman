//! `--selftest` step 6: a real pwsh in a shell tab, scrolled by the wheel, a line selected by a drag, copied and pasted through the system clipboard.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point, px};
use supraman_term::Terminal;

use super::{CLIPBOARD_TRIES, click, drawn, hex6, key, read_own, sleep, step, until_with, wait_for_tab_terminal};
use crate::view::FleetView;

/// The row is read from the last frame but the drag lands on the grid as it is: it waits for a frame that caught up, and drags again when the grid moved.
pub(crate) async fn select_wheel_line(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<(i32, String)> {
    let mut moved = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        loop {
            let (frame, grid) = terminal.read_with(cx, |t, _| ((t.last_content().display_offset, t.last_content().total_lines), t.grid_extent()));
            if frame == grid {
                break;
            }
            if started.elapsed() > Duration::from_secs(2) {
                bail!("the frame never caught up with the grid: (display_offset, total_lines) {frame:?} painted, {grid:?} now");
            }
            sleep(cx, Duration::from_millis(20)).await;
        }
        let (bounds, found, grid) = terminal.read_with(cx, |t, _| {
            let content = t.last_content();
            let mut rows: std::collections::BTreeMap<i32, String> = Default::default();
            for cell in &content.cells {
                rows.entry(cell.point.line).or_default().push(cell.cell.character());
            }
            let found = rows.into_iter().find_map(|(line, text)| {
                let text = text.trim_end().to_string();
                text.starts_with("wheel line ").then_some((line + content.display_offset as i32, text))
            });
            (content.terminal_bounds, found, t.grid_extent())
        });
        let (row, text) = found.ok_or_else(|| anyhow!("no `wheel line` row on screen"))?;
        let y = bounds.bounds.origin.y + bounds.line_height * (row as f32 + 0.5);
        let x = |column: f32| bounds.bounds.origin.x + bounds.cell_width * column;
        let (from, to) = (point(x(0.25), y), point(x(text.chars().count() as f32 - 0.25), y));
        let left = Some(MouseButton::Left);
        cx.update(|window, cx| {
            let none = Modifiers::default();
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers: none }), cx);
            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: from, modifiers: none, click_count: 1, first_mouse: false }), cx);
            for step in 1..=4 {
                let position = point(from.x + (to.x - from.x) * (step as f32 / 4.), y);
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: left, modifiers: none }), cx);
            }
            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: to, modifiers: none, click_count: 1 }), cx);
        })?;
        drawn(cx).await?;
        let read = |cx: &mut AsyncWindowContext| terminal.read_with(cx, |t, _| (t.selection_text(), t.grid_extent()));
        let (selected, now) = until_with(cx, Duration::from_secs(2), "the drag's selection", |cx| Some(read(cx)).filter(|(s, n)| s.as_deref() == Some(text.as_str()) || *n != grid)).await.unwrap_or_else(|_| read(cx));
        if selected.as_deref() == Some(text.as_str()) {
            return Ok((row, text));
        }
        if now == grid {
            bail!("dragged over display row {row} ({text:?}) from {from:?} to {to:?}; selection is {selected:?} (the grid held at {grid:?})");
        }
        moved.push(format!("{text:?} → {selected:?} while the grid went {grid:?} → {now:?}"));
    }
    bail!("output moved the grid under each of 3 drags: {}", moved.join("; "))
}

async fn select_copy_paste(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let _clipboard = crate::selftest_clipboard::borrow("6b", cx).await?;
    let saved = crate::selftest_clipboard::Snapshot::take()?;
    let result = async {
        let (mut row, mut text) = select_wheel_line(terminal, cx).await?;

        // Another selftest may write the system clipboard at any moment: each read waits for this run's own value, pressing again if a foreign one replaced it.
        let mut copied = None;
        for _ in 0..CLIPBOARD_TRIES {
            // A copy whose OpenClipboard failed (gpui_windows clipboard.rs) still clears the selection.
            if terminal.read_with(cx, |t, _| t.selection_text()).as_deref() != Some(text.as_str()) {
                (row, text) = select_wheel_line(terminal, cx).await?;
            }
            crate::selftest_clipboard::put_text("not copied", cx)?;
            key(cx, "c", Modifiers { control: true, shift: true, ..Default::default() })?;
            copied = read_own(cx, |c| c == text).await?;
            if copied.as_deref() == Some(text.as_str()) {
                break;
            }
        }
        if copied.as_deref() != Some(text.as_str()) {
            bail!("Ctrl+Shift+C with {text:?} selected left the clipboard at {copied:?} ({CLIPBOARD_TRIES} tries)");
        }

        let marker = format!("fm-paste-{}", hex6());
        let echoed = |cx: &mut AsyncWindowContext| {
            let content = terminal.read_with(cx, |t, _| t.get_content());
            content.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| l.contains(&marker))
        };
        let pasted = Instant::now();
        'paste: for _ in 0..CLIPBOARD_TRIES {
            crate::selftest_clipboard::put_text(&marker, cx)?;
            key(cx, "v", Modifiers::control())?;
            let tried = Instant::now();
            while tried.elapsed() < Duration::from_secs(2) {
                if echoed(cx) {
                    break 'paste;
                }
                sleep(cx, Duration::from_millis(50)).await;
            }
        }
        if !echoed(cx) {
            bail!("Ctrl+V: {marker} not echoed at the prompt in {CLIPBOARD_TRIES} tries of 2 s");
        }
        Ok(format!(
            "drag over row {row} selected {text:?}; Ctrl+Shift+C → clipboard {text:?}; Ctrl+V → {marker} at the prompt after {} ms",
            pasted.elapsed().as_millis()
        ))
    }
    .await;
    saved.restore()?;
    result
}


pub(super) async fn wheel_scroll(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max());
    // the user's profile may print another prompt, or take long; the perf mode does the same
    let args = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let clicked = click(view, "new-shell", cx).await;
    view.update(cx, |v, _| v.shell_args = args);
    clicked?;
    let tab = view
        .read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max())
        .filter(|id| Some(*id) != before)
        .ok_or_else(|| anyhow!("clicking + opened no tab"))?;
    let result = async {
        let terminal = wait_for_tab_terminal(view, tab, Duration::from_secs(10), cx).await?;
        let opened = Instant::now();
        // a terminal is built with placeholder bounds at the origin; output sized from those leaves too little scrollback
        let lines = loop {
            let bounds = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds);
            if bounds.bounds.origin != point(px(0.), px(0.)) {
                break bounds.num_lines();
            }
            if opened.elapsed() > Duration::from_secs(10) {
                bail!("tab {tab}: the grid never laid the terminal out (bounds {bounds:?})");
            }
            sleep(cx, Duration::from_millis(20)).await;
        };
        let count = lines * 2 + 20;
        // the tag makes step 6b's copied line this run's own
        let tag = format!("fm-{}", hex6());
        let last = format!("wheel line {count} {tag}");
        let mut typed = false;
        loop {
            let content = terminal.read_with(cx, |t, _| t.get_content());
            if !typed && content.contains("PS ") {
                let command = format!("1..{count} | % {{ \"wheel line $_ {tag}\" }}\r");
                terminal.update(cx, |t, _| t.input(command.into_bytes()));
                typed = true;
            }
            if typed && content.lines().any(|l| l.trim_end() == last) && content.trim_end().ends_with('>') {
                break;
            }
            if opened.elapsed() > Duration::from_secs(15) {
                let tail = content.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().to_string();
                bail!("tab {tab}: no prompt after {last:?} within 15 s (typed: {typed}; last line {tail:?})");
            }
            sleep(cx, Duration::from_millis(50)).await;
        }
        let (grid, scrollback) = terminal.read_with(cx, |t, _| {
            let bounds = t.last_content().terminal_bounds;
            (bounds.bounds, t.total_lines().saturating_sub(bounds.num_lines()))
        });
        if scrollback < 6 {
            bail!("{count} lines in a {lines}-line grid left {scrollback} lines of scrollback; two notches up need 6");
        }
        let offset = async |notches: f32, cx: &mut AsyncWindowContext| crate::selftest_term::wheel(&terminal, grid.center(), notches, cx).await;
        let up = offset(2., cx).await?;
        if up == 0 {
            bail!("two wheel notches up over the grid {grid:?} left display_offset at 0");
        }
        let half = offset(-1., cx).await?;
        let bottom = offset(-1., cx).await?;
        if !(half > 0 && half < up) || bottom != 0 {
            bail!("wheel down: display_offset {up} → {half} → {bottom}, expected {up} → 1..{up} → 0");
        }
        let wheel = format!("{count} lines in a {lines}-line grid; display_offset 0 → {up} (2 notches up) → {half} → {bottom} (2 down)");
        let copied = step("6b", select_copy_paste(&terminal, cx)).await?;
        let keys = step("6b", crate::selftest_term::run(&terminal, cx)).await?;
        println!("step 6b: {copied}; {keys}");
        let linked = step("6d", crate::selftest_link::run(view, tab, &terminal, cx)).await?;
        println!("step 6d: {linked}");
        Ok(wheel)
    }
    .await;
    view.update(cx, |v, cx| v.close_tab(tab, cx));
    result
}

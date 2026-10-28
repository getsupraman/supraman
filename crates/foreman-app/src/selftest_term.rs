//! `--selftest` step 6c (m5/23): the terminal behaves like Windows Terminal. On step 6's shell tab, after
//! step 6b left its marker at the PSReadLine prompt: Ctrl+C with a selection copies it and the shell gets
//! no ^C; PSReadLine's parameter colour is Campbell's bright black; Ctrl+C without a selection cancels the
//! line; a right-click with a selection copies it and without one pastes; the cursor is a blinking bar,
//! lit right after a key; Ctrl+Shift+F finds; Ctrl+= / Ctrl+0 zoom the terminals. The clipboard is shared with
//! other selftests (m5/15), so every clipboard check retries as step 6b does, and the clipboard is put back.

use std::time::{Duration, Instant};

use anyhow::bail;
use foreman_term::{Color, CursorShape, NamedColor, Terminal, terminal_settings::TerminalConfig};
use gpui::{
    AsyncWindowContext, ClipboardItem, Entity, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, PlatformInput, rgb,
};

use crate::{
    selftest::{CLIPBOARD_TRIES, hex6, key, read_own, select_wheel_line, sleep},
    view::FleetView,
};

pub(crate) async fn run(view: &Entity<FleetView>, tab: u64, terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = crate::selftest_clipboard::Snapshot::take()?;
    let result = steps(view, tab, terminal, cx).await;
    saved.restore()?;
    result
}

fn last_line(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> String {
    let content = terminal.read_with(cx, |t, _| t.get_content());
    content.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().trim_end().to_string()
}

fn clipboard(cx: &mut AsyncWindowContext) -> anyhow::Result<Option<String>> {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

fn put(text: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())))
}

fn right_click(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let position = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds.bounds.center());
    cx.update(|window, cx| {
        let none = Modifiers::default();
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position, modifiers: none, click_count: 1, first_mouse: false }), cx);
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position, modifiers: none, click_count: 1 }), cx);
    })
}

/// `text` typed into the focused find bar: keystrokes no binding takes, which gpui hands to the input's
/// handler as a real key's WM_CHAR is (m5/42).
fn typed(text: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    for c in text.chars() {
        let name = if c == ' ' { "space".to_string() } else { c.to_string() };
        let keystroke = Keystroke { modifiers: Modifiers::default(), key: name, key_char: Some(c.to_string()) };
        cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx))?;
    }
    Ok(())
}

async fn until(what: &str, within: Duration, cx: &mut AsyncWindowContext, mut done: impl FnMut(&mut AsyncWindowContext) -> bool) -> anyhow::Result<Duration> {
    let started = Instant::now();
    loop {
        if done(cx) {
            return Ok(started.elapsed());
        }
        if started.elapsed() > within {
            bail!("{what}: not within {} ms", within.as_millis());
        }
        sleep(cx, Duration::from_millis(30)).await;
    }
}

async fn steps(view: &Entity<FleetView>, tab: u64, terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let ctrl = Modifiers::control();
    let mut report = Vec::new();

    // Ctrl+C with a selection: the clipboard gets the text, the selection goes, the prompt line stays
    let prompt = last_line(terminal, cx);
    let mut copied = None;
    let mut text = String::new();
    for _ in 0..CLIPBOARD_TRIES {
        (_, text) = select_wheel_line(terminal, cx).await?;
        put("not copied", cx)?;
        key(cx, "c", ctrl)?;
        copied = read_own(cx, &clipboard, |c| c == text).await?;
        if copied.as_deref() == Some(text.as_str()) {
            break;
        }
    }
    if copied.as_deref() != Some(text.as_str()) {
        bail!("Ctrl+C with {text:?} selected left the clipboard at {copied:?} ({CLIPBOARD_TRIES} tries)");
    }
    sleep(cx, Duration::from_millis(400)).await;
    let selection = terminal.read_with(cx, |t, _| t.last_content().selection_text.clone());
    if selection.is_some() {
        bail!("Ctrl+C copied {text:?} but left the selection {selection:?}");
    }
    let after = last_line(terminal, cx);
    if after != prompt {
        bail!("Ctrl+C with a selection reached the shell: the prompt line {prompt:?} became {after:?}");
    }
    report.push(format!("Ctrl+C with {text:?} selected → clipboard, selection cleared, prompt line unchanged"));

    // PSReadLine colours a parameter as bright black (\e[90m): the theme's #666666, not near-black
    terminal.update(cx, |t, _| t.input(b" -LiteralPath fm".to_vec()));
    until("PSReadLine echoed -LiteralPath", Duration::from_secs(3), cx, |cx| last_line(terminal, cx).ends_with("-LiteralPath fm"))
        .await?;
    sleep(cx, Duration::from_millis(200)).await;
    // the `L` of the last `-LiteralPath` on screen (the prompt's path may hold other L's)
    let fg = terminal.read_with(cx, |t, _| {
        let cells = &t.last_content().cells;
        let chars: Vec<char> = cells.iter().map(|c| c.cell.character()).collect();
        let needle: Vec<char> = "-LiteralPath".chars().collect();
        let at = chars.windows(needle.len()).rposition(|w| w == needle.as_slice())?;
        Some(cells[at + 1].cell.foreground())
    });
    // m5/31: the default theme's own bright black (VS Code Dark Modern #666666), in the OSC answers too
    let (index_8, theme_8, theme) = cx.update(|_, cx| {
        let theme = theme::ActiveTheme::theme(cx);
        (TerminalConfig::get(cx).palette.ansi[8], theme.colors().terminal_ansi_bright_black, theme.name.to_string())
    })?;
    if theme != foreman_ui::themes::DEFAULT_THEME || foreman_ui::themes::hex(index_8) != "#666666" || index_8 != theme_8 {
        bail!("palette index 8 is {index_8:?} under {theme}, VS Code Dark Modern's bright black is #666666 ({theme_8:?})");
    }
    crate::selftest_manage::shot("m5-23-psreadline-parameter", cx).await;
    if !matches!(fg, Some(Color::Named(NamedColor::BrightBlack)) | Some(Color::Indexed(8))) {
        bail!("PSReadLine drew `-LiteralPath` in {fg:?}, expected bright black (index 8)");
    }
    report.push(format!("palette[8] = {theme} bright black #666666; PSReadLine drew `-LiteralPath` in {fg:?}"));

    // Ctrl+C without a selection: ^C reaches PSReadLine, which drops the line for a fresh prompt
    let line = last_line(terminal, cx);
    key(cx, "c", ctrl)?;
    let took = until("^C cancelled the prompt line", Duration::from_secs(5), cx, |cx| {
        let now = last_line(terminal, cx);
        now != line && now.ends_with('>')
    })
    .await?;
    report.push(format!("Ctrl+C without a selection → ^C: a fresh prompt after {} ms", took.as_millis()));

    // right-click with a selection copies it and clears it
    let mut copied = None;
    for _ in 0..CLIPBOARD_TRIES {
        (_, text) = select_wheel_line(terminal, cx).await?;
        put("not copied", cx)?;
        right_click(terminal, cx)?;
        copied = read_own(cx, &clipboard, |c| c == text).await?;
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

    // right-click without a selection pastes
    let marker = format!("fm-rclick-{}", hex6());
    let mut pasted = false;
    for _ in 0..CLIPBOARD_TRIES {
        put(&marker, cx)?;
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

    // a blinking bar: lit and unlit frames while idle, lit right after a key. PSReadLine hides the cursor
    // (DECTCEM) while it redraws the line, and a frame painted then has none (m5/24): wait that out only.
    let shown = until("the cursor shown after PSReadLine's redraw", Duration::from_secs(1), cx, |cx| {
        terminal.read_with(cx, |t, _| t.last_content().cursor.shape != CursorShape::Hidden)
    })
    .await?;
    let (shape, blinking) = terminal.read_with(cx, |t, _| (t.last_content().cursor.shape, t.last_content().cursor_blinking));
    if shape != CursorShape::Bar || !blinking {
        bail!("cursor is {shape:?}, blinking {blinking}; expected a blinking Bar");
    }
    let (mut lit, mut unlit) = (0, 0);
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(2500) && (lit < 2 || unlit < 2) {
        if terminal.read_with(cx, |t, _| t.cursor_painted_lit) {
            lit += 1;
        } else {
            unlit += 1;
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
    if lit == 0 || unlit == 0 {
        bail!("idle for {} ms: {lit} samples with the cursor painted lit, {unlit} unlit; it does not blink", started.elapsed().as_millis());
    }
    let mut solid_after_key = true;
    for _ in 0..4 {
        // wait for an unlit frame, press End (PSReadLine moves nowhere), and look at the next frame
        until("an unlit cursor frame", Duration::from_secs(2), cx, |cx| !terminal.read_with(cx, |t, _| t.cursor_painted_lit)).await?;
        key(cx, "end", Modifiers::default())?;
        sleep(cx, Duration::from_millis(80)).await;
        solid_after_key &= terminal.read_with(cx, |t, _| t.cursor_painted_lit);
    }
    if !solid_after_key {
        bail!("the cursor was not painted lit within 80 ms of a key, from an unlit frame");
    }
    report.push(format!(
        "cursor a blinking Bar (shown after {} ms): {lit} lit / {unlit} unlit samples idle, lit 80 ms after End (×4)",
        shown.as_millis()
    ));

    // Ctrl+Shift+F finds; Enter steps; Esc closes and clears the highlights
    key(cx, "f", Modifiers { control: true, shift: true, ..Default::default() })?;
    // the bar's input takes typed characters once a frame painted it focused
    until("the find bar's input painted focused", Duration::from_secs(3), cx, |cx| {
        cx.update(|window, cx| {
            view.read(cx).tab(tab).and_then(|t| t.find.as_ref()).is_some_and(|f| {
                let input = f.input.read(cx);
                input.is_focused(window) && input.caret_painted.get()
            })
        })
        .unwrap_or(false)
    })
    .await?;
    typed("wheel line 1", cx)?;
    until("find counted matches", Duration::from_secs(3), cx, |cx| {
        view.read_with(cx, |v, _| v.tab(tab).and_then(|t| t.find.as_ref()).is_some_and(|f| f.count > 0 && f.at.is_some()))
    })
    .await?;
    let (count, first) = view.read_with(cx, |v, _| v.tab(tab).and_then(|t| t.find.as_ref()).map(|f| (f.count, f.at)).unwrap_or_default());
    key(cx, "enter", Modifiers::default())?;
    sleep(cx, Duration::from_millis(100)).await;
    let second = view.read_with(cx, |v, _| v.tab(tab).and_then(|t| t.find.as_ref()).and_then(|f| f.at));
    let selected = terminal.read_with(cx, |t, _| t.last_content().selection_text.clone());
    if count > 1 && second == first {
        bail!("find: Enter stayed on match {first:?} of {count}");
    }
    if !selected.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("wheel line 1")) {
        bail!("find: the selected match is {selected:?}");
    }
    // Shift+Enter steps back down to where Enter came from
    key(cx, "enter", Modifiers { shift: true, ..Default::default() })?;
    sleep(cx, Duration::from_millis(100)).await;
    let back = view.read_with(cx, |v, _| v.tab(tab).and_then(|t| t.find.as_ref()).and_then(|f| f.at));
    if back != first {
        bail!("find: Shift+Enter went to match {back:?}, not back to {first:?}");
    }
    key(cx, "enter", Modifiers::default())?;
    sleep(cx, Duration::from_millis(100)).await;
    // m5/26: the selection as Windows Terminal paints it: white, its text in the terminal's background
    let background = cx.update(|_, cx| theme::ActiveTheme::theme(cx).colors().terminal_background)?;
    until("the selection painted", Duration::from_secs(2), cx, |cx| terminal.read_with(cx, |t, _| t.selection_painted.is_some())).await?;
    let painted = terminal.read_with(cx, |t, _| t.selection_painted);
    let white: gpui::Hsla = rgb(0xFFFFFF).into();
    if painted != Some((white, background)) {
        bail!("the selection was painted (background, text) {painted:?}, expected white with the text in the background {background:?}");
    }
    crate::selftest_manage::shot("m5-26-selection", cx).await;
    report.push("selection painted white, its text in the terminal background (Windows Terminal)".into());
    key(cx, "escape", Modifiers::default())?;
    sleep(cx, Duration::from_millis(100)).await;
    let (open, highlights) = (
        view.read_with(cx, |v, _| v.tab(tab).is_some_and(|t| t.find.is_some())),
        terminal.read_with(cx, |t, _| t.matches.len()),
    );
    if open || highlights > 0 {
        bail!("find: Esc left the bar open ({open}) with {highlights} highlights");
    }
    report.push(format!("Ctrl+Shift+F \"wheel line 1\" typed into its input: {count} matches, Enter {first:?} → {second:?}, Shift+Enter back, Esc closed"));

    // Ctrl+= / Ctrl+- / Ctrl+0 zoom every terminal (m5/41; step 7q checks a second tab and the file)
    let size = |cx: &mut AsyncWindowContext| view.read_with(cx, |v, _| v.term_font_size);
    let before = size(cx);
    key(cx, "=", ctrl)?;
    key(cx, "=", ctrl)?;
    let bigger = size(cx);
    key(cx, "-", ctrl)?;
    let smaller = size(cx);
    key(cx, "0", ctrl)?;
    let reset = size(cx);
    if !(bigger > before && smaller < bigger && reset == before) {
        bail!("font size {before:?} → Ctrl+=×2 {bigger:?} → Ctrl+- {smaller:?} → Ctrl+0 {reset:?}");
    }
    report.push(format!("font {before:?} → {bigger:?} → {smaller:?} → {reset:?}"));

    Ok(report.join("; "))
}

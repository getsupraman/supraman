use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use foreman_term::Terminal;
use gpui::{
    AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
    Pixels, Point, point, px,
};

use crate::{
    selftest::{click, hex6, scrolled_into_view, sleep, wait_for_tab_terminal, until},
    selftest_work::WORKSPACE,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(10);
const READY: &str = "fm-keys ready";
const KEY: &str = "fm-key ";
pub(crate) const READER: &str = "[Console]::TreatControlCAsInput = $true\n\
Write-Host 'fm-keys ready'\n\
while ($true) { $k = [Console]::ReadKey($true); Write-Host ('fm-key ' + $k.Key + ' ' + [int]$k.Modifiers) }\n";

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let script = std::env::temp_dir().join(format!("fm-selftest-keys-{}.ps1", hex6()));
    std::fs::write(&script, READER).context("the key reader script")?;
    let result = cycle(view, &script, cx).await;
    let mine: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| mine.into_iter().for_each(|id| v.close_tab(id, cx)));
    std::fs::remove_file(&script).ok();
    result
}

async fn cycle(view: &Entity<FleetView>, script: &std::path::Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    // the earlier steps' tabs go, so every tab cycled through is one of these three
    let before: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| before.into_iter().for_each(|id| v.close_tab(id, cx)));
    until(view, cx, WITHIN, "the earlier tabs closed", |v| v.tabs.is_empty().then_some(())).await?;

    let args = ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"].map(String::from).into_iter();
    let args: Vec<String> = args.chain([script.display().to_string()]).collect();
    let cwd = std::env::temp_dir();
    let (a, b) = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            let saved = std::mem::replace(&mut v.shell_args, args);
            let a = v.open_shell_in(cwd.clone(), "keys A".into(), window, cx);
            let b = v.open_shell_in(cwd, "keys B".into(), window, cx);
            v.shell_args = saved;
            (a, b)
        })
    })?;
    let work = view.update(cx, |v, cx| v.open_work(WORKSPACE, cx));
    let order: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    if order != [a, b, work] {
        bail!("expected tabs [{a}, {b}, {work}], found {order:?}");
    }
    let (ta, tb) = (reader(view, a, cx).await?, reader(view, b, cx).await?);

    click(view, &format!("tab-{a}"), cx).await?;
    let held = cx.update(|window, cx| view.read(cx).has_keys(window))?;
    if !held || view.read_with(cx, |v, _| v.active) != Some(a) {
        bail!("tab {a} clicked: active {:?}, the view holds the keys: {held}", view.read_with(cx, |v, _| v.active));
    }

    let (ctrl, ctrl_shift) = (Modifiers::control(), Modifiers { control: true, shift: true, ..Default::default() });
    // every chord is pressed at least once while a reader is the active tab
    let steps = [
        ("tab", ctrl, b),
        ("tab", ctrl, work),
        ("tab", ctrl, a),
        ("tab", ctrl_shift, work),
        ("tab", ctrl_shift, b),
        ("pagedown", ctrl, work),
        ("pagedown", ctrl, a),
        ("pageup", ctrl, work),
        ("pageup", ctrl, b),
        ("pageup", ctrl, a),
    ];
    for (i, (key, modifiers, expected)) in steps.into_iter().enumerate() {
        let from = view.read_with(cx, |v, _| v.active);
        press(cx, key, modifiers)?;
        let now = view.read_with(cx, |v, _| v.active);
        if now != Some(expected) {
            bail!("press {} ({}{key}) on tab {from:?}: active is {now:?}, expected {expected}", i + 1, chord(modifiers));
        }
    }

    // after the chords, on the same PTY: what does arrive is all that arrived
    press(cx, "tab", Modifiers::default())?;
    press(cx, "tab", Modifiers::shift())?;
    let got_a = keys(&ta, 2, cx).await.context("reader A")?;
    click(view, &format!("tab-{b}"), cx).await?;
    press(cx, "tab", Modifiers::default())?;
    let got_b = keys(&tb, 1, cx).await.context("reader B")?;
    // ConsoleModifiers: Alt = 1, Shift = 2, Control = 4
    if got_a != ["fm-key Tab 0", "fm-key Tab 2"] || got_b != ["fm-key Tab 0"] {
        bail!("the readers got A {got_a:?} and B {got_b:?}; expected only Tab, Shift+Tab on A and Tab on B");
    }
    crate::selftest_manage::shot("m5-17-tab-hotkeys", cx).await;
    Ok(format!(
        "{} chords cycled [{a}, {b}, {work}] both ways with wraps; readers saw none of them, A got {got_a:?}, B got {got_b:?}",
        steps.len()
    ))
}

/// Three tabs; the second dragged out of the strip, then before the first, then the first to the end.
pub(crate) async fn reorder(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| before.into_iter().for_each(|id| v.close_tab(id, cx)));
    until(view, cx, WITHIN, "the earlier tabs closed", |v| v.tabs.is_empty().then_some(())).await?;
    let cwd = std::env::temp_dir();
    let (a, b) = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            let saved = std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]);
            let a = v.open_shell_in(cwd.clone(), "drag A".into(), window, cx);
            let b = v.open_shell_in(cwd, "drag B".into(), window, cx);
            v.shell_args = saved;
            (a, b)
        })
    })?;
    let work = view.update(cx, |v, cx| v.open_work(WORKSPACE, cx));
    let result = drags(view, [a, b, work], cx).await;
    let mine: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| mine.into_iter().for_each(|id| v.close_tab(id, cx)));
    result
}

async fn drags(view: &Entity<FleetView>, [a, b, work]: [u64; 3], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let state = |cx: &mut AsyncWindowContext| view.read_with(cx, |v, _| (v.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), v.active));
    click(view, &format!("tab-{a}"), cx).await?;
    if state(cx) != (vec![a, b, work], Some(a)) {
        bail!("before the drags: (order, active) {:?}, expected ([{a}, {b}, {work}], {a})", state(cx));
    }

    let strip = scrolled_into_view(view, &format!("tab-{b}"), cx).await?;
    let outside = point(strip.center().x, strip.bottom() + px(200.));
    drag(view, b, outside, None, cx).await?;
    if state(cx) != (vec![a, b, work], Some(a)) {
        bail!("tab {b} dropped below the strip: (order, active) {:?}, expected it unchanged", state(cx));
    }

    let onto_a = scrolled_into_view(view, &format!("tab-{a}"), cx).await?.center();
    drag(view, b, onto_a, Some("m5-53-tab-drag"), cx).await?;
    if state(cx) != (vec![b, a, work], Some(a)) {
        bail!("tab {b} dropped on tab {a}: (order, active) {:?}, expected ([{b}, {a}, {work}], {a})", state(cx));
    }

    let (ctrl, ctrl_shift) = (Modifiers::control(), Modifiers { control: true, shift: true, ..Default::default() });
    let steps = [(ctrl, work), (ctrl, b), (ctrl, a), (ctrl_shift, b), (ctrl_shift, work), (ctrl_shift, a)];
    for (i, (modifiers, expected)) in steps.into_iter().enumerate() {
        press(cx, "tab", modifiers)?;
        let now = view.read_with(cx, |v, _| v.active);
        if now != Some(expected) {
            bail!("press {} ({}tab) in [{b}, {a}, {work}]: active is {now:?}, expected {expected}", i + 1, chord(modifiers));
        }
    }

    let onto_work = scrolled_into_view(view, &format!("tab-{work}"), cx).await?.center();
    drag(view, b, onto_work, None, cx).await?;
    if state(cx) != (vec![a, work, b], Some(a)) {
        bail!("tab {b} dropped on the last tab {work}: (order, active) {:?}, expected ([{a}, {work}, {b}], {a})", state(cx));
    }
    press(cx, "tab", ctrl)?;
    if view.read_with(cx, |v, _| v.active) != Some(work) {
        bail!("Ctrl+Tab from {a} in [{a}, {work}, {b}]: active is {:?}, expected {work}", view.read_with(cx, |v, _| v.active));
    }
    Ok(format!(
        "dropped below the strip: unchanged; {b} onto {a}: [{b}, {a}, {work}], {} chords followed it; {b} onto {work}: [{a}, {work}, {b}]; {a} stayed active",
        steps.len()
    ))
}

/// Left button down on `tab-<id>`, moved in steps to `to` and let go there, as a hand drags it.
async fn drag(view: &Entity<FleetView>, id: u64, to: Point<Pixels>, shot: Option<&str>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let from = scrolled_into_view(view, &format!("tab-{id}"), cx).await?.center();
    let none = Modifiers::default();
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers: none }), cx);
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: from, modifiers: none, click_count: 1, first_mouse: false }), cx);
        for step in 1..=8 {
            let t = step as f32 / 8.;
            let position = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: none }), cx);
        }
    })?;
    sleep(cx, Duration::from_millis(100)).await;
    if let Some(name) = shot {
        crate::selftest_manage::shot(name, cx).await;
    }
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: to, modifiers: none, click_count: 1 }), cx);
    })?;
    sleep(cx, Duration::from_millis(100)).await;
    Ok(())
}

fn chord(m: Modifiers) -> &'static str {
    if m.shift { "ctrl-shift-" } else { "ctrl-" }
}

pub(crate) fn press(cx: &mut AsyncWindowContext, key: &str, modifiers: Modifiers) -> anyhow::Result<()> {
    let keystroke = Keystroke { modifiers, key: key.into(), key_char: None };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx);
    })?;
    Ok(())
}

pub(crate) async fn reader(view: &Entity<FleetView>, tab: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<Entity<Terminal>> {
    let terminal = wait_for_tab_terminal(view, tab, WITHIN, cx).await?;
    let started = Instant::now();
    while !terminal.read_with(cx, |t, _| t.get_content()).contains(READY) {
        if started.elapsed() > WITHIN {
            bail!("tab {tab}: the key reader never printed {READY:?}: {:?}", terminal.read_with(cx, |t, _| t.get_content()));
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    Ok(terminal)
}

pub(crate) async fn keys(terminal: &Entity<Terminal>, count: usize, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<String>> {
    let lines = |cx: &mut AsyncWindowContext| -> Vec<String> {
        let text = terminal.read_with(cx, |t, _| t.get_content());
        text.lines().map(str::trim).filter(|l| l.starts_with(KEY)).map(String::from).collect()
    };
    let started = Instant::now();
    while lines(cx).len() < count {
        if started.elapsed() > WITHIN {
            return Err(anyhow!("{count} key line(s) not printed within {} s: {:?}", WITHIN.as_secs(), lines(cx)));
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    sleep(cx, Duration::from_millis(500)).await;
    Ok(lines(cx))
}

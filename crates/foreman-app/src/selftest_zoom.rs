//! `--selftest` step 7q (m5/41): one terminal font size for every tab, kept in `settings.toml`. Two `+`
//! tabs; Ctrl+= twice in the second; both grids are then laid out at the new size (the first once it is
//! shown again), and after the debounce `settings.toml` holds it. Three Ctrl+wheel notches leave the file
//! unwritten until the spin stops, then hold the sum. Settings → Appearance shows the size, its + applies to
//! the open terminal, and a Ctrl+- in the terminal shows on the page. Ctrl+0 is 14 again, saved as unset.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use foreman_core::Settings;
use foreman_term::Terminal;
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseMoveEvent, Pixels, PlatformInput, ScrollDelta, ScrollEvent, point, px};
use foreman_ui::term_element;

use crate::{
    selftest::{key, sleep, wait_for_tab_terminal},
    settings_window::Page,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, home, cx).await;
    view.update(cx, |v, _| v.shell_args = saved);
    // whatever failed, the next run starts at 14
    view.update(cx, |v, cx| v.set_term_font_size(term_element::FONT_SIZE, cx));
    close_all(view, cx);
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    close_all(view, cx);
    let start = view.read_with(cx, |v, _| v.term_font_size);
    if start != term_element::FONT_SIZE {
        bail!("the terminal font size starts at {start:?}, not 14");
    }
    let first = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let first_term = wait_for_tab_terminal(view, first, WITHIN, cx).await.context("first tab")?;
    let second = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let second_term = wait_for_tab_terminal(view, second, WITHIN, cx).await.context("second tab")?;
    laid_out_at(view, second, &second_term, start, cx).await?;

    let ctrl = Modifiers::control();
    key(cx, "=", ctrl)?;
    key(cx, "=", ctrl)?;
    let zoomed = view.read_with(cx, |v, _| v.term_font_size);
    if zoomed != start + px(2.) {
        bail!("Ctrl+= twice in tab {second}: {start:?} → {zoomed:?}");
    }
    let second_ms = laid_out_at(view, second, &second_term, zoomed, cx).await.context("the zoomed tab")?;
    view.update(cx, |v, cx| v.activate_tab(first, cx));
    let first_ms = laid_out_at(view, first, &first_term, zoomed, cx).await.context("the other tab")?;
    let saved_ms = saved_as(view, home, f32::from(zoomed) as u32, cx).await?;

    // a wheel spin: the file waits for the last notch
    view.update(cx, |v, cx| v.activate_tab(second, cx));
    laid_out_at(view, second, &second_term, zoomed, cx).await?;
    let grid = second_term.read_with(cx, |t, _| t.last_content().terminal_bounds.bounds);
    for _ in 0..3 {
        cx.update(|window, cx| {
            let position = grid.center();
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
            let event = ScrollEvent { position, delta: ScrollDelta::Lines(point(0., 3.)), modifiers: ctrl, ..Default::default() };
            window.dispatch_event(PlatformInput::Scroll(event), cx);
        })?;
        sleep(cx, Duration::from_millis(60)).await;
    }
    let spun = view.read_with(cx, |v, _| v.term_font_size);
    if spun != zoomed + px(3.) {
        bail!("3 Ctrl+wheel notches up: {zoomed:?} → {spun:?}");
    }
    let during = Settings::load(home)?.terminal_font_size();
    if during != f32::from(zoomed) as u32 {
        bail!("settings.toml held {during} right after the spin; the write is not debounced");
    }
    let spun_ms = saved_as(view, home, f32::from(spun) as u32, cx).await?;
    view.update(cx, |v, cx| v.activate_tab(first, cx));
    laid_out_at(view, first, &first_term, spun, cx).await.context("the other tab after the wheel")?;

    let settings = settings_page(view, home, first, &first_term, spun, cx).await.context("Settings → Appearance")?;

    key(cx, "0", ctrl)?;
    let reset = view.read_with(cx, |v, _| v.term_font_size);
    if reset != term_element::FONT_SIZE {
        bail!("Ctrl+0: {spun:?} → {reset:?}, not 14");
    }
    laid_out_at(view, first, &first_term, reset, cx).await?;
    saved_as(view, home, 14, cx).await?;
    if std::fs::read_to_string(Settings::path(home))?.contains("terminal_font_size") {
        bail!("14 is the default and is saved as unset, but settings.toml names it");
    }
    Ok(format!(
        "Ctrl+=×2 in tab {second}: {start:?} → {zoomed:?}, its grid in {second_ms} ms, tab {first}'s on showing in {first_ms} ms, \
         settings.toml {} after {saved_ms} ms; Ctrl+wheel ×3 → {spun:?}, file unchanged during the spin, saved after {spun_ms} ms; \
         {settings}; Ctrl+0 → 14, saved as unset",
        f32::from(zoomed)
    ))
}

/// Settings → Appearance shows `size`; its + applies to the open terminal and is saved; a Ctrl+- in the
/// terminal shows on the page.
async fn settings_page(
    view: &Entity<FleetView>,
    home: &Path,
    tab: u64,
    terminal: &Entity<Terminal>,
    size: Pixels,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    view.update(cx, |v, cx| v.open_settings_at(Some(Page::Appearance), cx));
    let mut handle = None;
    for _ in 0..100 {
        handle = view.read_with(cx, |v, _| v.settings_window).filter(|h| h.read_with(cx, |_, _| ()).is_ok());
        if handle.is_some() {
            break;
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let handle = handle.ok_or_else(|| anyhow!("Settings did not open within 5 s"))?;
    let shown = async |want: Pixels, cx: &mut AsyncWindowContext| -> anyhow::Result<()> {
        let started = Instant::now();
        loop {
            let shown = handle.read_with(cx, |s, _| s.shown_font_size()).map_err(|e| anyhow!("settings window: {e}"))?;
            if shown == Some(want) {
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(3) {
                bail!("the page shows {shown:?}, expected {want:?}");
            }
            sleep(cx, Duration::from_millis(30)).await;
        }
    };
    let result: anyhow::Result<String> = async {
        shown(size, cx).await?;
        crate::selftest_manage::shot_window("m5-41-settings-font-size", Some("Foreman Settings"), cx).await;
        handle.update(cx, |s, _, cx| s.set_term_font_size(size + px(1.), cx)).map_err(|e| anyhow!("settings window: {e}"))?;
        let plus = size + px(1.);
        let now = view.read_with(cx, |v, _| v.term_font_size);
        if now != plus {
            bail!("+ on the page: {size:?} → {now:?}");
        }
        shown(plus, cx).await?;
        laid_out_at(view, tab, terminal, plus, cx).await.context("the open terminal after +")?;
        saved_as(view, home, f32::from(plus) as u32, cx).await?;
        key(cx, "-", Modifiers::control())?;
        shown(size, cx).await.context("after Ctrl+- in the terminal")?;
        saved_as(view, home, f32::from(size) as u32, cx).await?;
        Ok(format!("Appearance showed {size:?}, + → {plus:?} in the open tab and settings.toml, Ctrl+- in the terminal → {size:?} on the page"))
    }
    .await;
    view.update(cx, |v, cx| v.close_settings(cx));
    sleep(cx, Duration::from_millis(300)).await;
    result
}

/// Waits until `tab`'s grid is painted with the cell metrics of `size`.
async fn laid_out_at(view: &Entity<FleetView>, tab: u64, terminal: &Entity<Terminal>, size: Pixels, cx: &mut AsyncWindowContext) -> anyhow::Result<u128> {
    let font = view.read_with(cx, |v, _| term_element::TermFont { size, ..v.term_font() });
    laid_out_with(view, tab, terminal, &font, cx).await
}

/// Waits until `tab`'s grid is painted with the cell metrics of `font`.
pub(crate) async fn laid_out_with(view: &Entity<FleetView>, tab: u64, terminal: &Entity<Terminal>, font: &term_element::TermFont, cx: &mut AsyncWindowContext) -> anyhow::Result<u128> {
    let started = Instant::now();
    let want = cx.update(|_, cx| term_element::cell_metrics(font, cx))?;
    loop {
        let bounds = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds);
        if (bounds.line_height, bounds.cell_width) == want {
            return Ok(started.elapsed().as_millis());
        }
        if started.elapsed() > Duration::from_secs(3) {
            let active = view.read_with(cx, |v, _| v.active);
            bail!(
                "tab {tab} (active {active:?}) is laid out at line height {:?} / cell {:?}, expected {want:?} for {font:?}",
                bounds.line_height,
                bounds.cell_width
            );
        }
        sleep(cx, Duration::from_millis(30)).await;
    }
}

async fn saved_as(view: &Entity<FleetView>, home: &Path, size: u32, cx: &mut AsyncWindowContext) -> anyhow::Result<u128> {
    let started = Instant::now();
    loop {
        let pending = view.read_with(cx, |v, _| v.font_save_pending());
        let held = Settings::load(home)?.terminal_font_size();
        if !pending && held == size {
            return Ok(started.elapsed().as_millis());
        }
        if started.elapsed() > Duration::from_secs(3) {
            bail!("settings.toml holds terminal_font_size {held}, expected {size} (write pending: {pending})");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn close_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) {
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| ids.into_iter().for_each(|id| v.close_tab(id, cx)));
}

//! `--selftest` step 7m (m5/31): the theme. The app starts in VS Code Dark Modern with its terminal colours;
//! Settings → Appearance switches to Foreman Dark with Campbell's ANSI colours at once (the theme every
//! window paints with, the terminals' palette, `settings.toml`), and back. Shots with `FOREMAN_SHOTS`.

use std::path::Path;

use anyhow::{anyhow, bail};
use foreman_core::Settings;
use foreman_term::terminal_settings::{CAMPBELL, TerminalConfig};
use foreman_ui::themes::{DARK_MODERN, FOREMAN_DARK, TerminalColors, hex};
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::sleep,
    selftest_manage::{shot, shot_window},
    settings_window::Page,
    view::FleetView,
};

/// (theme name, panel background, ANSI 8) as painted now.
fn painted(cx: &mut AsyncWindowContext) -> anyhow::Result<(String, String, String)> {
    cx.update(|_, cx| {
        let theme = theme::ActiveTheme::theme(cx);
        (theme.name.to_string(), hex(theme.colors().panel_background), hex(TerminalConfig::get(cx).palette.ansi[8]))
    })
    .map_err(|e| anyhow!("{e}"))
}

pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let start = painted(cx)?;
    if start != (DARK_MODERN.into(), "#181818".into(), "#666666".into()) {
        bail!("the app started in {start:?}, not VS Code Dark Modern (panel #181818, ANSI 8 #666666)");
    }
    shot("m5-31-tree-tabs", cx).await;

    view.update(cx, |v, cx| v.open_settings_at(Some(Page::Appearance), cx));
    let handle = {
        let mut handle = None;
        for _ in 0..100 {
            handle = view.read_with(cx, |v, _| v.settings_window).filter(|h| h.read_with(cx, |_, _| ()).is_ok());
            if handle.is_some() {
                break;
            }
            sleep(cx, std::time::Duration::from_millis(50)).await;
        }
        handle.ok_or_else(|| anyhow!("Settings did not open within 5 s"))?
    };
    let page = handle.read_with(cx, |s, _| s.page()).map_err(|e| anyhow!("settings window: {e}"))?;
    if page != Page::Appearance {
        bail!("open_settings_at(Appearance) left the settings window on {page:?}");
    }
    shot_window("m5-31-settings-appearance", Some("Foreman Settings"), cx).await;

    let switch = |name: &str, terminal: TerminalColors, cx: &mut AsyncWindowContext| {
        handle
            .update(cx, |s, _, cx| s.set_theme(name.to_string(), terminal, cx))
            .map_err(|e| anyhow!("settings window: {e}"))
    };
    switch(FOREMAN_DARK, TerminalColors::Campbell, cx)?;
    let dark = painted(cx)?;
    let campbell_8 = hex(gpui::rgb(CAMPBELL[8]).into());
    if dark.0 != FOREMAN_DARK || dark.2 != campbell_8 || dark.1 == start.1 {
        bail!("after choosing Foreman Dark and Campbell the app paints {dark:?}");
    }
    let saved = Settings::load(home)?;
    if saved.theme.as_deref() != Some(FOREMAN_DARK) || saved.terminal_colors.as_deref() != Some("campbell") {
        bail!("settings.toml has theme {:?}, terminal_colors {:?}", saved.theme, saved.terminal_colors);
    }
    shot_window("m5-31-settings-foreman-dark", Some("Foreman Settings"), cx).await;
    shot("m5-31-main-foreman-dark", cx).await;

    switch(DARK_MODERN, TerminalColors::Theme, cx)?;
    let back = painted(cx)?;
    if back != start {
        bail!("back to VS Code Dark Modern, the app paints {back:?}, at start {start:?}");
    }
    let saved = Settings::load(home)?;
    if saved.theme.as_deref() != Some(DARK_MODERN) || saved.terminal_colors.is_some() {
        bail!("settings.toml has theme {:?}, terminal_colors {:?} after switching back", saved.theme, saved.terminal_colors);
    }
    view.update(cx, |v, cx| v.close_settings(cx));
    sleep(cx, std::time::Duration::from_millis(500)).await;

    Ok(format!(
        "started in {} (panel {}, ANSI 8 {}); Appearance → {FOREMAN_DARK} + Campbell painted panel {}, ANSI 8 {} and saved; back to {DARK_MODERN}",
        start.0, start.1, start.2, dark.1, dark.2
    ))
}

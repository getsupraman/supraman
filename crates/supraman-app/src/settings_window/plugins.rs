//! The settings window's Plugins page: the extra plugin dirs every launch passes.

use std::path::PathBuf;

use gpui::{AnyElement, Context, SharedString, div, prelude::*};
use supraman_core::Settings;
use supraman_ui::settings_page::row;
use theme::ActiveTheme as _;

use super::SettingsView;

impl SettingsView {
    pub(super) fn render_plugins(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let dirs = &self.settings.sessions.extra_plugin_dirs;
        let lines: Vec<AnyElement> = if dirs.is_empty() {
            vec![div().text_sm().text_color(colors.text_muted).child("none").into_any_element()]
        } else {
            dirs.iter()
                .map(|dir| {
                    let missing = self.missing_plugins.contains(dir);
                    let text = format!("{}{}", dir.display(), if missing { "  (no .claude-plugin/plugin.json here)" } else { "" });
                    div()
                        .text_sm()
                        .when(missing, |d| d.text_color(cx.theme().status().warning))
                        .child(SharedString::from(text))
                        .into_any_element()
                })
                .collect()
        };
        vec![row(
            "Extra plugin dirs",
            "Passed as --plugin-dir to every session Supraman launches, after Supraman's own plugin. A plugin \
             installed for your user, such as workman, loads without being listed here. Change them in \
             settings.toml ([sessions] extra_plugin_dirs = [\"…\"]); a running session keeps the plugins it was launched with.",
            div().into_any_element(),
            Some(div().flex().flex_col().gap_1().pt_2().children(lines).into_any_element()),
            cx,
        )]
    }
}

pub(super) fn missing(settings: &Settings) -> Vec<PathBuf> {
    settings.sessions.extra_plugin_dirs.iter().filter(|dir| !dir.join(".claude-plugin").join("plugin.json").is_file()).cloned().collect()
}

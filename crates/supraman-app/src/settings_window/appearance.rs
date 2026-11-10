//! The settings window's Appearance page: theme, terminal colours, font and size, ligatures, the shell of new tabs
//! and the tab chords.

use std::sync::{Arc, OnceLock};

use gpui::{AnyElement, ClickEvent, Context, FontWeight, Pixels, SharedString, div, prelude::*, px};
use supraman_core::shells::ShellSetting;
use supraman_core::{DEFAULT_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE, TabKeys};
use supraman_ui::settings_page::row;
use supraman_ui::themes::{self, TerminalColors};
use supraman_ui::{button, term_element};
use theme::ActiveTheme as _;

use super::{SettingsView, choose_theme};

pub(super) static MONO_FONTS: OnceLock<Arc<Vec<String>>> = OnceLock::new();

impl SettingsView {
    pub(super) fn set_theme(&mut self, name: String, terminal: TerminalColors, cx: &mut Context<Self>) {
        let chosen = name.clone();
        self.change(move |s| choose_theme(s, &chosen, terminal), cx);
        self.theme_picks += 1;
        let (pick, home) = (self.theme_picks, self.home.clone());
        let resolving = cx.background_spawn(async move { themes::resolve(Some(&name), terminal, home.as_deref()) });
        cx.spawn(async move |this, cx| {
            let resolved = resolving.await;
            this.update(cx, |this, cx| {
                if this.theme_picks == pick {
                    this.theme_problem = themes::apply_resolved(cx, resolved).problem;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The fleet window owns and saves the size: one path for Settings, the keys and the wheel.
    pub fn set_term_font_size(&mut self, size: Pixels, cx: &mut Context<Self>) {
        self.fleet.update(cx, |fleet, cx| fleet.set_term_font_size(size, cx)).ok();
        cx.notify();
    }

    #[cfg(feature = "selftest")]
    pub fn shown_font_size(&self) -> Option<Pixels> {
        self.shown_font_size.get()
    }

    pub fn set_term_font(&mut self, family: Option<&str>, ligatures: bool, cx: &mut Context<Self>) {
        let result = self.fleet.update(cx, |fleet, cx| fleet.set_term_font(family, ligatures, cx));
        self.font_error = match result {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some("the main window is closed".into()),
        };
        self.reload(cx);
        cx.notify();
    }

    #[cfg(feature = "selftest")]
    pub fn mono_fonts(&self) -> Option<Arc<Vec<String>>> {
        self.mono_fonts.clone()
    }

    pub(super) fn open_themes_folder(&mut self, cx: &mut Context<Self>) {
        let Some(home) = &self.home else {
            return;
        };
        let dir = themes::themes_dir(home);
        cx.spawn(async move |this, cx| {
            let made = cx.background_spawn({
                let dir = dir.clone();
                async move { std::fs::create_dir_all(&dir) }
            });
            match made.await {
                Ok(()) => cx.update(|cx| supraman_ui::open::open_off_thread(dir, cx).detach()),
                Err(e) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(format!("Cannot create {}: {e}", dir.display()));
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn find_mono_fonts(&self, cx: &mut Context<Self>) {
        let text_system = cx.text_system().clone();
        let installed = self.fleet.upgrade().map(|f| f.read(cx).installed_fonts.clone()).unwrap_or_default();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { installed.iter().filter(|f| term_element::is_monospace(&text_system, f)).cloned().collect::<Vec<_>>() })
                .await;
            let found = MONO_FONTS.get_or_init(|| Arc::new(found)).clone();
            this.update(cx, |this, cx| {
                this.mono_fonts = Some(found);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_appearance(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = self.settings.appearance.theme.as_deref().unwrap_or(themes::DEFAULT_THEME).to_string();
        let terminal = TerminalColors::from_setting(self.settings.appearance.terminal_colors.as_deref());
        let (family, ligatures, installed) = self
            .fleet
            .upgrade()
            .map(|f| {
                let f = f.read(cx);
                (f.term_font_family.clone(), f.term_ligatures, f.installed_fonts.clone())
            })
            .unwrap_or_else(|| (SharedString::default(), self.settings.terminal.ligatures(), Arc::default()));
        vec![
            self.render_theme(&current, terminal, cx),
            self.render_terminal_colors(&current, terminal, cx),
            self.render_font_size(cx),
            self.render_font(&family, ligatures, &installed, cx),
            self.render_ligatures(ligatures, cx),
            self.render_shell(cx),
            self.render_tab_keys(cx),
        ]
    }

    fn render_theme(&self, current: &str, terminal: TerminalColors, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let mut list = div().flex().flex_col().gap_1().pt_2();
        for (ix, entry) in self.catalog.entries.iter().enumerate() {
            let selected = entry.name == current;
            let name = entry.name.clone();
            let from: SharedString = match &entry.file {
                None if entry.name == themes::DEFAULT_THEME => "bundled · default".into(),
                None => "bundled".into(),
                Some(file) => file.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()).into(),
            };
            list = list.child(
                button::choice(("theme", ix), SharedString::from(entry.name.clone()), selected, cx)
                    .child(div().text_xs().text_color(colors.text_muted).child(from))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_theme(name.clone(), terminal, cx))),
            );
        }
        let notes = self
            .theme_problem
            .iter()
            .chain(&self.catalog.errors)
            .map(|e| div().text_xs().text_color(status.warning).child(SharedString::from(e.clone())).into_any_element())
            .collect::<Vec<_>>();
        list = list.children(notes).child(
            div()
                .pt_1()
                .text_xs()
                .text_color(colors.text_muted)
                .child("VS Code Dark Modern: kevcamel/vscode_dark_modern.zed, © 2024 Kevin Camellini, MIT licence (THIRD_PARTY.md)."),
        );
        let folder = button::small("themes-folder", "Open themes folder", cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_themes_folder(cx)));
        row(
            "Theme",
            "The colours of every Supraman window. Zed theme files (*.json) in the themes folder of the \
             Supraman home are listed too; a key a file leaves out keeps One Dark's. Applied at once.",
            folder.into_any_element(),
            Some(list.into_any_element()),
            cx,
        )
    }

    fn render_terminal_colors(&self, current: &str, terminal: TerminalColors, cx: &mut Context<Self>) -> AnyElement {
        let mut terminal_choices = div().flex().gap_1();
        for (ix, (label, value)) in [("Theme", TerminalColors::Theme), ("Campbell", TerminalColors::Campbell)].into_iter().enumerate() {
            let name = current.to_string();
            terminal_choices = terminal_choices.child(
                button::choice(("terminal-colors", ix), label, terminal == value, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_theme(name.clone(), value, cx))),
            );
        }
        row(
            "Terminal colours",
            "The 16 ANSI colours in terminal tabs: the theme's, or Windows Terminal's Campbell. Background, \
             text and cursor stay the theme's; the selection stays white, as in Windows Terminal.",
            terminal_choices.into_any_element(),
            None,
            cx,
        )
    }

    fn render_font_size(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let size = self.fleet.upgrade().map(|f| f.read(cx).term_font_size);
        self.shown_font_size.set(size);
        let font_size = match size {
            Some(size) => div()
                .flex()
                .items_center()
                .gap_1()
                .child(button::small("term-font-dec", "−", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_term_font_size(size - px(1.), cx);
                })))
                .child(
                    div()
                        .id("term-font-size")
                        .w(px(28.))
                        .flex()
                        .justify_center()
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(format!("{}", f32::from(size)))),
                )
                .child(button::small("term-font-inc", "+", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_term_font_size(size + px(1.), cx);
                })))
                .child(button::small("term-font-reset", "Reset", cx).ml_1().on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.set_term_font_size(term_element::FONT_SIZE, cx);
                })))
                .into_any_element(),
            None => div().text_sm().text_color(colors.text_muted).child("the main window is closed").into_any_element(),
        };
        row(
            "Terminal font size",
            format!(
                "The text size in every terminal tab, in pixels ({MIN_TERMINAL_FONT_SIZE} to {MAX_TERMINAL_FONT_SIZE}, \
                 default {DEFAULT_TERMINAL_FONT_SIZE}). Ctrl+= and Ctrl+- or Ctrl+wheel in any terminal change it \
                 too, Ctrl+0 resets it. Applied to open terminals at once and kept in settings.toml \
                 ([terminal] font_size)."
            ),
            font_size,
            None,
            cx,
        )
    }

    fn render_font(&self, family: &SharedString, ligatures: bool, installed: &[String], cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let mut fonts = div().flex().flex_wrap().gap_1().pt_2();
        match &self.mono_fonts {
            Some(mono) => {
                for (ix, name) in mono.iter().enumerate() {
                    let pick = name.clone();
                    fonts = fonts.child(
                        button::choice(("term-font", ix), SharedString::from(name.clone()), *name == *family, cx)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(Some(&pick), ligatures, cx))),
                    );
                }
            }
            None => fonts = fonts.child(div().text_xs().text_color(colors.text_muted).child("finding the monospace fonts…")),
        }
        let font_notes = self
            .settings
            .terminal.missing_font(installed)
            .map(|missing| format!("{missing} (settings.toml) is not installed: {family} is shown."))
            .into_iter()
            .chain(self.font_error.clone())
            .map(|e| div().w_full().text_xs().text_color(status.warning).child(SharedString::from(e)).into_any_element())
            .collect::<Vec<_>>();
        fonts = fonts.children(font_notes);
        let default = supraman_core::default_terminal_font_family(installed);
        let font_reset = button::small("term-font-default", "Default", cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(None, ligatures, cx)));
        row(
            "Terminal font",
            format!(
                "The font of every terminal tab, from the monospace fonts installed. Default: Cascadia Code, else \
                 Cascadia Mono, else Consolas (here {default}). A character the font lacks is drawn from a fallback \
                 font, shrunk to fit its cell. Applied to open terminals at once and kept in settings.toml \
                 ([terminal] font_family)."
            ),
            font_reset.into_any_element(),
            Some(fonts.into_any_element()),
            cx,
        )
    }

    fn render_ligatures(&self, ligatures: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut ligature_choices = div().flex().gap_1();
        for (ix, (label, on)) in [("On", true), ("Off", false)].into_iter().enumerate() {
            let family = self.settings.terminal.font_family.clone();
            ligature_choices = ligature_choices.child(
                button::choice(("ligatures", ix), label, ligatures == on, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(family.as_deref(), on, cx))),
            );
        }
        row(
            "Ligatures",
            "Character pairs such as -> and != drawn as one symbol, when the font has them (Cascadia Code does), \
             each character still in its own cell. Kept in settings.toml ([terminal] ligatures).",
            ligature_choices.into_any_element(),
            None,
            cx,
        )
    }

    /// Applied to the main window once saved.
    pub fn set_tab_keys(&mut self, mode: TabKeys, cx: &mut Context<Self>) {
        self.change_then(
            move |s| s.window.set_tab_keys(mode),
            |this, cx| {
                let mode = this.settings.window.tab_keys();
                this.fleet.update(cx, |fleet, cx| fleet.tab_keys.update(cx, |keys, _| keys.mode = mode)).ok();
            },
            cx,
        );
    }

    pub(super) fn render_tab_keys(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.window.tab_keys();
        let mut choices = div().flex().gap_1();
        for (ix, (label, mode)) in [("Outside terminals", TabKeys::OutsideTerminals), ("Everywhere", TabKeys::Everywhere)].into_iter().enumerate() {
            choices = choices.child(
                button::choice(("tab-keys", ix), label, current == mode, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_tab_keys(mode, cx))),
            );
        }
        row(
            "Ctrl+T and Ctrl+W",
            "Ctrl+T opens a new task tab and Ctrl+W closes the active tab. Outside terminals, the default, leaves both              to the program in a terminal tab; Everywhere takes them from terminals too. Ctrl+Shift+T and Ctrl+Shift+W              work in every tab, Ctrl+Alt+T reopens the last closed tab. Applied at once and kept in settings.toml              ([window] tab_keys).",
            choices.into_any_element(),
            None,
            cx,
        )
    }

    pub fn set_shell(&mut self, shell: ShellSetting, cx: &mut Context<Self>) {
        self.change(move |s| s.terminal.set_shell(shell), cx);
    }

    pub(super) fn render_shell(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let current = self.settings.terminal.shell();
        let (shells, notice) = self
            .fleet
            .upgrade()
            .map(|f| {
                let default = &f.read(cx).shell_default;
                (default.found().cloned(), default.resolved().and_then(|(_, notice)| notice.clone()))
            })
            .unwrap_or_default();
        let mut choices = div().flex().flex_wrap().gap_1().pt_2();
        let mut notes = Vec::new();
        match &shells {
            Some(shells) => {
                let named = shells.list.iter().filter_map(|s| s.setting.clone().map(|setting| (s.label.clone(), setting)));
                let custom = matches!(current, ShellSetting::Custom(_)).then(|| (format!("{current} ([terminal] shell)"), current.clone()));
                for (ix, (label, setting)) in [("Automatic".to_string(), ShellSetting::Auto)].into_iter().chain(named).chain(custom).enumerate() {
                    let selected = setting == current;
                    choices = choices.child(
                        button::choice(("term-shell", ix), SharedString::from(label), selected, cx)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_shell(setting.clone(), cx))),
                    );
                }
                notes.extend(notice);
            }
            None => choices = choices.child(div().text_xs().text_color(colors.text_muted).child("finding this machine's shells…")),
        }
        let notes = notes.into_iter().map(|n| div().w_full().text_xs().text_color(cx.theme().status().warning).child(SharedString::from(n)).into_any_element());
        row(
            "Shell of new tabs",
            "The shell + and Open terminal here start. Automatic is PowerShell 7 (pwsh), else Windows PowerShell. \
             The ⌄ beside + opens any shell of this machine, Visual Studio's developer shells too. Tabs that start or \
             attach claude always run PowerShell. Kept in settings.toml ([terminal] shell).",
            div().into_any_element(),
            Some(choices.children(notes).into_any_element()),
            cx,
        )
    }
}

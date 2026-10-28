//! A terminal tab's own chords, as Windows Terminal's defaults plus René's settings (m5/23): copy, paste,
//! find, font size and scrolling. [`chord`] decides; [`FleetView::terminal_key`] carries it out. The
//! full list, with the mouse, is docs/KEYS.md.

use std::time::Duration;

use foreman_core::Settings;
use foreman_term::{Search, Terminal};
use gpui::{AnyElement, App, Context, Entity, Keystroke, MouseButton, MouseDownEvent, Pixels, SharedString, Subscription, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use super::{FleetView, hit_target};
use crate::text_input::{InputEvent, TextInput};
use foreman_ui::term_element::{FONT_SIZE, TermFont, step_font_size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chord {
    /// Ctrl+C with a selection, Ctrl+Shift+C, Ctrl+Insert.
    Copy,
    /// Ctrl+V, Ctrl+Shift+V, Shift+Insert.
    Paste,
    Find,
    /// +1 / -1 px.
    FontSize(i32),
    FontReset,
    /// Lines: +1 up, -1 down.
    ScrollLines(i32),
    /// Pages: +1 up, -1 down.
    ScrollPage(i32),
    ScrollTop,
    ScrollBottom,
}

/// A zoom is written to `settings.toml` this long after the last step, so a wheel spin writes it once.
pub const FONT_SAVE_AFTER: Duration = Duration::from_millis(500);

/// The chord `k` is in a terminal tab, if any; everything else goes to the program. Ctrl+C is the
/// program's ^C unless text is selected.
pub fn chord(k: &Keystroke, has_selection: bool) -> Option<Chord> {
    let m = &k.modifiers;
    if m.alt || m.platform {
        return None;
    }
    let key = k.key.as_str();
    match (m.control, m.shift) {
        (true, false) => match key {
            "c" if has_selection => Some(Chord::Copy),
            "insert" => Some(Chord::Copy),
            "v" => Some(Chord::Paste),
            "=" | "+" => Some(Chord::FontSize(1)),
            "-" => Some(Chord::FontSize(-1)),
            "0" => Some(Chord::FontReset),
            _ => None,
        },
        (true, true) => match key {
            "c" => Some(Chord::Copy),
            "v" => Some(Chord::Paste),
            "f" => Some(Chord::Find),
            "=" | "+" => Some(Chord::FontSize(1)),
            "up" => Some(Chord::ScrollLines(1)),
            "down" => Some(Chord::ScrollLines(-1)),
            "pageup" => Some(Chord::ScrollPage(1)),
            "pagedown" => Some(Chord::ScrollPage(-1)),
            "home" => Some(Chord::ScrollTop),
            "end" => Some(Chord::ScrollBottom),
            _ => None,
        },
        (false, true) => (key == "insert").then_some(Chord::Paste),
        (false, false) => None,
    }
}

/// A terminal tab's find bar (Ctrl+Shift+F): a one-line [`TextInput`] that searches as it is edited.
/// Enter goes to the next match up, towards older output, Shift+Enter back down, as Windows Terminal's
/// default direction; Esc closes it.
pub struct Find {
    pub input: Entity<TextInput>,
    /// Index into the terminal's `matches` (top to bottom) of the selected one.
    pub at: Option<usize>,
    pub count: usize,
    _events: Subscription,
}

impl Find {
    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }
}

/// The match `step` away from `at` (+1: up, older) among `count`, wrapping; with none selected, the
/// bottom one.
pub fn find_step(at: Option<usize>, count: usize, step: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let at = at.map_or(count as isize, |at| at as isize);
    Some((at - step).rem_euclid(count as isize) as usize)
}

impl FleetView {
    /// A key for the active terminal tab; true when it was one of [`chord`]'s (or went to the find bar)
    /// and must not reach the program.
    pub(super) fn terminal_key(&mut self, tab_id: u64, terminal: &Entity<Terminal>, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let has_selection = terminal.read(cx).last_content().selection.is_some();
        let Some(chord) = chord(k, has_selection) else {
            return false;
        };
        match chord {
            Chord::Copy => terminal.update(cx, |t, _| t.copy(Some(false))),
            Chord::Paste => match cx.read_from_clipboard().and_then(|c| c.text()) {
                Some(text) => terminal.update(cx, |t, _| t.paste(&text)),
                // an image on the clipboard: Ctrl+V goes to the program as ^V
                None => return false,
            },
            Chord::Find => self.open_find(tab_id, window, cx),
            Chord::FontSize(delta) => self.set_term_font_size(step_font_size(self.term_font_size, delta), cx),
            Chord::FontReset => self.set_term_font_size(FONT_SIZE, cx),
            Chord::ScrollLines(n) if n > 0 => terminal.update(cx, |t, _| t.scroll_up_by(n as usize)),
            Chord::ScrollLines(n) => terminal.update(cx, |t, _| t.scroll_down_by(n.unsigned_abs() as usize)),
            Chord::ScrollPage(n) if n > 0 => terminal.update(cx, |t, _| t.scroll_page_up()),
            Chord::ScrollPage(_) => terminal.update(cx, |t, _| t.scroll_page_down()),
            Chord::ScrollTop => terminal.update(cx, |t, _| t.scroll_to_top()),
            Chord::ScrollBottom => terminal.update(cx, |t, _| t.scroll_to_bottom()),
        }
        cx.notify();
        true
    }

    /// Ctrl+wheel over a terminal: one font step per notch.
    pub(super) fn wheel_font_size(&mut self, up: bool, cx: &mut Context<Self>) {
        self.set_term_font_size(step_font_size(self.term_font_size, if up { 1 } else { -1 }), cx);
    }

    /// Every terminal tab's font size, clamped, painted at once and saved [`FONT_SAVE_AFTER`] later.
    pub fn set_term_font_size(&mut self, size: Pixels, cx: &mut Context<Self>) {
        let size = step_font_size(size.round(), 0);
        if size == self.term_font_size {
            return;
        }
        self.term_font_size = size;
        cx.notify();
        let Some(home) = self.home.clone() else {
            return;
        };
        // replacing the task drops (cancels) the write still waiting
        self.font_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FONT_SAVE_AFTER).await;
            let saved = Settings::load(&home).and_then(|mut settings| {
                settings.set_terminal_font_size(f32::from(size) as u32);
                settings.save(&home)
            });
            this.update(cx, |this, cx| {
                this.font_save = None;
                if let Err(e) = saved {
                    this.action_error = Some(format!("Cannot save the terminal font size: {e}"));
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// The debounced write is still waiting.
    pub fn font_save_pending(&self) -> bool {
        self.font_save.is_some()
    }

    pub fn term_font(&self) -> TermFont {
        TermFont { family: self.term_font_family.clone(), size: self.term_font_size, ligatures: self.term_ligatures }
    }

    /// Every terminal tab's font family (`None`: the default) and ligatures, saved and painted at once; the
    /// next paint lays each grid out with the new cell metrics and resizes its pty.
    pub fn set_term_font(&mut self, family: Option<&str>, ligatures: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let mut settings = match &self.home {
            Some(home) => Settings::load(home).map_err(|e| e.to_string())?,
            None => Settings::default(),
        };
        settings.set_terminal_font_family(family);
        settings.set_terminal_ligatures(ligatures);
        if let Some(home) = &self.home {
            settings.save(home).map_err(|e| format!("Cannot save the terminal font: {e}"))?;
        }
        self.term_font_family = settings.terminal_font_family(&self.installed_fonts).into();
        self.term_ligatures = ligatures;
        cx.notify();
        Ok(())
    }

    /// The tab's find bar, opened or brought back, focused with its query selected.
    fn open_find(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab(tab_id).is_some_and(|t| t.find.is_none()) {
            let input = cx.new(|cx| TextInput::new("", None, window, cx));
            let events = cx.subscribe_in(&input, window, move |this: &mut Self, _, event: &InputEvent, window, cx| this.on_find_event(tab_id, event, window, cx));
            if let Some(tab) = self.tab_mut(tab_id) {
                tab.find = Some(Find { input, at: None, count: 0, _events: events });
            }
        }
        if let Some(find) = self.tab(tab_id).and_then(|t| t.find.as_ref()) {
            find.input.update(cx, |input, cx| input.focus_all(window, cx));
        }
    }

    fn on_find_event(&mut self, tab_id: u64, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.tab(tab_id).and_then(|t| t.terminal.clone()) else {
            return;
        };
        match event {
            InputEvent::Edited => self.run_find(tab_id, &terminal, cx),
            InputEvent::Confirm => self.step_find(tab_id, &terminal, 1, cx),
            InputEvent::ConfirmBack => self.step_find(tab_id, &terminal, -1, cx),
            InputEvent::Cancel => {
                self.close_find(tab_id, &terminal, cx);
                self.focus.focus(window, cx);
            }
            InputEvent::Refused(_) => {}
        }
        cx.notify();
    }

    fn run_find(&mut self, tab_id: u64, terminal: &Entity<Terminal>, cx: &mut Context<Self>) {
        let Some(query) = self.tab(tab_id).and_then(|t| t.find.as_ref()).map(|f| f.query(cx)) else {
            return;
        };
        let search = (!query.is_empty()).then(|| Search::literal(&query)).flatten();
        let Some(search) = search else {
            terminal.update(cx, |t, _| t.matches.clear());
            if let Some(find) = self.tab_mut(tab_id).and_then(|t| t.find.as_mut()) {
                (find.at, find.count) = (None, 0);
            }
            return;
        };
        let task = terminal.update(cx, |t, cx| t.find_matches(search, cx));
        let terminal = terminal.clone();
        cx.spawn(async move |this, cx| {
            let matches = task.await;
            this.update(cx, |this, cx| {
                // a later keystroke searched again; its result wins
                if this.tab(tab_id).and_then(|t| t.find.as_ref()).map(|f| f.query(cx)) != Some(query) {
                    return;
                }
                let Some(find) = this.tab_mut(tab_id).and_then(|t| t.find.as_mut()) else {
                    return;
                };
                (find.at, find.count) = (None, matches.len());
                terminal.update(cx, |t, _| t.matches = matches);
                this.step_find(tab_id, &terminal, 1, cx);
            })
            .ok();
        })
        .detach();
    }

    fn step_find(&mut self, tab_id: u64, terminal: &Entity<Terminal>, step: isize, cx: &mut Context<Self>) {
        let Some(find) = self.tab_mut(tab_id).and_then(|t| t.find.as_mut()) else {
            return;
        };
        find.at = find_step(find.at, find.count, step);
        if let Some(at) = find.at {
            terminal.update(cx, |t, _| t.activate_match(at));
        }
        cx.notify();
    }

    fn close_find(&mut self, tab_id: u64, terminal: &Entity<Terminal>, cx: &mut Context<Self>) {
        if let Some(tab) = self.tab_mut(tab_id) {
            tab.find = None;
        }
        terminal.update(cx, |t, _| t.matches.clear());
        cx.notify();
    }

    pub(super) fn find_bar(&self, tab_id: u64, find: &Find, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let status = match (find.at, find.count) {
            (_, 0) if find.input.read(cx).text().is_empty() => String::new(),
            (_, 0) => "no matches".into(),
            (Some(at), count) => format!("{} of {count}", count - at),
            (None, count) => format!("{count}"),
        };
        div()
            .id("find-bar")
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(28.))
            .px_2()
            .bg(colors.toolbar_background)
            .border_b_1()
            .border_color(colors.border)
            .text_sm()
            .child(div().text_color(colors.text_muted).child("Find"))
            .child(div().flex_1().min_w_0().child(find.input.clone()))
            .child(div().text_color(colors.text_muted).child(SharedString::from(status)))
            .child(div().text_color(colors.text_muted).child("Enter ↑  Shift+Enter ↓  Esc"))
            .child(hit_target(&self.hits, "find-bar".into()))
            // a click on the bar beside the input gives it the keys, not the window
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(find) = this.tab(tab_id).and_then(|t| t.find.as_ref()) {
                        find.input.update(cx, |input, cx| input.focus(window, cx));
                    }
                }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn k(key: &str, control: bool, shift: bool) -> Keystroke {
        Keystroke { modifiers: Modifiers { control, shift, ..Default::default() }, key: key.into(), key_char: None }
    }

    #[test]
    fn the_settings_bounds_are_the_painters() {
        use foreman_core::{DEFAULT_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE};
        use foreman_ui::term_element::{MAX_FONT_SIZE, MIN_FONT_SIZE};
        assert_eq!(FONT_SIZE, px(DEFAULT_TERMINAL_FONT_SIZE as f32));
        assert_eq!(MIN_FONT_SIZE, px(MIN_TERMINAL_FONT_SIZE as f32));
        assert_eq!(MAX_FONT_SIZE, px(MAX_TERMINAL_FONT_SIZE as f32));
    }

    #[test]
    fn ctrl_c_copies_only_with_a_selection() {
        assert_eq!(chord(&k("c", true, false), true), Some(Chord::Copy));
        assert_eq!(chord(&k("c", true, false), false), None);
        assert_eq!(chord(&k("c", true, true), false), Some(Chord::Copy));
        assert_eq!(chord(&k("insert", true, false), false), Some(Chord::Copy));
    }

    #[test]
    fn paste_find_font_and_scroll_chords_are_windows_terminals() {
        for (key, ctrl, shift) in [("v", true, false), ("v", true, true), ("insert", false, true)] {
            assert_eq!(chord(&k(key, ctrl, shift), false), Some(Chord::Paste), "{key} {ctrl} {shift}");
        }
        assert_eq!(chord(&k("f", true, true), false), Some(Chord::Find));
        assert_eq!(chord(&k("=", true, false), false), Some(Chord::FontSize(1)));
        assert_eq!(chord(&k("-", true, false), false), Some(Chord::FontSize(-1)));
        assert_eq!(chord(&k("0", true, false), false), Some(Chord::FontReset));
        assert_eq!(chord(&k("up", true, true), false), Some(Chord::ScrollLines(1)));
        assert_eq!(chord(&k("down", true, true), false), Some(Chord::ScrollLines(-1)));
        assert_eq!(chord(&k("pageup", true, true), false), Some(Chord::ScrollPage(1)));
        assert_eq!(chord(&k("pagedown", true, true), false), Some(Chord::ScrollPage(-1)));
        assert_eq!(chord(&k("home", true, true), false), Some(Chord::ScrollTop));
        assert_eq!(chord(&k("end", true, true), false), Some(Chord::ScrollBottom));
    }

    #[test]
    fn the_programs_keys_stay_the_programs() {
        // Ctrl+Tab / Ctrl+PgUp/PgDn are the tab bar's bindings; plain keys, Shift+Enter, Ctrl+letters
        // other than C/V, and AltGr (Ctrl+Alt) reach the program
        for (key, ctrl, shift) in [
            ("tab", true, false),
            ("pageup", true, false),
            ("pagedown", true, false),
            ("enter", false, true),
            ("pageup", false, true),
            ("a", true, false),
            ("d", true, false),
            ("f", true, false),
            ("up", true, false),
        ] {
            assert_eq!(chord(&k(key, ctrl, shift), true), None, "{key} {ctrl} {shift}");
        }
        let altgr = Keystroke { modifiers: Modifiers { control: true, alt: true, ..Default::default() }, key: "c".into(), key_char: None };
        assert_eq!(chord(&altgr, true), None);
    }

    #[test]
    fn find_steps_up_from_the_bottom_and_wraps() {
        assert_eq!(find_step(None, 3, 1), Some(2));
        assert_eq!(find_step(Some(0), 3, 1), Some(2));
        assert_eq!(find_step(Some(0), 3, -1), Some(1));
        assert_eq!(find_step(Some(0), 0, 1), None);
    }
}

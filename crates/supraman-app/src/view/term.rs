//! The full list, with the mouse, is docs/KEYS.md.

use std::time::Duration;

use supraman_core::Settings;
use supraman_term::{Search, Terminal};
use gpui::{AnyElement, App, Context, Entity, Keystroke, MouseButton, MouseDownEvent, Pixels, ScrollDelta, SharedString, Subscription, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use super::{FleetView, hit_target};
use crate::text_input::{InputEvent, TextInput};
use supraman_ui::term_element::{FONT_SIZE, TermFont, step_font_size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chord {
    Copy,
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

pub const FONT_SAVE_AFTER: Duration = Duration::from_millis(500);

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

pub struct Find {
    pub input: Entity<TextInput>,
    pub at: Option<usize>,
    pub count: usize,
    _events: Subscription,
}

impl Find {
    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }
}

/// +1 goes up, to older output; with none selected, the bottom one.
pub fn find_step(at: Option<usize>, count: usize, step: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let at = at.map_or(count as isize, |at| at as isize);
    Some((at - step).rem_euclid(count as isize) as usize)
}

const ZOOM_PIXELS: f32 = 48.;

pub fn zoom_steps(delta: ScrollDelta, carried: &mut f32) -> i32 {
    match delta {
        ScrollDelta::Lines(lines) => {
            *carried = 0.;
            if lines.y > 0. { 1 } else if lines.y < 0. { -1 } else { 0 }
        }
        ScrollDelta::Pixels(pixels) => {
            *carried += f32::from(pixels.y);
            let steps = (*carried / ZOOM_PIXELS).trunc();
            *carried -= steps * ZOOM_PIXELS;
            steps as i32
        }
    }
}

impl FleetView {
    pub(super) fn terminal_key(&mut self, tab_id: u64, terminal: &Entity<Terminal>, k: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let has_selection = terminal.read(cx).last_content().selection.is_some();
        let Some(chord) = chord(k, has_selection) else {
            return false;
        };
        match chord {
            Chord::Copy => terminal.update(cx, |t, _| t.copy(Some(false))),
            Chord::Paste => {
                if !terminal.update(cx, |t, cx| t.paste_clipboard(cx)) {
                    return false;
                }
            }
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

    pub(super) fn wheel_font_size(&mut self, delta: ScrollDelta, cx: &mut Context<Self>) {
        let steps = zoom_steps(delta, &mut self.zoom_carried);
        if steps != 0 {
            self.set_term_font_size(step_font_size(self.term_font_size, steps), cx);
        }
    }

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
        self.font_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FONT_SAVE_AFTER).await;
            let saved = Settings::update(&home, |settings| settings.terminal.set_font_size(f32::from(size) as u32));
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

    pub fn font_save_pending(&self) -> bool {
        self.font_save.is_some()
    }

    pub fn term_font(&self) -> TermFont {
        TermFont { family: self.term_font_family.clone(), size: self.term_font_size, ligatures: self.term_ligatures }
    }

    pub fn set_term_font(&mut self, family: Option<&str>, ligatures: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let edit = |settings: &mut Settings| {
            settings.terminal.set_font_family(family);
            settings.terminal.set_ligatures(ligatures);
        };
        let settings = match &self.home {
            Some(home) => Settings::update(home, edit).map_err(|e| format!("Cannot save the terminal font: {e}"))?.0,
            None => {
                let mut settings = Settings::default();
                edit(&mut settings);
                settings
            }
        };
        self.term_font_family = settings.terminal.font_family(&self.installed_fonts).into();
        self.term_ligatures = ligatures;
        cx.notify();
        Ok(())
    }

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

    /// The OSC 8 URI under the mouse, over the terminal's bottom left corner.
    pub(super) fn link_hint(&self, tab_id: u64, terminal: &Entity<Terminal>, cx: &App) -> Option<AnyElement> {
        let link = terminal.read(cx).hovered_link()?.to_owned();
        let colors = cx.theme().colors();
        let key = format!("terminal-link-{tab_id}");
        Some(
            div()
                .id(SharedString::from(key.clone()))
                .absolute()
                .bottom_1()
                .left_1()
                .max_w(gpui::relative(0.66))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .text_xs()
                .text_color(colors.text_muted)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(link))
                .child(hit_target(&self.hits, key))
                .into_any_element(),
        )
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
    fn ctrl_wheel_steps_once_per_notch_and_a_touchpad_by_its_distance() {
        use gpui::point;
        let mut carried = 0.;
        assert_eq!(zoom_steps(ScrollDelta::Lines(point(0., 3.)), &mut carried), 1);
        assert_eq!(zoom_steps(ScrollDelta::Lines(point(0., -1.)), &mut carried), -1);
        let pixels = |y: f32| ScrollDelta::Pixels(point(px(0.), px(y)));
        let steps: Vec<i32> = (0..10).map(|_| zoom_steps(pixels(12.), &mut carried)).collect();
        assert_eq!(steps.iter().sum::<i32>(), 2, "120 px is two steps, not ten: {steps:?}");
        assert_eq!(zoom_steps(pixels(-30.), &mut carried), 0, "24 px carried, 6 px down: no step");
        assert_eq!(zoom_steps(pixels(-100.), &mut carried), -2);
        assert_eq!(zoom_steps(pixels(202.), &mut carried), 4, "one large swipe, with the 10 px carried down");
        assert_eq!(zoom_steps(ScrollDelta::Lines(point(0., 0.)), &mut carried), 0);
        assert_eq!(carried, 0., "a notch drops what a touchpad left");
    }

    #[test]
    fn the_settings_bounds_are_the_painters() {
        use supraman_core::{DEFAULT_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE};
        use supraman_ui::term_element::{MAX_FONT_SIZE, MIN_FONT_SIZE};
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
        // Ctrl+Tab / Ctrl+PgUp/PgDn are the tab bar's bindings
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

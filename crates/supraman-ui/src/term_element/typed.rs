use gpui::{AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, KeyDownEvent, Keystroke, Modifiers, PlatformInput, Render, TestAppContext, VisualTestContext, Window, div};
use supraman_term::{
    Terminal, TerminalBuilder,
    terminal_settings::{AlternateScroll, CursorShape},
};

use util::paths::PathStyle;

use super::*;

/// A terminal tab as the app wires it: the grid, and a key down handed to [`Terminal::key_down`] first.
struct Tab {
    terminal: Entity<Terminal>,
    focus: FocusHandle,
    blink: Entity<Blink>,
}

impl Render for Tab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = Cursor { focus: self.focus.clone(), blink: self.blink.clone() };
        let font = TermFont { family: "Consolas".into(), size: FONT_SIZE, ligatures: false };
        div()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let shown = this.focus.is_focused(window);
                if this.terminal.update(cx, |t, _| t.key_down(event, shown)) {
                    cx.stop_propagation();
                }
            }))
            .child(terminal_grid(self.terminal.clone(), font, cursor))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Tab>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (tab, cx) = cx.add_window_view(|_, cx| {
        let builder = TerminalBuilder::new_display_only(CursorShape::default(), AlternateScroll::On, None, cx.background_executor(), PathStyle::local());
        let terminal = cx.new(|cx| builder.subscribe(cx));
        Tab { terminal, focus: cx.focus_handle(), blink: cx.new(Blink::new) }
    });
    let focus = tab.read_with(cx, |t, _| t.focus.clone());
    cx.update(|window, cx| {
        window.activate_window();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    written(&tab, cx);
    (tab, cx)
}

fn written(tab: &Entity<Tab>, cx: &mut VisualTestContext) -> Vec<Vec<u8>> {
    cx.run_until_parked();
    let terminal = tab.read_with(cx, |t, _| t.terminal.clone());
    terminal.update(cx, |t, _| t.take_pty_write_log())
}

fn key_down(keystroke: Keystroke, prefer_character_input: bool, cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input }), cx));
}

#[gpui::test]
fn typed_text_reaches_the_pty_through_the_grids_input_handler(cx: &mut TestAppContext) {
    let (tab, cx) = open(cx);
    cx.simulate_input("é");
    assert_eq!(written(&tab, cx), [vec![0xC3, 0xA9]], "é as UTF-8, once");
    cx.simulate_input("a");
    assert_eq!(written(&tab, cx), [b"a".to_vec()]);
}

#[gpui::test]
fn a_dead_key_sends_nothing_and_its_character_arrives_once(cx: &mut TestAppContext) {
    let (tab, cx) = open(cx);
    key_down(Keystroke { modifiers: Modifiers::default(), key: "`".into(), key_char: Some("`".into()) }, true, cx);
    assert!(written(&tab, cx).is_empty(), "the bare accent waits for its character");
    cx.simulate_input("é");
    assert_eq!(written(&tab, cx), [vec![0xC3, 0xA9]]);
}

#[gpui::test]
fn ctrl_c_still_sends_etx(cx: &mut TestAppContext) {
    let (tab, cx) = open(cx);
    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(written(&tab, cx), [vec![0x03]]);
    cx.simulate_keystrokes("shift-left");
    assert_eq!(written(&tab, cx), [b"\x1b[1;2D".to_vec()], "Shift+arrows reach the program unchanged");
}

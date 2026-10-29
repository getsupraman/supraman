// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::alacritty::make_content;
use crate::terminal_settings::{
    AlternateScroll, CursorShape as SettingsCursorShape, TerminalConfig,
};
use crate::{Content, MouseInputMode, Terminal, TerminalBounds, TerminalBuilder};
use gpui::AppContext as _;
use gpui::{
    Context, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point as GpuiPoint, TestAppContext, VisualTestContext, bounds, point, px, size,
};
use std::time::Duration;
use util::paths::PathStyle;

pub(crate) fn init_terminal_test(cx: &mut TestAppContext, output: &[u8]) -> Entity<Terminal> {
    cx.update(|cx| {
        cx.set_global(TerminalConfig::default());
    });

    let terminal = cx.new(|cx| {
        TerminalBuilder::new_display_only(
            SettingsCursorShape::default(),
            AlternateScroll::On,
            None,
            cx.background_executor(),
            PathStyle::local(),
        )
        .subscribe(cx)
    });

    terminal.update(cx, |terminal, cx| {
        terminal.write_output(output, cx);
    });

    cx.run_until_parked();

    terminal.update(cx, |terminal, _cx| {
        let term_lock = terminal.term.lock();
        terminal.last_content = make_content(&term_lock, &terminal.last_content);
        drop(term_lock);

        let terminal_bounds = TerminalBounds::new(
            px(20.0),
            px(10.0),
            bounds(point(px(0.0), px(0.0)), size(px(400.0), px(400.0))),
        );
        terminal.last_content.terminal_bounds = terminal_bounds;
        terminal.events.clear();
        terminal.take_pty_write_log();
    });

    terminal
}

pub(crate) fn init_terminal_test_with_window<'a>(
    cx: &'a mut TestAppContext,
    initial_content: &[u8],
) -> (Entity<Terminal>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(TerminalConfig::default());
    });

    cx.executor().allow_parking();

    let window = cx.add_empty_window();
    let builder = window.update(|_, cx| {
        TerminalBuilder::new_display_only(
            SettingsCursorShape::default(),
            AlternateScroll::On,
            None,
            cx.background_executor(),
            PathStyle::local(),
        )
    });
    let terminal = window.new(|cx| builder.subscribe(cx));

    terminal.update(window, |term, cx| {
        term.write_output(initial_content, cx);
    });

    (terminal, window)
}

pub(crate) fn display_only_terminal(cx: &mut TestAppContext) -> Entity<Terminal> {
    cx.new(|cx| {
        TerminalBuilder::new_display_only(
            SettingsCursorShape::default(),
            AlternateScroll::On,
            None,
            cx.background_executor(),
            PathStyle::local(),
        )
        .subscribe(cx)
    })
}

pub(crate) fn raw_content(chunks: &[&[u8]], cx: &mut TestAppContext) -> Content {
    let terminal = display_only_terminal(cx);
    for chunk in chunks {
        terminal.update(cx, |terminal, cx| terminal.write_raw_output(chunk, cx));
    }
    terminal.read_with(cx, |terminal, _| {
        let term = terminal.term.lock_unfair();
        make_content(&term, &terminal.last_content)
    })
}

/// Polls the terminal content until `expected` appears, or panics after ~1s.
/// The PTY IO thread writes into the terminal grid independently of the
/// GPUI executor, so we need a real-time polling loop to synchronize.
pub(crate) async fn assert_content_eventually(
    terminal: &Entity<Terminal>,
    expected: &str,
    cx: &mut TestAppContext,
) {
    let mut content = String::new();
    for _ in 0..100 {
        content = terminal.update(cx, |term, _| term.get_content());
        if content.contains(expected) {
            return;
        }
        cx.background_executor
            .timer(Duration::from_millis(10))
            .await;
    }
    panic!("Expected terminal content to contain {expected:?}, got: {content}");
}

pub(crate) fn left_mouse_down_at(
    terminal: &mut Terminal,
    position: GpuiPoint<Pixels>,
    cx: &mut Context<Terminal>,
) {
    let mouse_down = MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: true,
    };
    terminal.mouse_down(&mouse_down, MouseInputMode::ReportToTerminal, cx);
}

pub(crate) fn left_mouse_up_at(
    terminal: &mut Terminal,
    position: GpuiPoint<Pixels>,
    cx: &mut Context<Terminal>,
) {
    let mouse_up = MouseUpEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::none(),
        click_count: 1,
    };
    terminal.mouse_up(&mouse_up, MouseInputMode::ReportToTerminal, cx);
}

pub(crate) fn left_mouse_drag_to(
    terminal: &mut Terminal,
    position: GpuiPoint<Pixels>,
    cx: &mut Context<Terminal>,
) {
    let region = terminal.last_content.terminal_bounds.bounds;
    let drag_event = MouseMoveEvent {
        position,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::none(),
    };
    terminal.mouse_drag(&drag_event, region, MouseInputMode::ReportToTerminal, cx);
}

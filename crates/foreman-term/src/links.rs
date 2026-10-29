// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::alacritty::{HyperlinkMatch, find_from_terminal_point};
use crate::events::InternalEvent;
use crate::mouse::SelectionPhase;
use crate::{Event, HoveredWord, Point, Range, Terminal};
use gpui::{Context, Modifiers, Pixels, Point as GpuiPoint, Window, px};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathLikeTarget {
    /// File system path, absolute or relative, existing or not.
    /// Might have line and column number(s) attached as `file.rs:1:23`
    pub maybe_path: String,
    /// Current working directory of the terminal
    pub working_directory: Option<PathBuf>,
}

/// A string inside terminal, potentially useful as a URI that can be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaybeNavigationTarget {
    /// HTTP, git, etc. string determined by the `URL_REGEX` regex.
    Url(String),
    /// File system path, absolute or relative, existing or not.
    /// Might have line and column number(s) attached as `file.rs:1:23`
    PathLike(PathLikeTarget),
}

pub(crate) const FIND_HYPERLINK_THROTTLE_PX: Pixels = px(5.0);

pub(crate) const FIND_HYPERLINK_THROTTLE: Duration = Duration::from_millis(100);

impl Terminal {
    pub(crate) fn process_hyperlink(
        &mut self,
        hyperlink: HyperlinkMatch,
        open: bool,
        history_size: usize,
        cx: &mut Context<Self>,
    ) {
        let HyperlinkMatch {
            text: maybe_url_or_path,
            is_url,
            range,
        } = hyperlink;
        let prev_hovered_word = self.last_content.last_hovered_word.take();
        let match_line = range.start().line;
        let working_directory = self.cwd_at_line(match_line, history_size);

        let target = if is_url {
            MaybeNavigationTarget::Url(maybe_url_or_path.clone())
        } else {
            MaybeNavigationTarget::PathLike(PathLikeTarget {
                maybe_path: maybe_url_or_path.clone(),
                working_directory,
            })
        };

        if open {
            cx.emit(Event::Open(target));
        } else {
            self.update_selected_word(prev_hovered_word, range, maybe_url_or_path, target, cx);
        }
    }

    pub(crate) fn clear_hyperlink(&mut self, cx: &mut Context<Self>) {
        if self.last_content.last_hovered_word.take().is_some() {
            cx.emit(Event::NewNavigationTarget(None));
        }
    }

    pub(crate) fn find_hyperlink_at_point(&mut self, point: Point) -> Option<HyperlinkMatch> {
        let term_lock = self.term.lock();
        find_from_terminal_point(
            &term_lock,
            point,
            &mut self.hyperlink_regex_searches,
            self.path_style,
        )
    }

    pub(crate) fn update_selected_word(
        &mut self,
        prev_word: Option<HoveredWord>,
        word_match: Range,
        word: String,
        navigation_target: MaybeNavigationTarget,
        cx: &mut Context<Self>,
    ) {
        if let Some(prev_word) = prev_word
            && prev_word.word == word
            && prev_word.word_match == word_match
        {
            self.last_content.last_hovered_word = Some(prev_word);
            return;
        }

        self.last_content.last_hovered_word = Some(HoveredWord {
            word,
            word_match,
            id: self.next_link_id(),
        });
        cx.emit(Event::NewNavigationTarget(Some(navigation_target)));
        cx.notify()
    }

    pub(crate) fn next_link_id(&mut self) -> usize {
        let res = self.next_link_id;
        self.next_link_id = self.next_link_id.wrapping_add(1);
        res
    }

    pub(crate) fn schedule_find_hyperlink(
        &mut self,
        modifiers: Modifiers,
        position: GpuiPoint<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.selection_phase == SelectionPhase::Selecting
            || !modifiers.secondary()
            || !self.last_content.terminal_bounds.bounds.contains(&position)
        {
            self.clear_hyperlink(cx);
            return;
        }

        // Throttle hyperlink searches to avoid excessive processing
        let now = Instant::now();
        let throttle = !self
            .last_hyperlink_search_position
            .map_or(true, |last_pos| {
                // Only search if mouse moved significantly or enough time passed
                let distance_moved = ((position.x - last_pos.x).abs()
                    + (position.y - last_pos.y).abs())
                    > FIND_HYPERLINK_THROTTLE_PX;
                let time_elapsed =
                    now.duration_since(self.last_mouse_move_time) > FIND_HYPERLINK_THROTTLE;
                distance_moved || time_elapsed
            });

        #[cfg(test)]
        let throttle = if self.suppress_hyperlink_throttle_once {
            self.suppress_hyperlink_throttle_once = false;
            false
        } else {
            throttle
        };

        if throttle {
            return;
        }

        self.last_mouse_move_time = now;
        self.last_hyperlink_search_position = Some(position);
        self.events.push_back(InternalEvent::FindHyperlink(
            position - self.last_content.terminal_bounds.bounds.origin,
            false,
        ));
        cx.notify();
    }

    pub(crate) fn refresh_hovered_word(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.schedule_find_hyperlink(window.modifiers(), window.mouse_position(), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alacritty::make_content;
    use crate::events::InternalEvent;
    use crate::terminal_settings::TerminalConfig;
    use crate::test_support::*;
    use crate::{
        Event, HoveredWord, IndexedCell, Modes, MouseInputMode, Point, Range, Terminal,
        TerminalBounds,
    };
    use gpui::AppContext as _;
    use gpui::VisualContext as _;
    use gpui::{
        Bounds, Context, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
        MouseUpEvent, Pixels, Point as GpuiPoint, Size, Subscription, TestAppContext,
        VisualTestContext, Window, point, px,
    };
    use itertools::Itertools as _;
    use std::ops::RangeInclusive;
    use util::default;

    fn ctrl_mouse_down_at(
        terminal: &mut Terminal,
        position: GpuiPoint<Pixels>,
        cx: &mut Context<Terminal>,
    ) {
        let mouse_down = MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::secondary_key(),
            click_count: 1,
            first_mouse: true,
        };
        terminal.mouse_down(&mouse_down, MouseInputMode::ReportToTerminal, cx);
    }

    fn ctrl_mouse_drag_to(
        terminal: &mut Terminal,
        position: GpuiPoint<Pixels>,
        cx: &mut Context<Terminal>,
    ) {
        let terminal_bounds = terminal.last_content.terminal_bounds.bounds;
        let drag_event = MouseMoveEvent {
            position,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::secondary_key(),
        };
        terminal.mouse_drag(
            &drag_event,
            terminal_bounds,
            MouseInputMode::ReportToTerminal,
            cx,
        );
    }

    fn ctrl_mouse_up_at(
        terminal: &mut Terminal,
        position: GpuiPoint<Pixels>,
        cx: &mut Context<Terminal>,
    ) {
        let mouse_up = MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::secondary_key(),
            click_count: 1,
        };
        terminal.mouse_up(&mouse_up, MouseInputMode::ReportToTerminal, cx);
    }

    macro_rules! any_event_matches {
        ($terminal:ident, $event:pat) => {
            $terminal.events.iter().any(|event| matches!(event, $event))
        };
    }

    fn opens_after_click(
        output: &[u8],
        modifiers: Modifiers,
        cx: &mut TestAppContext,
    ) -> (Vec<MaybeNavigationTarget>, Option<String>) {
        let (terminal, cx) = init_terminal_test_with_window(cx, output);
        cx.run_until_parked();
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = opened.clone();
        cx.update(|_, cx| {
            cx.subscribe(&terminal, move |_, event: &Event, _| {
                if let Event::Open(target) = event {
                    sink.borrow_mut().push(target.clone());
                }
            })
            .detach()
        });
        cx.update_window_entity(&terminal, |terminal, window, cx| {
            terminal.sync(window, cx);
            terminal.last_content.terminal_bounds = TerminalBounds::new(
                px(20.0),
                px(10.0),
                gpui::bounds(point(px(0.0), px(0.0)), gpui::size(px(400.0), px(400.0))),
            );
            let position = point(px(85.0), px(5.0));
            terminal.mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                },
                MouseInputMode::ReportToTerminal,
                cx,
            );
            terminal.mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                },
                MouseInputMode::ReportToTerminal,
                cx,
            );
            terminal.sync(window, cx);
        });
        cx.run_until_parked();
        let opened = opened.borrow().clone();
        (opened, cx.opened_url())
    }

    const OSC8_LINK: &[u8] =
        b"Visit \x1b]8;;vscode://file/c:/x.exe\x07the link\x1b]8;;\x07 now\r\n";
    const PLAIN_URL: &[u8] = b"Visit https://zed.dev/ for more\r\n";

    #[gpui::test]
    async fn test_plain_click_opens_no_link(cx: &mut TestAppContext) {
        for output in [OSC8_LINK, PLAIN_URL] {
            let (opened, platform) = opens_after_click(output, Modifiers::none(), cx);
            assert!(opened.is_empty(), "a plain click must not open {opened:?}");
            assert_eq!(platform, None, "a plain click reached the platform opener");
        }
    }

    #[gpui::test]
    async fn test_ctrl_click_opens_osc8_link_of_any_scheme(cx: &mut TestAppContext) {
        let (opened, _) = opens_after_click(OSC8_LINK, Modifiers::secondary_key(), cx);
        assert_eq!(
            opened,
            [MaybeNavigationTarget::Url("vscode://file/c:/x.exe".into())]
        );
    }

    #[gpui::test]
    async fn test_ctrl_click_opens_plain_url(cx: &mut TestAppContext) {
        let (opened, _) = opens_after_click(PLAIN_URL, Modifiers::secondary_key(), cx);
        assert_eq!(
            opened,
            [MaybeNavigationTarget::Url("https://zed.dev/".into())]
        );
    }

    #[gpui::test]
    async fn test_ctrl_click_same_position(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"Visit https://zed.dev/ for more\r\n");

        terminal.update(cx, |terminal, cx| {
            let click_position = point(px(80.0), px(10.0));
            ctrl_mouse_down_at(terminal, click_position, cx);
            ctrl_mouse_up_at(terminal, click_position, cx);

            assert!(
                any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, true)),
                "Should have ProcessHyperlink event when ctrl+clicking on same hyperlink position"
            );
        });
    }

    #[gpui::test]
    async fn test_hyperlink_ctrl_click_same_position_in_mouse_mode(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"Visit https://zed.dev/ for more\r\n");

        terminal.update(cx, |terminal, cx| {
        terminal.last_content.mode = Modes::MOUSE_MODE;

        let click_position = point(px(80.0), px(10.0));
        ctrl_mouse_down_at(terminal, click_position, cx);
        ctrl_mouse_up_at(terminal, click_position, cx);

        assert!(
            any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, true)),
            "Should have ProcessHyperlink event when ctrl+clicking on same hyperlink position in mouse mode"
        );
        assert!(
            terminal.take_pty_write_log().is_empty(),
            "a consumed link click must not be reported to the PTY"
        );
    });
    }

    #[gpui::test]
    async fn test_local_selection_activates_hyperlink_in_mouse_mode(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"Visit https://zed.dev/ for more\r\n");

        terminal.update(cx, |terminal, cx| {
            terminal.last_content.mode = Modes::MOUSE_MODE;
            let position = point(px(80.0), px(10.0));
            let mouse_down = MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::secondary_key(),
                click_count: 1,
                first_mouse: true,
            };
            let mouse_up = MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::secondary_key(),
                click_count: 1,
            };

            terminal.mouse_down(&mouse_down, MouseInputMode::LocalSelection, cx);
            terminal.mouse_up(&mouse_up, MouseInputMode::LocalSelection, cx);

            assert!(any_event_matches!(
                terminal,
                InternalEvent::ProcessHyperlink(_, true)
            ));
            assert!(terminal.take_pty_write_log().is_empty());
        });
    }

    #[gpui::test]
    async fn test_hyperlink_ctrl_click_mismatch_in_mouse_mode_consumes_gesture(
        cx: &mut TestAppContext,
    ) {
        let terminal = init_terminal_test(
            cx,
            b"Visit https://zed.dev/ for more\r\nThis is another line\r\n",
        );

        terminal.update(cx, |terminal, cx| {
        terminal.last_content.mode = Modes::MOUSE_MODE;
        terminal.take_pty_write_log();

        let down_position = point(px(80.0), px(10.0));
        let up_position = point(px(10.0), px(30.0));

        ctrl_mouse_down_at(terminal, down_position, cx);
        terminal.mouse_move(
            &MouseMoveEvent {
                position: up_position,
                pressed_button: Some(MouseButton::Left),
                modifiers: Modifiers::secondary_key(),
            },
            MouseInputMode::ReportToTerminal,
            cx,
        );
        ctrl_mouse_up_at(terminal, up_position, cx);

        assert!(
            !any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, _)),
            "Should NOT open a link when press and release land on different hyperlinks"
        );
        let pty_writes = terminal.take_pty_write_log();
        assert!(
            pty_writes.is_empty(),
            "a captured press must consume the whole gesture, but reports leaked to the PTY: {pty_writes:?}"
        );
    });
    }

    #[gpui::test]
    async fn test_plain_click_on_hyperlink_in_mouse_mode_is_reported(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"Visit https://zed.dev/ for more\r\n");

        terminal.update(cx, |terminal, cx| {
            terminal.last_content.mode = Modes::MOUSE_MODE;
            terminal.take_pty_write_log();

            let click_position = point(px(80.0), px(10.0));
            left_mouse_down_at(terminal, click_position, cx);
            left_mouse_up_at(terminal, click_position, cx);

            assert!(
                !any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, _)),
                "a plain click must not open a link"
            );
            let pty_writes = terminal.take_pty_write_log();
            assert_eq!(
                pty_writes.len(),
                2,
                "expected press and release reports, got {pty_writes:?}"
            );
        });
    }

    #[gpui::test]
    async fn test_ctrl_click_on_non_hyperlink_in_mouse_mode_is_reported(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"Visit https://zed.dev/ for more\r\n");

        terminal.update(cx, |terminal, cx| {
            terminal.last_content.mode = Modes::MOUSE_MODE;
            terminal.take_pty_write_log();

            // Past the end of the line: nothing link-like under the cursor.
            let click_position = point(px(370.0), px(10.0));
            ctrl_mouse_down_at(terminal, click_position, cx);
            ctrl_mouse_up_at(terminal, click_position, cx);

            assert!(
                !any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, _)),
                "a secondary click off a link must not open anything"
            );
            let pty_writes = terminal.take_pty_write_log();
            assert_eq!(
                pty_writes.len(),
                2,
                "expected press and release reports, got {pty_writes:?}"
            );
        });
    }

    #[gpui::test]
    async fn test_ctrl_click_in_mouse_mode_forwards_when_setting_disabled(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"Visit https://zed.dev/ for more\r\n");

        cx.update_global(|config: &mut TerminalConfig, _| {
            config.open_links_in_mouse_mode = false;
        });

        terminal.update(cx, |terminal, cx| {
            terminal.last_content.mode = Modes::MOUSE_MODE;

            let click_position = point(px(80.0), px(10.0));
            ctrl_mouse_down_at(terminal, click_position, cx);
            ctrl_mouse_up_at(terminal, click_position, cx);

            assert!(
                !any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, _)),
                "with the setting disabled, ctrl+click must not open links in mouse mode"
            );
            let pty_writes = terminal.take_pty_write_log();
            assert_eq!(
                pty_writes.len(),
                2,
                "expected press and release reports, got {pty_writes:?}"
            );
        });
    }

    #[gpui::test]
    async fn test_hyperlink_ctrl_click_drag_outside_bounds(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(
            cx,
            b"Visit https://zed.dev/ for more\r\nThis is another line\r\n",
        );

        terminal.update(cx, |terminal, cx| {
            let down_position = point(px(80.0), px(10.0));
            let up_position = point(px(10.0), px(50.0));

            ctrl_mouse_down_at(terminal, down_position, cx);
            ctrl_mouse_drag_to(terminal, up_position, cx);
            ctrl_mouse_up_at(terminal, up_position, cx);

            assert!(
                !any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, _)),
                "Should NOT have ProcessHyperlink event when dragging outside the hyperlink"
            );
        });
    }

    #[gpui::test]
    async fn test_ctrl_click_drag_within_bounds(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"Visit https://zed.dev/ for more\r\n");

        terminal.update(cx, |terminal, cx| {
            let down_position = point(px(70.0), px(10.0));
            let up_position = point(px(130.0), px(10.0));

            ctrl_mouse_down_at(terminal, down_position, cx);
            ctrl_mouse_drag_to(terminal, up_position, cx);
            ctrl_mouse_up_at(terminal, up_position, cx);

            assert!(
                any_event_matches!(terminal, InternalEvent::ProcessHyperlink(_, true)),
                "Should have ProcessHyperlink event when dragging within hyperlink bounds"
            );
        });
    }

    impl<T: AsRef<str>> From<(T, RangeInclusive<(i32, usize)>, usize)> for HoveredWord {
        fn from(value: (T, RangeInclusive<(i32, usize)>, usize)) -> Self {
            let match_start = Point::new(value.1.start().0, value.1.start().1);
            let match_end = Point::new(value.1.end().0, value.1.end().1);
            Self {
                word: value.0.as_ref().to_string(),
                word_match: Range::new(match_start, match_end),
                id: value.2,
            }
        }
    }

    trait WithLineAndId {
        fn with_line(&self, line: i32) -> Self;
        fn with_id(&self, id: usize) -> Self;
        fn with_line_and_id(&self, line: i32, id: usize) -> Self;
    }

    impl WithLineAndId for HoveredWord {
        fn with_line(&self, line: i32) -> Self {
            Self::from((
                self.word.clone(),
                (line, self.word_match.start().column)..=(line, self.word_match.end().column),
                self.id,
            ))
        }

        fn with_id(&self, id: usize) -> Self {
            Self {
                word: self.word.clone(),
                word_match: self.word_match,
                id,
            }
        }

        fn with_line_and_id(&self, line: i32, id: usize) -> Self {
            Self::from((
                self.word.clone(),
                (line, self.word_match.start().column)..=(line, self.word_match.end().column),
                id,
            ))
        }
    }

    struct HyperlinkVisualTestContext<'a, 'b> {
        terminal: &'a mut Terminal,
        window: &'a mut Window,
        cx: &'a mut Context<'b, Terminal>,
    }

    impl<'a, 'b> HyperlinkVisualTestContext<'a, 'b> {
        fn new(
            terminal: &'a mut Terminal,
            window: &'a mut Window,
            cx: &'a mut Context<'b, Terminal>,
        ) -> Self {
            Self {
                terminal,
                window,
                cx,
            }
        }

        #[track_caller]
        fn assert_visible_lines_match(
            &self,
            expected_lines: impl IntoIterator<Item = &'static str>,
        ) {
            fn visible_lines(terminal: &Terminal) -> Vec<String> {
                let content = make_content(&terminal.term.lock_unfair(), &terminal.last_content);
                let mut text_lines = Vec::new();
                let linegroups = content.cells.iter().chunk_by(|cell| cell.point.line);
                for (_, line) in linegroups.into_iter() {
                    let mut previous_cell_had_extras = false;
                    let mut text = String::new();
                    for IndexedCell { cell, .. } in line {
                        if cell.is_wide_char_spacer() {
                            continue;
                        }
                        if cell.character() == ' ' && previous_cell_had_extras {
                            previous_cell_had_extras = false;
                            continue;
                        }
                        previous_cell_had_extras =
                            matches!(cell.zerowidth(), Some(chars) if !chars.is_empty());
                        text.push(cell.character());
                    }
                    text_lines.push(text.trim_end().to_string());
                }
                text_lines
            }

            let lines = visible_lines(self.terminal);
            let mut expected_lines = expected_lines.into_iter();
            for (line, text) in lines.into_iter().enumerate() {
                let Some(expected_text) = expected_lines.next() else {
                    // More actual lines than expected lines, ignore
                    return;
                };
                assert_eq!(text, expected_text, "Mismatched text at line {line}");
            }

            assert!(expected_lines.next().is_none(), "Extra expected lines")
        }

        #[track_caller]
        fn assert_display_offset(&self, expected_display_offset: usize) {
            assert_eq!(
                self.terminal.last_content().display_offset,
                expected_display_offset,
                "Mismatched display offset"
            );
        }

        #[track_caller]
        fn assert_hovered_word(&self, expected_hovered_word: Option<&HoveredWord>) {
            assert_eq!(
                self.terminal.last_content().last_hovered_word.as_ref(),
                expected_hovered_word,
                "Mismatched hovered word"
            );
        }

        fn ctrl_mouse_move_to(&mut self, position: GpuiPoint<Pixels>) {
            let modifiers = Modifiers::secondary_key();
            let move_event = MouseMoveEvent {
                position,
                modifiers,
                ..default()
            };
            self.window.simulate_mouse_move(position, self.cx);
            self.unthrottle();
            self.terminal
                .mouse_move(&move_event, MouseInputMode::ReportToTerminal, self.cx);
        }

        fn write_output_lines(&mut self, output: &str, repeat: usize) {
            for _ in 0..repeat {
                self.terminal.write_output(output.as_bytes(), self.cx);
                self.terminal.write_output(b"\n", self.cx);
            }
        }

        fn scroll_up_by_and_sync(&mut self, lines: usize) {
            self.terminal.scroll_up_by(lines);
            self.sync();
        }

        fn set_size_and_sync(&mut self, new_bounds: TerminalBounds) {
            self.terminal.set_size(new_bounds);
            self.sync();
        }

        fn sync(&mut self) {
            self.unthrottle();
            self.terminal.sync(self.window, self.cx);
        }

        fn unthrottle(&mut self) {
            // Suppress hyperlink throttling for testing
            self.terminal.suppress_hyperlink_throttle_once = true;
        }

        fn set_window_secondary_key(&mut self) {
            self.window.set_modifiers(Modifiers::secondary_key());
        }

        fn clear_window_secondary_key(&mut self) {
            self.window.set_modifiers(default());
        }
    }

    struct TestView {
        wakeups: usize,
        notifies: usize,
        _terminal_subscriptions: Vec<Subscription>,
    }

    impl TestView {
        fn new(terminal: &Entity<Terminal>, window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self {
                wakeups: 0,
                notifies: 0,
                _terminal_subscriptions: Self::subscribe_for_terminal_events(terminal, window, cx),
            }
        }

        fn subscribe_for_terminal_events(
            terminal: &Entity<Terminal>,
            window: &mut Window,
            cx: &mut Context<TestView>,
        ) -> Vec<Subscription> {
            let terminal_subscription =
                cx.observe_in(terminal, window, |test_view, terminal, window, cx| {
                    test_view.notifies += 1;
                    cx.update_entity(&terminal, |terminal, cx| {
                        terminal.suppress_hyperlink_throttle_once = true;
                        terminal.sync(window, cx)
                    })
                });
            let terminal_events_subscription = cx.subscribe_in(
                terminal,
                window,
                |test_view, terminal, event, window, cx| match event {
                    Event::Wakeup => {
                        test_view.wakeups += 1;
                        cx.update_entity(terminal, |terminal, cx| {
                            terminal.suppress_hyperlink_throttle_once = true;
                            terminal.sync(window, cx)
                        })
                    }
                    _ => {}
                },
            );
            vec![terminal_subscription, terminal_events_subscription]
        }
    }

    struct Wakeups(usize);
    struct Notifies(usize);
    struct Expected(Wakeups, Notifies);

    #[track_caller]
    fn assert_wakeups_and_notifies(
        Expected(Wakeups(wakeups), Notifies(notifies)): Expected,
        test_view: &Entity<TestView>,
        cx: &mut VisualTestContext,
    ) {
        let (actual_wakeups, actual_notifies) = cx.update_entity(&test_view, |test_view, _| {
            (
                std::mem::take(&mut test_view.wakeups),
                std::mem::take(&mut test_view.notifies),
            )
        });
        assert_eq!(actual_wakeups, wakeups, "Mismatched wakeups");
        assert_eq!(actual_notifies, notifies, "Mismatced notifies");
    }

    struct TestEntities {
        terminal: Entity<Terminal>,
        test_view: Entity<TestView>,
    }

    impl TestEntities {
        fn new(terminal: &Entity<Terminal>, test_view: &Entity<TestView>) -> Self {
            Self {
                terminal: terminal.clone(),
                test_view: test_view.clone(),
            }
        }
    }

    #[track_caller]
    fn update_test_entities(
        TestEntities {
            terminal,
            test_view,
        }: &TestEntities,
        cx: &mut VisualTestContext,
        update: impl FnOnce(&mut HyperlinkVisualTestContext) -> Option<Expected>,
    ) {
        let expected = cx.update_window_entity(terminal, |terminal, window, cx| {
            update(&mut HyperlinkVisualTestContext::new(terminal, window, cx))
        });
        cx.run_until_parked();
        if let Some(expected) = expected {
            assert_wakeups_and_notifies(expected, test_view, cx)
        }
    }

    async fn init_ctrl_hover_hyperlink_test_with_window(
        cx: &mut TestAppContext,
    ) -> (TestEntities, HoveredWord, &mut VisualTestContext) {
        let (terminal, cx) = init_terminal_test_with_window(cx, b"");
        let test_view = cx.new_window_entity(|window, cx| TestView::new(&terminal, window, cx));
        let test_entities = TestEntities::new(&terminal, &test_view);
        let expected_hovered_word: HoveredWord = (ZED_DEV_STR, (0, 6)..=(0, 21), 0).into();
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            // Set initial expected hovered word
            cx.write_output_lines(OUTPUT_ZED_DEV, 1);
            Some(Expected(Wakeups(2), Notifies(0)))
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.assert_display_offset(0);
            cx.assert_visible_lines_match(vec![OUTPUT_ZED_DEV]);
            cx.set_window_secondary_key();
            cx.ctrl_mouse_move_to(ZED_DEV_PT);
            Some(Expected(Wakeups(0), Notifies(2)))
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.assert_hovered_word(Some(&expected_hovered_word));
            None
        });
        (test_entities, expected_hovered_word, cx)
    }

    const OUTPUT_ZED_DEV: &str = "Visit https://zed.dev/ for more";
    const OUTPUT_NONE: &str = "None";
    const ZED_DEV_STR: &str = "https://zed.dev/";
    const ZED_DEV_PT: GpuiPoint<Pixels> = point(px(30.0), px(2.5));

    #[gpui::test]
    async fn test_ctrl_hover_with_changing_content(cx: &mut TestAppContext) {
        let (test_entities, mut expected_hovered_word, cx) =
            init_ctrl_hover_hyperlink_test_with_window(cx).await;
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.write_output_lines(OUTPUT_ZED_DEV, 1);
            Some(Expected(Wakeups(2), Notifies(0)))
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.assert_display_offset(0);
            cx.assert_visible_lines_match(vec![OUTPUT_ZED_DEV, OUTPUT_ZED_DEV]);
            // Existing hovered_word IS reused when viewport is static
            cx.assert_hovered_word(Some(&expected_hovered_word));
            cx.write_output_lines(OUTPUT_ZED_DEV, 8);
            // Explicitly sync here, because the next sync will emit a wake up, which will
            // trigger another sync, and we need to check in-between them.
            cx.sync();
            cx.assert_display_offset(0);
            cx.assert_visible_lines_match(vec![
                OUTPUT_ZED_DEV,
                OUTPUT_ZED_DEV,
                OUTPUT_ZED_DEV,
                OUTPUT_ZED_DEV,
                OUTPUT_ZED_DEV,
            ]);
            // Existing hovered_word IS NOT reused when viewport is changing (total lines changed,
            // but display offset did not have a corresponding change)
            cx.assert_hovered_word(None);
            // NOTE: 16 of these wakeups are from write_output() (2x8). The 17th is the important
            // one we are testing that comes from sync() and results in the hovered word being
            // set again, which we asserted below.
            Some(Expected(Wakeups(17), Notifies(1)))
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.clear_window_secondary_key();
            // ...AND new hovered_word is set if secondary was held.
            expected_hovered_word = expected_hovered_word.with_id(expected_hovered_word.id + 1);
            cx.assert_hovered_word(Some(&expected_hovered_word));
            cx.set_window_secondary_key();
            for _ in 0..5 {
                cx.write_output_lines(OUTPUT_ZED_DEV, 1);
                cx.write_output_lines(OUTPUT_NONE, 1);
            }
            Some(Expected(Wakeups(21), Notifies(1)))
        });
        for _ in 0..5 {
            update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
                // Should not have a hovered word from previous iteration
                cx.assert_hovered_word(None);
                cx.write_output_lines(OUTPUT_ZED_DEV, 1);
                // Explicitly sync here, because the next sync will emit a wake up, which will
                // trigger another sync, and we need to check in-between them.
                cx.sync();
                cx.assert_hovered_word(None);
                None
            });
            update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
                expected_hovered_word = expected_hovered_word.with_id(expected_hovered_word.id + 1);
                cx.assert_hovered_word(Some(&expected_hovered_word));
                cx.write_output_lines(OUTPUT_NONE, 1);
                // Explicitly sync here, because the next sync will emit a wake up, which will
                // trigger another sync, and we need to check in-between them.
                cx.sync();
                cx.assert_hovered_word(None);
                None
            });
        }
        assert_wakeups_and_notifies(
            Expected(Wakeups(30), Notifies(10)),
            &test_entities.test_view,
            cx,
        );
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.scroll_up_by_and_sync(3);
            // Existing hovered word IS NOT reused when scrolling
            cx.assert_hovered_word(None);
            cx.ctrl_mouse_move_to(ZED_DEV_PT);
            Some(Expected(Wakeups(0), Notifies(2)))
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.assert_display_offset(3);
            cx.assert_visible_lines_match(vec![
                OUTPUT_ZED_DEV,
                OUTPUT_NONE,
                OUTPUT_ZED_DEV,
                OUTPUT_NONE,
                OUTPUT_ZED_DEV,
                OUTPUT_NONE,
            ]);
            None
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            expected_hovered_word =
                expected_hovered_word.with_line_and_id(-3, expected_hovered_word.id + 2);
            cx.assert_hovered_word(Some(&expected_hovered_word));
            // Use an odd number of lines so the previous grid coordinate contains
            // OUTPUT_NONE after the scrollback coordinates shift.
            cx.write_output_lines(OUTPUT_ZED_DEV, 1);
            // All wakeups here are from write_output()
            Some(Expected(Wakeups(2), Notifies(0)))
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.assert_display_offset(4);
            cx.assert_visible_lines_match(vec![
                OUTPUT_ZED_DEV,
                OUTPUT_NONE,
                OUTPUT_ZED_DEV,
                OUTPUT_NONE,
                OUTPUT_ZED_DEV,
                OUTPUT_NONE,
            ]);
            // Existing hovered word IS reused (and adjusted) when total lines changed, but
            // visible lines unchanged
            expected_hovered_word = expected_hovered_word.with_line(-4);
            cx.assert_hovered_word(Some(&expected_hovered_word));
            None
        });
    }

    #[gpui::test]
    async fn test_ctrl_hover_with_changing_bounds(cx: &mut TestAppContext) {
        let (test_entities, mut expected_hovered_word, cx) =
            init_ctrl_hover_hyperlink_test_with_window(cx).await;
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.set_size_and_sync(TerminalBounds::new(
                px(5.0),
                px(5.0),
                Bounds {
                    origin: GpuiPoint::default(),
                    size: Size {
                        width: px(161.8),
                        height: px(61.8),
                    },
                },
            ));
            cx.assert_display_offset(0);
            cx.assert_visible_lines_match(vec![OUTPUT_ZED_DEV]);
            // Existing hovered word IS NOT reused when bounds change
            cx.assert_hovered_word(None);
            Some(Expected(Wakeups(1), Notifies(2)))
        });
        update_test_entities(&test_entities, cx, |cx: &mut HyperlinkVisualTestContext| {
            cx.clear_window_secondary_key();
            // ...AND new hovered_word is set if secondary was held.
            expected_hovered_word = expected_hovered_word.with_id(expected_hovered_word.id + 1);
            cx.assert_hovered_word(Some(&expected_hovered_word));
            Some(Expected(Wakeups(0), Notifies(0)))
        });
    }
}

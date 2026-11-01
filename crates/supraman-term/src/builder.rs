// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

#[cfg(test)]
use crate::Content;
#[cfg(not(windows))]
use crate::alacritty::current_child_signal_mask;
use crate::alacritty::{
    RegexSearches, new_term, open_pty, pty_options, pty_term_config, spawn_event_loop,
};
use crate::cwd::CwdHistoryEntry;
use crate::events::TerminalBackendEvent;
use crate::mouse::SelectionPhase;
use crate::process::{PtyResources, TerminalType};
use crate::pty_info::{ProcessIdGetter, PtyProcessInfo};
use crate::terminal_settings::{
    AlternateScroll, CursorShape as SettingsCursorShape, TerminalConfig,
};
use crate::{Terminal, TerminalBounds};
#[cfg(not(windows))]
use anyhow::Context as _;
use anyhow::{Result, bail};
use collections::{HashMap, VecDeque};
use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use futures_lite::future::yield_now;
use gpui::AppContext as _;
#[cfg(test)]
use gpui::BackgroundExecutor;
use gpui::{App, Context, Task, px};
use itertools::Itertools as _;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use util::paths::PathStyle;
use util::shell::Shell;

pub(crate) fn insert_terminal_env(
    env: &mut HashMap<String, String>,
    version: &impl std::fmt::Display,
) {
    env.insert("TERM_PROGRAM".to_string(), "supraman".to_string());
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    env.insert("TERM_PROGRAM_VERSION".to_string(), version.to_string());
    // claude gives synchronized output and strikethrough only to terminals it knows; see LEARNINGS m5/44.9.
    env.insert("CLAUDE_CODE_FORCE_SYNC_OUTPUT".to_string(), "1".to_string());
    env.insert(
        "CLAUDE_CODE_FORCE_STRIKETHROUGH".to_string(),
        "1".to_string(),
    );
    // claude writes a markdown link as OSC 8 only with this set on Windows; see LEARNINGS sched2/11.
    env.insert("FORCE_HYPERLINK".to_string(), "1".to_string());
}

#[derive(Error, Debug)]
pub(crate) struct TerminalError {
    pub directory: Option<PathBuf>,
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    pub title_override: Option<String>,
    pub source: std::io::Error,
}

impl TerminalError {
    fn fmt_directory(&self) -> String {
        self.directory
            .clone()
            .map(|path| {
                match path
                    .into_os_string()
                    .into_string()
                    .map_err(|os_str| format!("<non-utf8 path> {}", os_str.to_string_lossy()))
                {
                    Ok(s) => s,
                    Err(s) => s,
                }
            })
            .unwrap_or_else(|| "<none specified>".to_string())
    }

    fn fmt_shell(&self) -> String {
        if let Some(title_override) = &self.title_override {
            format!(
                "{} {} ({})",
                self.program.as_deref().unwrap_or("<system defined shell>"),
                self.args.as_ref().into_iter().flatten().format(" "),
                title_override
            )
        } else {
            format!(
                "{} {}",
                self.program.as_deref().unwrap_or("<system defined shell>"),
                self.args.as_ref().into_iter().flatten().format(" ")
            )
        }
    }
}

impl Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let dir_string: String = self.fmt_directory();
        let shell = self.fmt_shell();

        write!(
            f,
            "Working directory: {} Shell command: `{}`, IOError: {}",
            dir_string, shell, self.source
        )
    }
}

// https://github.com/alacritty/alacritty/blob/cb3a79dbf6472740daca8440d5166c1d4af5029e/extra/man/alacritty.5.scd?plain=1#L207-L213
pub(crate) const DEFAULT_SCROLL_HISTORY_LINES: usize = 10_000;

pub(crate) const MAX_SCROLL_HISTORY_LINES: usize = 100_000;

pub struct TerminalMode(());

impl TerminalMode {
    pub fn interactive() -> Self {
        Self(())
    }
}

pub struct TerminalBuilder {
    pub(crate) terminal: Terminal,
    events_rx: UnboundedReceiver<TerminalBackendEvent>,
}

impl TerminalBuilder {
    #[cfg(test)]
    pub(crate) fn new_display_only(
        cursor_shape: SettingsCursorShape,
        alternate_scroll: AlternateScroll,
        max_scroll_history_lines: Option<usize>,
        background_executor: &BackgroundExecutor,
        path_style: PathStyle,
    ) -> TerminalBuilder {
        let terminal_bounds = TerminalBounds::default();
        let scrolling_history = max_scroll_history_lines
            .unwrap_or(DEFAULT_SCROLL_HISTORY_LINES)
            .min(MAX_SCROLL_HISTORY_LINES);
        let config = pty_term_config(scrolling_history, cursor_shape);

        let (events_tx, events_rx) = unbounded();
        let term = new_term(&config, terminal_bounds, events_tx, alternate_scroll);

        let terminal = Terminal {
            terminal_type: TerminalType::DisplayOnly,
            term,
            term_config: config,
            output_processor: None,
            events: VecDeque::with_capacity(10),
            last_content: Content {
                terminal_bounds,
                ..Default::default()
            },
            last_mouse: None,
            mouse_down_position: None,
            matches: Vec::new(),
            selection_head: None,
            breadcrumb_text: String::new(),
            scroll_px: px(0.),
            next_link_id: 0,
            selection_phase: SelectionPhase::Ended,
            hyperlink_regex_searches: RegexSearches::default(),
            is_remote_terminal: false,
            last_mouse_move_time: Instant::now(),
            last_hyperlink_search_position: None,
            mouse_down_hyperlink: None,
            hovered_link: None,
            #[cfg(windows)]
            shell_program: None,
            child_exited: None,
            keyboard_input_sent: false,
            cursor_moved_at: Instant::now(),
            cursor_painted_lit: true,
            selection_painted: None,
            hand_painted: false,
            event_loop_task: Task::ready(Ok(())),
            background_executor: background_executor.clone(),
            path_style,
            cwd_history: Vec::new(),
            pending_cwd_boundary: None,
            input_log: Vec::new(),
            suppress_hyperlink_throttle_once: false,
            pty_write_log: Default::default(),
        };

        TerminalBuilder {
            terminal,
            events_rx,
        }
    }

    pub fn new(
        working_directory: Option<PathBuf>,
        _mode: TerminalMode,
        shell: Shell,
        mut env: HashMap<String, String>,
        cursor_shape: SettingsCursorShape,
        alternate_scroll: AlternateScroll,
        max_scroll_history_lines: Option<usize>,
        path_hyperlink_regexes: Vec<String>,
        path_hyperlink_timeout: Duration,
        is_remote_terminal: bool,
        window_id: u64,
        cx: &App,
        activation_script: Vec<String>,
        path_style: PathStyle,
    ) -> Task<Result<TerminalBuilder>> {
        let version = TerminalConfig::get(cx).program_version;
        let background_executor = cx.background_executor().clone();
        #[cfg(not(windows))]
        let child_signal_mask = match current_child_signal_mask()
            .context("failed to capture terminal child signal mask")
        {
            Ok(signal_mask) => Some(signal_mask),
            Err(error) => return Task::ready(Err(error)),
        };
        let fut = async move {
            // Removed so the shell starts SHLVL at 1, as standalone terminals do.
            env.remove("SHLVL");

            if std::env::var("LANG").is_err() {
                env.entry("LANG".to_string())
                    .or_insert_with(|| "en_US.UTF-8".to_string());
            }

            insert_terminal_env(&mut env, &version);

            #[derive(Default)]
            struct ShellParams {
                program: String,
                args: Option<Vec<String>>,
                title_override: Option<String>,
            }

            impl ShellParams {
                fn new(
                    program: String,
                    args: Option<Vec<String>>,
                    title_override: Option<String>,
                ) -> Self {
                    log::debug!("Using {program} as shell");
                    Self {
                        program,
                        args,
                        title_override,
                    }
                }
            }

            let shell_params = match shell.clone() {
                Shell::System => {
                    if cfg!(windows) {
                        Some(ShellParams::new(
                            util::shell::get_windows_system_shell(),
                            None,
                            None,
                        ))
                    } else {
                        None
                    }
                }
                Shell::Program(program) => Some(ShellParams::new(program, None, None)),
                Shell::WithArguments {
                    program,
                    args,
                    title_override,
                } => Some(ShellParams::new(program, Some(args), title_override)),
            };
            let terminal_title_override =
                shell_params.as_ref().and_then(|e| e.title_override.clone());

            #[cfg(windows)]
            let shell_program = shell_params.as_ref().map(|params| {
                use util::ResultExt;

                Self::resolve_path(&params.program)
                    .log_err()
                    .unwrap_or(params.program.clone())
            });

            let shell_kind = shell.shell_kind(cfg!(windows));

            let scrolling_history = max_scroll_history_lines
                .unwrap_or(DEFAULT_SCROLL_HISTORY_LINES)
                .min(MAX_SCROLL_HISTORY_LINES);
            let config = pty_term_config(scrolling_history, cursor_shape);

            let (events_tx, events_rx) = unbounded();
            let term = new_term(
                &config,
                TerminalBounds::default(),
                events_tx.clone(),
                alternate_scroll,
            );

            let terminal_type = {
                let alacritty_shell = shell_params.as_ref().map(|params| {
                    (
                        params.program.clone(),
                        params.args.clone().unwrap_or_default(),
                    )
                });
                let pty_options = pty_options(
                    alacritty_shell,
                    working_directory.clone(),
                    env.clone(),
                    // The foreground thread's signal mask: built on a background thread, the child would block Ctrl-C.
                    #[cfg(not(windows))]
                    child_signal_mask,
                    #[cfg(windows)]
                    shell_kind.tty_escape_args(),
                );

                let pty = match open_pty(&pty_options, TerminalBounds::default(), window_id) {
                    Ok(pty) => pty,
                    Err(error) => {
                        bail!(TerminalError {
                            directory: working_directory,
                            program: shell_params.as_ref().map(|params| params.program.clone()),
                            args: shell_params.as_ref().and_then(|params| params.args.clone()),
                            title_override: terminal_title_override,
                            source: error,
                        });
                    }
                };

                let pty_info = PtyProcessInfo::new(ProcessIdGetter::from(&pty));

                let pty_tx =
                    spawn_event_loop(term.clone(), events_tx, pty, pty_options.drain_on_exit)?;

                TerminalType::Pty {
                    resources: PtyResources::Active(pty_tx),
                    info: Arc::new(pty_info),
                }
            };

            let terminal = Terminal {
                terminal_type,
                term,
                term_config: config,
                #[cfg(test)]
                output_processor: None,
                events: VecDeque::with_capacity(10),
                last_content: Default::default(),
                last_mouse: None,
                mouse_down_position: None,
                matches: Vec::new(),

                selection_head: None,
                breadcrumb_text: String::new(),
                scroll_px: px(0.),
                next_link_id: 0,
                selection_phase: SelectionPhase::Ended,
                hyperlink_regex_searches: RegexSearches::new(
                    &path_hyperlink_regexes,
                    path_hyperlink_timeout,
                ),
                is_remote_terminal,
                last_mouse_move_time: Instant::now(),
                last_hyperlink_search_position: None,
                mouse_down_hyperlink: None,
                hovered_link: None,
                #[cfg(windows)]
                shell_program,
                child_exited: None,
                keyboard_input_sent: false,
                cursor_moved_at: Instant::now(),
                cursor_painted_lit: true,
                selection_painted: None,
                hand_painted: false,
                event_loop_task: Task::ready(Ok(())),
                background_executor,
                path_style,
                cwd_history: if is_remote_terminal {
                    Vec::new()
                } else {
                    working_directory
                        .as_ref()
                        .map(|working_directory| {
                            vec![CwdHistoryEntry {
                                scrollback_position: i32::MIN,
                                working_directory: working_directory.clone(),
                            }]
                        })
                        .unwrap_or_default()
                },
                pending_cwd_boundary: None,
                #[cfg(test)]
                input_log: Vec::new(),
                #[cfg(test)]
                suppress_hyperlink_throttle_once: false,
                #[cfg(test)]
                pty_write_log: Default::default(),
            };

            if !activation_script.is_empty() {
                for activation_script in activation_script {
                    terminal.write_to_pty(activation_script.into_bytes());
                    // `\r\n` would put PowerShell in continuation mode (>>).
                    terminal.write_to_pty(b"\x0d");
                }
                // The shell clears: `terminal.clear()` now would run before the shell has run the activation script.
                terminal.write_to_pty(shell_kind.clear_screen_command().as_bytes());
                terminal.write_to_pty(b"\x0d");
            }

            Ok(TerminalBuilder {
                terminal,
                events_rx,
            })
        };
        cx.background_spawn(fut)
    }

    pub fn subscribe(mut self, cx: &Context<Terminal>) -> Terminal {
        self.terminal.event_loop_task = cx.spawn(async move |terminal, cx| {
            while let Some(event) = self.events_rx.next().await {
                terminal.update(cx, |terminal, cx| {
                    //Process the first event immediately for lowered latency
                    terminal.process_event(event, cx);
                })?;

                'outer: loop {
                    let mut events = Vec::new();

                    #[cfg(test)]
                    let mut timer = cx.background_executor().simulate_random_delay().fuse();
                    #[cfg(not(test))]
                    let mut timer = cx
                        .background_executor()
                        .timer(std::time::Duration::from_millis(4))
                        .fuse();

                    let mut wakeup = false;
                    loop {
                        futures::select_biased! {
                            _ = timer => break,
                            event = self.events_rx.next() => {
                                if let Some(event) = event {
                                    match event {
                                        TerminalBackendEvent::Wakeup => {
                                            wakeup = true;
                                        }
                                        event => events.push(event),
                                    }

                                    if events.len() > 100 {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            },
                        }
                    }

                    if events.is_empty() && !wakeup {
                        yield_now().await;
                        break 'outer;
                    }

                    terminal.update(cx, |this, cx| {
                        if wakeup {
                            this.process_event(TerminalBackendEvent::Wakeup, cx);
                        }

                        for event in events {
                            this.process_event(event, cx);
                        }
                    })?;
                    yield_now().await;
                }
            }
            anyhow::Ok(())
        });
        self.terminal
    }

    #[cfg(windows)]
    fn resolve_path(path: &str) -> Result<String> {
        use windows::Win32::Storage::FileSystem::SearchPathW;
        use windows::core::HSTRING;

        let path = if path.starts_with(r"\\?\") || !path.contains(&['/', '\\']) {
            path.to_string()
        } else {
            r"\\?\".to_string() + path
        };

        let required_length = unsafe { SearchPathW(None, &HSTRING::from(&path), None, None, None) };
        let mut buf = vec![0u16; required_length as usize];
        let size = unsafe { SearchPathW(None, &HSTRING::from(&path), None, Some(&mut buf), None) };

        Ok(String::from_utf16(&buf[..size as usize])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_settings::{AlternateScroll, CursorShape as SettingsCursorShape};
    use crate::test_support::*;
    use crate::{Event, Terminal};
    use collections::HashMap;
    use gpui::{Entity, TestAppContext};
    use parking_lot::Mutex;
    use std::process::ExitStatus;
    use std::sync::Arc;
    use std::time::Duration;

    use util::paths::PathStyle;
    use util::shell::Shell;
    use util::shell_builder::ShellBuilder;

    async fn build_test_terminal(
        cx: &mut TestAppContext,
        command: &str,
        args: &[&str],
    ) -> Entity<Terminal> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (program, args) =
            ShellBuilder::new(&Shell::System, false).build(Some(command.to_owned()), &args);
        let builder = cx
            .update(|cx| {
                TerminalBuilder::new(
                    None,
                    TerminalMode::interactive(),
                    Shell::WithArguments {
                        program,
                        args,
                        title_override: None,
                    },
                    HashMap::default(),
                    SettingsCursorShape::default(),
                    AlternateScroll::On,
                    None,
                    vec![],
                    Duration::ZERO,
                    false,
                    0,
                    cx,
                    vec![],
                    PathStyle::local(),
                )
            })
            .await
            .unwrap();
        cx.new(|cx| builder.subscribe(cx))
    }

    async fn wait_for_exit(terminal: &Entity<Terminal>, cx: &mut TestAppContext) -> ExitStatus {
        for _ in 0..500 {
            if let Some(status) = terminal.read_with(cx, |terminal, _| terminal.child_exited) {
                return status;
            }
            cx.background_executor
                .timer(Duration::from_millis(10))
                .await;
        }
        panic!("the shell did not exit");
    }

    #[gpui::test]
    async fn test_basic_terminal(cx: &mut TestAppContext) {
        cx.executor().allow_parking();

        let terminal = build_test_terminal(cx, "echo", &["hello"]).await;
        assert_eq!(wait_for_exit(&terminal, cx).await, ExitStatus::default());
        assert_content_eventually(&terminal, "hello", cx).await;

        // Inject additional output directly into the emulator (display-only path)
        terminal.update(cx, |term, cx| {
            term.write_output(b"\nfrom_injection", cx);
        });

        let content_after = terminal.update(cx, |term, _| term.get_content());
        assert!(
            content_after.contains("from_injection"),
            "expected injected output to appear, got: {content_after}"
        );
    }

    #[gpui::test(iterations = 10)]
    async fn test_terminal_no_exit_on_spawn_failure(cx: &mut TestAppContext) {
        cx.executor().allow_parking();

        let terminal = build_test_terminal(cx, "asdasdasdasd", &["@@@@@"]).await;

        let all_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
        cx.update({
            let all_events = all_events.clone();
            |cx| {
                cx.subscribe(&terminal, move |_, e, _| {
                    all_events.lock().push(e.clone());
                })
            }
        })
        .detach();
        let exit_status = wait_for_exit(&terminal, cx).await;
        assert!(
            !exit_status.success(),
            "Wrong shell command should result in a failure"
        );
        assert_eq!(exit_status.code(), Some(1));
        cx.executor().timer(Duration::from_millis(500)).await;

        assert!(
            !all_events
                .lock()
                .iter()
                .any(|event| event == &Event::CloseTerminal),
            "Wrong shell command should update the title but not should not close the terminal to show the error message, but got events: {all_events:?}",
        );
    }

    #[test]
    fn test_terminal_env_names_supraman_and_forces_hyperlinks() {
        let mut env = HashMap::default();
        insert_terminal_env(&mut env, &"1.2.3");
        assert_eq!(env.get("TERM_PROGRAM").map(String::as_str), Some("supraman"));
        assert_eq!(env.get("FORCE_HYPERLINK").map(String::as_str), Some("1"));
        assert_eq!(
            env.get("TERM_PROGRAM_VERSION").map(String::as_str),
            Some("1.2.3")
        );
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(
            env.get("CLAUDE_CODE_FORCE_SYNC_OUTPUT").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            env.get("CLAUDE_CODE_FORCE_STRIKETHROUGH")
                .map(String::as_str),
            Some("1")
        );
        assert!(!env.contains_key("ZED_TERM"));
    }
}

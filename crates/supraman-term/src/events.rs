// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::alacritty::{
    AlacrittyTerm, HyperlinkMatch, display_offset, find_from_terminal_point, resize,
    scroll_display, scroll_to_point, selection_text, set_selection as set_term_selection,
    update_selection as update_term_selection,
};
use crate::bounds::normalize_terminal_bounds;
use crate::input::trim_line_ends;
use crate::mappings::colors::to_vte_rgb;
use crate::mappings::mouse::{grid_point, grid_point_and_side};
use crate::mouse::{Selection, SelectionType};
use crate::process::{PtyResources, TerminalType};
use crate::scroll::Scroll;
use crate::terminal_settings::TerminalConfig;
use crate::{MaybeNavigationTarget, Point, Terminal, TerminalBounds, get_color_at_index};
use alacritty_terminal::grid::Dimensions as _;
use gpui::{ClipboardItem, Context, Pixels, Point as GpuiPoint, Window};
use log::trace;
use std::fmt;
use std::fmt::Formatter;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::Arc;
use vte::ansi::Rgb;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    TitleChanged,
    BreadcrumbsChanged,
    CloseTerminal,
    Bell,
    Wakeup,
    BlinkChanged(bool),
    SelectionsChanged,
    NewNavigationTarget(Option<MaybeNavigationTarget>),
    Open(MaybeNavigationTarget),
}

#[derive(Clone)]
pub(crate) enum InternalEvent {
    Resize(TerminalBounds),
    Scroll(Scroll),
    ScrollToPoint(Point),
    SetSelection(Option<Selection>),
    /// Anchors a selection at a position, resolved against the grid as it is when processed,
    /// as `UpdateSelection` is: output since the last frame may have moved `display_offset`.
    BeginSelection(SelectionType, GpuiPoint<Pixels>),
    UpdateSelection(GpuiPoint<Pixels>),
    FindHyperlink(GpuiPoint<Pixels>, bool),
    ProcessHyperlink(HyperlinkMatch, bool),
    Copy(Option<bool>),
}

pub(crate) type ClipboardFormatter = Arc<dyn Fn(&str) -> String + Sync + Send + 'static>;

pub(crate) type ColorFormatter = Arc<dyn Fn(Rgb) -> String + Sync + Send + 'static>;

pub(crate) type TextAreaSizeFormatter =
    Arc<dyn Fn(TerminalBounds) -> String + Sync + Send + 'static>;

#[derive(Clone)]
pub(crate) enum TerminalBackendEvent {
    MouseCursorDirty,
    Title(String),
    ResetTitle,
    ClipboardStore(String),
    ClipboardLoad(ClipboardFormatter),
    ColorRequest(usize, ColorFormatter),
    PtyWrite(String),
    TextAreaSizeRequest(TextAreaSizeFormatter),
    CursorBlinkingChange,
    Wakeup,
    Bell,
    Exit,
    ChildExit(ExitStatus),
}

impl fmt::Debug for TerminalBackendEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MouseCursorDirty => f.write_str("MouseCursorDirty"),
            Self::Title(title) => write!(f, "Title({title})"),
            Self::ResetTitle => f.write_str("ResetTitle"),
            Self::ClipboardStore(data) => write!(f, "ClipboardStore({data})"),
            Self::ClipboardLoad(_) => f.write_str("ClipboardLoad"),
            Self::ColorRequest(index, _) => write!(f, "ColorRequest({index})"),
            Self::PtyWrite(output) => write!(f, "PtyWrite({output})"),
            Self::TextAreaSizeRequest(_) => f.write_str("TextAreaSizeRequest"),
            Self::CursorBlinkingChange => f.write_str("CursorBlinkingChange"),
            Self::Wakeup => f.write_str("Wakeup"),
            Self::Bell => f.write_str("Bell"),
            Self::Exit => f.write_str("Exit"),
            Self::ChildExit(status) => write!(f, "ChildExit({status})"),
        }
    }
}

/// `title` is ConPTY's name for a console nothing named: the shell program itself, or the absolute path of
/// whichever executable runs in it (`pwsh.exe` from its install folder, `claude.exe`). `program` is
/// often `""`, as `resolve_path` leaves an App Execution Alias. An elevated console prefixes it with
/// "Administrator: " (localised), so any prefix ending in ": " before the path counts too.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_default_title(program: Option<&str>, title: &str) -> bool {
    let title = title.trim();
    let path_part = title
        .match_indices(": ")
        .map(|(i, _)| &title[i + 2..])
        .find(|rest| Path::new(rest).is_absolute())
        .unwrap_or(title);
    let path = Path::new(path_part);
    program.is_some_and(|p| !p.is_empty() && p.eq_ignore_ascii_case(title))
        || path.is_absolute()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

impl Terminal {
    pub(crate) fn process_event(&mut self, event: TerminalBackendEvent, cx: &mut Context<Self>) {
        match event {
            TerminalBackendEvent::Title(title) => {
                // ConPTY names the console after the running executable whenever nothing else names
                // it (at start, and when a program clears its title): that is no title
                #[cfg(windows)]
                let title = if is_default_title(self.shell_program.as_deref(), &title) {
                    String::new()
                } else {
                    title
                };

                self.breadcrumb_text = title;
                cx.emit(Event::BreadcrumbsChanged);
            }
            TerminalBackendEvent::ResetTitle => {
                self.breadcrumb_text = String::new();
                cx.emit(Event::BreadcrumbsChanged);
            }
            TerminalBackendEvent::ClipboardStore(data) => {
                cx.write_to_clipboard(ClipboardItem::new_string(data))
            }
            TerminalBackendEvent::ClipboardLoad(format) => {
                self.write_to_pty(
                    match &cx.read_from_clipboard().and_then(|item| item.text()) {
                        // The terminal only supports pasting strings, not images.
                        Some(text) => format(text),
                        _ => format(""),
                    }
                    .into_bytes(),
                )
            }
            TerminalBackendEvent::PtyWrite(out) => self.write_to_pty(out.into_bytes()),
            TerminalBackendEvent::TextAreaSizeRequest(format) => {
                self.write_to_pty(format(self.last_content.terminal_bounds).into_bytes())
            }
            TerminalBackendEvent::CursorBlinkingChange => {
                let terminal = self.term.lock();
                let blinking = terminal.cursor_style().blinking;
                cx.emit(Event::BlinkChanged(blinking));
            }
            TerminalBackendEvent::Bell => {
                cx.emit(Event::Bell);
            }
            TerminalBackendEvent::Exit => self.register_task_finished(None, cx),
            TerminalBackendEvent::MouseCursorDirty => {
                //NOOP, Handled in render
            }
            TerminalBackendEvent::Wakeup => {
                cx.emit(Event::Wakeup);

                if let TerminalType::Pty { info, .. } = &self.terminal_type {
                    info.emit_title_changed_if_changed(cx);
                }
            }
            TerminalBackendEvent::ColorRequest(index, format) => {
                // Answered here, in order with other PTY writes: `OSC 11 ; ? ST` then `CSI c` must get their replies in
                // that order. Not from `last_content`, which a set-colour just before the query leaves stale.

                let color = self.term.lock().colors()[index].unwrap_or_else(|| {
                    to_vte_rgb(get_color_at_index(index, &TerminalConfig::get(cx).palette))
                });
                self.write_to_pty(format(color).into_bytes());
            }
            TerminalBackendEvent::ChildExit(exit_status) => {
                self.register_task_finished(Some(exit_status), cx);
            }
        }
    }

    pub(crate) fn process_terminal_event(
        &mut self,
        event: &InternalEvent,
        term: &mut AlacrittyTerm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            &InternalEvent::Resize(new_bounds) => {
                let new_bounds = normalize_terminal_bounds(new_bounds);
                trace!("Resizing: new_bounds={new_bounds:?}");

                let columns_changed =
                    self.last_content.terminal_bounds.num_columns() != new_bounds.num_columns();
                self.last_content.terminal_bounds = new_bounds;

                if let TerminalType::Pty {
                    resources: PtyResources::Active(pty_tx),
                    ..
                } = &self.terminal_type
                {
                    pty_tx.resize(new_bounds);
                }

                resize(term, new_bounds);
                if columns_changed {
                    self.reset_cwd_history();
                }
                // matches move with the new layout and must be recalculated
                if !self.matches.is_empty() {
                    cx.emit(Event::Wakeup);
                }
            }
            InternalEvent::Scroll(scroll) => {
                trace!("Scrolling: scroll={scroll:?}");
                scroll_display(term, *scroll);
                self.refresh_hovered_word(window, cx);
            }
            InternalEvent::SetSelection(selection) => {
                trace!("Setting selection: selection={selection:?}");
                set_term_selection(term, selection.as_ref());

                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                if let Some(selection_text) = selection_text(term) {
                    cx.write_to_primary(ClipboardItem::new_string(selection_text));
                }

                if let Some(selection) = selection {
                    self.selection_head = Some(selection.head);
                }
                cx.emit(Event::SelectionsChanged)
            }
            InternalEvent::BeginSelection(selection_type, position) => {
                let (point, side) = grid_point_and_side(
                    *position,
                    self.last_content.terminal_bounds,
                    display_offset(term),
                );
                let selection = Some(Selection::new(*selection_type, point, side));
                self.process_terminal_event(
                    &InternalEvent::SetSelection(selection),
                    term,
                    window,
                    cx,
                );
            }
            InternalEvent::UpdateSelection(position) => {
                trace!("Updating selection: position={position:?}");
                let (point, side) = grid_point_and_side(
                    *position,
                    self.last_content.terminal_bounds,
                    display_offset(term),
                );

                if update_term_selection(term, point, side) {
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    if let Some(selection_text) = selection_text(term) {
                        cx.write_to_primary(ClipboardItem::new_string(selection_text));
                    }

                    self.selection_head = Some(point);
                    cx.emit(Event::SelectionsChanged)
                }
            }

            InternalEvent::Copy(keep_selection) => {
                trace!("Copying selection: keep_selection={keep_selection:?}");
                if let Some(txt) = selection_text(term) {
                    cx.write_to_clipboard(ClipboardItem::new_string(trim_line_ends(&txt)));
                    if !keep_selection
                        .unwrap_or_else(|| TerminalConfig::get(cx).keep_selection_on_copy)
                    {
                        self.events.push_back(InternalEvent::SetSelection(None));
                    }
                }
            }
            InternalEvent::ScrollToPoint(point) => {
                trace!("Scrolling to point: point={point:?}");
                scroll_to_point(term, *point);
                self.refresh_hovered_word(window, cx);
            }
            InternalEvent::FindHyperlink(position, open) => {
                trace!("Finding hyperlink at position: position={position:?}, open={open:?}");

                let point = grid_point(
                    *position,
                    self.last_content.terminal_bounds,
                    display_offset(term),
                );

                match find_from_terminal_point(
                    term,
                    point,
                    &mut self.hyperlink_regex_searches,
                    self.path_style,
                ) {
                    Some(hyperlink) => {
                        let history_size = term.history_size();
                        self.process_hyperlink(hyperlink, *open, history_size, cx);
                    }
                    None => {
                        self.clear_hyperlink(cx);
                    }
                }
            }
            InternalEvent::ProcessHyperlink(hyperlink, open) => {
                // read here: `sync()` holds the term lock while it dispatches events
                let history_size = term.history_size();
                self.process_hyperlink(hyperlink.clone(), *open, history_size, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use gpui::TestAppContext;

    #[cfg(windows)]
    #[test]
    fn the_shells_own_path_is_no_title() {
        // as measured: the program did not resolve
        let none = Some("");
        assert!(is_default_title(
            none,
            r"C:\Program Files\WindowsApps\Microsoft.PowerShell_7.6.7.0_x64__8wekyb3d8bbwe\pwsh.exe"
        ));
        assert!(is_default_title(none, r"c:\users\me\.local\bin\claude.exe"));
        assert!(is_default_title(Some("pwsh"), "pwsh"));
        assert!(is_default_title(None, r"C:\WINDOWS\system32\CMD.EXE"));
        // an elevated Foreman
        assert!(is_default_title(
            none,
            r"Administrator: C:\Program Files\WindowsApps\Microsoft.PowerShell_7.6.7.0_x64__8wekyb3d8bbwe\pwsh.exe"
        ));
        assert!(
            !is_default_title(none, r"notes: P:\Projects\notes"),
            "a prefixed folder is still a title"
        );
        assert!(!is_default_title(none, ""), "empty is a reset, not this");
        assert!(!is_default_title(none, "pwsh.exe"), "not a path");
        assert!(!is_default_title(none, "\u{25D1} fm-selftest-11fa0c"));
        assert!(
            !is_default_title(none, r"P:\Projects\notes"),
            "a folder some prompt shows"
        );
    }

    #[gpui::test]
    async fn test_osc_2_sets_the_title_and_an_empty_one_clears_it(cx: &mut TestAppContext) {
        let terminal = display_only_terminal(cx);
        terminal.update(cx, |terminal, cx| {
            terminal.write_raw_output("\x1b]2;\u{2733} fix tray\x07".as_bytes(), cx)
        });
        cx.run_until_parked();
        terminal.read_with(cx, |terminal, _| {
            assert_eq!(terminal.breadcrumb_text, "\u{2733} fix tray")
        });
        terminal.update(cx, |terminal, cx| {
            terminal.write_raw_output(b"\x1b]0;\x1b\\", cx)
        });
        cx.run_until_parked();
        terminal.read_with(cx, |terminal, _| {
            assert!(terminal.breadcrumb_text.is_empty())
        });
    }
}

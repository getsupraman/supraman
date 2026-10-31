// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::Terminal;
use crate::process::TerminalType;
use alacritty_terminal::grid::Dimensions as _;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CwdHistoryEntry {
    /// Line offset in the retained scrollback buffer.
    pub(crate) scrollback_position: i32,
    pub(crate) working_directory: PathBuf,
}

impl Terminal {
    pub(crate) fn working_directory(&self) -> Option<PathBuf> {
        if self.is_remote_terminal {
            // No reliable working directory on an SSH host.
            None
        } else {
            self.client_side_working_directory()
        }
    }

    pub(crate) fn client_side_working_directory(&self) -> Option<PathBuf> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => info
                .current
                .read()
                .as_ref()
                .map(|process| process.cwd.clone()),
            TerminalType::DisplayOnly => None,
        }
    }

    pub(crate) fn record_cwd_change(&mut self, new_working_directory: PathBuf) {
        if self.is_remote_terminal {
            return;
        }

        let (scrollback_position, history_size) = {
            let term = self.term.lock_unfair();
            let at = self.pending_cwd_boundary.take().unwrap_or_else(|| {
                Self::scrollback_position(term.grid().cursor.point.line.0, term.history_size())
            });
            (at, term.history_size())
        };
        // cwd_at_line ignores the history once the scrollback is full.
        if history_size >= self.term_config.scrolling_history {
            self.cwd_history.clear();
            return;
        }
        self.cwd_history
            .retain(|entry| entry.scrollback_position != scrollback_position);
        self.cwd_history.push(CwdHistoryEntry {
            scrollback_position,
            working_directory: new_working_directory,
        });
    }

    pub(crate) fn reset_cwd_history(&mut self) {
        self.pending_cwd_boundary = None;
        self.cwd_history = self
            .working_directory()
            .map(|working_directory| {
                vec![CwdHistoryEntry {
                    scrollback_position: i32::MIN,
                    working_directory,
                }]
            })
            .unwrap_or_default();
    }

    pub(crate) fn cwd_at_line(&self, line: i32, history_size: usize) -> Option<PathBuf> {
        // Once the scrollback cap is reached, evictions move retained lines without changing
        // `history_size`, so stored row offsets no longer identify their original lines.
        if self.is_remote_terminal
            || self.cwd_history.is_empty()
            || history_size >= self.term_config.scrolling_history
        {
            return self.working_directory();
        }
        let scrollback_position = Self::scrollback_position(line, history_size);
        self.cwd_history
            .iter()
            .rev()
            .find(|entry| entry.scrollback_position <= scrollback_position)
            .map(|entry| entry.working_directory.clone())
            .or_else(|| self.working_directory())
    }

    pub(crate) fn scrollback_position(line: i32, history_size: usize) -> i32 {
        let history_size = i32::try_from(history_size).unwrap_or(i32::MAX);
        history_size.saturating_add(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::terminal_settings::{AlternateScroll, CursorShape as SettingsCursorShape};
    use crate::{Terminal, TerminalBuilder};

    use std::path::PathBuf;

    use util::paths::PathStyle;

    fn make_display_only_terminal() -> Terminal {
        let dispatcher = gpui::TestDispatcher::new(rand::random());
        let executor = gpui::BackgroundExecutor::new(std::sync::Arc::new(dispatcher));
        TerminalBuilder::new_display_only(
            SettingsCursorShape::default(),
            AlternateScroll::On,
            None,
            &executor,
            PathStyle::local(),
        )
        .terminal
    }

    #[test]
    fn test_cwd_at_line_empty_history_returns_none() {
        let terminal = make_display_only_terminal();
        assert_eq!(terminal.cwd_at_line(0, 0), None);
    }

    #[test]
    fn test_cwd_at_line_returns_cwd_for_line_at_or_after_recorded_position() {
        let mut terminal = make_display_only_terminal();
        let working_directory_a = PathBuf::from("/home/user/project_a");
        terminal.cwd_history.push(CwdHistoryEntry {
            scrollback_position: 5,
            working_directory: working_directory_a.clone(),
        });

        // click_pos = history_size(5) + line(3) = 8 >= 5
        assert_eq!(
            terminal.cwd_at_line(3, 5),
            Some(working_directory_a.clone())
        );
        // click_pos = history_size(5) + line(0) = 5 == 5 (exact match)
        assert_eq!(terminal.cwd_at_line(0, 5), Some(working_directory_a));
    }

    #[test]
    fn test_cwd_at_line_ignores_history_at_scrollback_cap() {
        let mut terminal = make_display_only_terminal();
        terminal.term_config.scrolling_history = 10;
        terminal.cwd_history.push(CwdHistoryEntry {
            scrollback_position: 0,
            working_directory: PathBuf::from("/stale/cwd"),
        });

        assert_eq!(terminal.cwd_at_line(-5, 10), None);
    }

    #[test]
    fn test_cwd_at_line_returns_none_when_line_is_before_any_recorded_cwd() {
        let mut terminal = make_display_only_terminal();
        terminal.cwd_history.push(CwdHistoryEntry {
            scrollback_position: 10,
            working_directory: PathBuf::from("/home/user/project_a"),
        });

        // click_pos = 0 + 3 = 3 < 10, no match, falls back to working_directory (None)
        assert_eq!(terminal.cwd_at_line(3, 0), None);
    }

    #[test]
    fn test_cwd_at_line_selects_most_recent_cwd_before_click() {
        let mut terminal = make_display_only_terminal();
        let working_directory_a = PathBuf::from("/home/user/project_a");
        let working_directory_b = PathBuf::from("/home/user/project_b");
        let working_directory_c = PathBuf::from("/home/user/project_c");
        terminal.cwd_history.push(CwdHistoryEntry {
            scrollback_position: 0,
            working_directory: working_directory_a.clone(),
        });
        terminal.cwd_history.push(CwdHistoryEntry {
            scrollback_position: 10,
            working_directory: working_directory_b.clone(),
        });
        terminal.cwd_history.push(CwdHistoryEntry {
            scrollback_position: 20,
            working_directory: working_directory_c.clone(),
        });

        // click_pos=5: between 0 and 10, working_directory_a
        assert_eq!(terminal.cwd_at_line(5, 0), Some(working_directory_a));
        // click_pos=15: between 10 and 20, working_directory_b
        assert_eq!(terminal.cwd_at_line(15, 0), Some(working_directory_b));
        // click_pos=25: after 20, working_directory_c
        assert_eq!(terminal.cwd_at_line(25, 0), Some(working_directory_c));
    }

    #[test]
    fn test_record_cwd_change_stores_entry_at_current_cursor_position() {
        let mut terminal = make_display_only_terminal();
        let working_directory = PathBuf::from("/tmp/test");
        terminal.record_cwd_change(working_directory.clone());

        assert_eq!(terminal.cwd_history.len(), 1);
        let entry = &terminal.cwd_history[0];
        assert_eq!(entry.scrollback_position, 0);
        assert_eq!(entry.working_directory, working_directory);
    }

    #[test]
    fn test_record_cwd_change_keeps_one_entry_per_position_and_none_at_scrollback_cap() {
        let mut terminal = make_display_only_terminal();
        for dir in ["/a", "/b", "/c"] {
            terminal.record_cwd_change(PathBuf::from(dir));
        }
        assert_eq!(
            terminal.cwd_history,
            vec![CwdHistoryEntry {
                scrollback_position: 0,
                working_directory: PathBuf::from("/c"),
            }]
        );

        terminal.term_config.scrolling_history = 0;
        terminal.record_cwd_change(PathBuf::from("/d"));
        assert!(terminal.cwd_history.is_empty());
    }

    #[test]
    fn test_record_cwd_change_uses_command_boundary() {
        let mut terminal = make_display_only_terminal();
        terminal.write_input(b"\r".to_vec());
        assert_eq!(terminal.pending_cwd_boundary, Some(0));

        let working_directory = PathBuf::from("/tmp/test");
        terminal.record_cwd_change(working_directory.clone());

        assert_eq!(terminal.pending_cwd_boundary, None);
        assert_eq!(
            terminal.cwd_history,
            vec![CwdHistoryEntry {
                scrollback_position: 0,
                working_directory,
            }]
        );
    }

    #[test]
    fn test_remote_terminal_does_not_record_local_cwd() {
        let mut terminal = make_display_only_terminal();
        terminal.is_remote_terminal = true;
        terminal.write_input(b"\r".to_vec());
        terminal.record_cwd_change(PathBuf::from("/local/ssh/cwd"));

        assert_eq!(terminal.pending_cwd_boundary, None);
        assert!(terminal.cwd_history.is_empty());
        assert_eq!(terminal.cwd_at_line(0, 0), None);
    }

    #[test]
    fn test_reset_cwd_history_discards_stale_coordinates() {
        let mut terminal = make_display_only_terminal();
        terminal.cwd_history.push(CwdHistoryEntry {
            scrollback_position: 42,
            working_directory: PathBuf::from("/tmp/test"),
        });
        terminal.pending_cwd_boundary = Some(43);

        terminal.reset_cwd_history();

        assert!(terminal.cwd_history.is_empty());
        assert_eq!(terminal.pending_cwd_boundary, None);
    }
}

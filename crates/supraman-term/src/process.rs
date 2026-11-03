// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::alacritty::PtySender;
use crate::pty_info::PtyProcessInfo;
use crate::{Event, Terminal};
use gpui::Context;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;

/// Separates retained PTY process metadata from resources needed only while
/// the terminal is live.
pub(crate) enum PtyResources {
    Active(PtySender),
    Released,
}

pub(crate) enum TerminalType {
    Pty {
        resources: PtyResources,
        info: Arc<PtyProcessInfo>,
    },
    #[cfg_attr(not(test), allow(dead_code))]
    DisplayOnly,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.release_pty_resources();
    }
}

impl Terminal {
    /// Releases live PTY resources while retaining process metadata and buffered output.
    /// Calling this method after the resources have already been released is a no-op.
    pub fn release_pty_resources(&mut self) {
        let TerminalType::Pty { resources, info } = &mut self.terminal_type else {
            return;
        };
        let PtyResources::Active(pty_tx) = std::mem::replace(resources, PtyResources::Released)
        else {
            return;
        };
        let info = info.clone();

        pty_tx.shutdown();
        info.terminate_child_process();

        let timer = self.background_executor.timer(Duration::from_millis(100));
        self.background_executor
            .spawn(async move {
                timer.await;
                info.kill_child_process();
            })
            .detach();
    }

    pub fn pid(&self) -> Option<sysinfo::Pid> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => info.pid(),
            TerminalType::DisplayOnly => None,
        }
    }

    pub(crate) fn register_task_finished(
        &mut self,
        exit_status: Option<ExitStatus>,
        cx: &mut Context<Terminal>,
    ) {
        if let Some(e) = exit_status {
            self.child_exited = Some(e);
        }
        // A shell that got input closes on any exit; one that never did stays open on failure, to show a spawn error.
        let should_close = if self.keyboard_input_sent {
            true
        } else {
            self.child_exited.is_none_or(|e| e.code() == Some(0))
        };
        if should_close {
            cx.emit(Event::CloseTerminal);
        }
    }
}

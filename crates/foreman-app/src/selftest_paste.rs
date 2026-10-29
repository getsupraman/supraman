//! `--selftest` step 7r (m5/46): an image on the clipboard survives a step that borrows the clipboard
//! (as 6b, 6c and 7l do), and claude's Alt+V image check, run in a Foreman shell tab, still finds it.
//! The tab runs the exact command claude 2.1 runs on win32, through `cmd` so that pwsh's fix-up of a
//! `powershell.exe` child's environment does not apply, as it does not for claude. The user's clipboard
//! is saved first and put back afterwards ([`crate::selftest_clipboard`]).

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{hex6, sleep, wait_for_tab_terminal},
    selftest_clipboard::{Snapshot, put_test_image, sequence},
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(30);

/// claude 2.1.312's `checkImage` on win32, verbatim.
pub(crate) const CHECK: &str = "Add-Type -AssemblyName System.Windows.Forms; if (-not [System.Windows.Forms.Clipboard]::ContainsImage()) { exit 1 }";

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, cx).await;
    view.update(cx, |v, _| v.shell_args = saved);
    result
}

async fn check(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let id = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let result = async {
        let terminal = wait_for_tab_terminal(view, id, WITHIN, cx).await?;
        let started = Instant::now();
        while !terminal.read_with(cx, |t, _| t.get_content()).contains("PS ") {
            if started.elapsed() > WITHIN {
                bail!("tab {id}: no pwsh prompt");
            }
            sleep(cx, Duration::from_millis(50)).await;
        }
        let mark = format!("fm-image-{}=", hex6());
        let _clipboard = crate::selftest_clipboard::borrow("7r", cx).await?;
        let user = Snapshot::take().map_err(|e| anyhow!("saving the clipboard: {e}"))?;
        let held = Instant::now();
        let exit = async {
            put_test_image().map_err(|e| anyhow!("putting the test image on the clipboard: {e}"))?;
            let borrowed = Snapshot::take()?;
            crate::selftest_clipboard::put_text(&mark, cx)?;
            borrowed.restore()?;
            let restored = sequence();
            terminal.update(cx, |t, _| {
                t.input(format!("cmd /d /c powershell -NoProfile -NonInteractive -Sta -Command \"{CHECK}\"; '{mark}' + $LASTEXITCODE\r").into_bytes())
            });
            loop {
                let content = terminal.read_with(cx, |t, _| t.get_content());
                // the echoed command has `'` right after the mark; the printed line has the exit code
                if let Some(code) = content.lines().filter_map(|l| l.trim().strip_prefix(mark.as_str())).find(|c| !c.starts_with('\'')) {
                    return Ok((code.trim().to_string(), sequence() != restored));
                }
                if held.elapsed() > WITHIN {
                    bail!("tab {id} never printed the check's exit code: {content:?}");
                }
                sleep(cx, Duration::from_millis(50)).await;
            }
        }
        .await;
        let restored = user.restore();
        let held = held.elapsed();
        let (exit, written) = exit?;
        restored.map_err(|e| anyhow!("restoring the clipboard: {e}"))?;
        if exit != "0" && written {
            bail!(
                "claude's image check in a Foreman tab exited {exit}, but another process wrote the clipboard between the put-back and the check (the clipboard is system-wide: another selftest running at the same time takes it)"
            );
        }
        if exit != "0" {
            bail!("claude's image check in a Foreman tab exited {exit} with the test image borrowed and put back on the clipboard, which nothing wrote since");
        }
        Ok(format!(
            "test image on the clipboard, borrowed and put back; claude's image check in tab {id} exited 0; the user's {} clipboard formats back after {} ms",
            user.formats(),
            held.as_millis()
        ))
    }
    .await;
    view.update(cx, |v, cx| v.close_tab(id, cx));
    result
}

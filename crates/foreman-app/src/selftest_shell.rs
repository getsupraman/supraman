//! `--selftest` step 7g (m5/22): where the tab bar's `+` shell starts. With a Work tab active, in that
//! workspace's folder; with no tab, in the first workspace's; never in the exe's folder. Each shell
//! prints its own `Get-Location`, so this is the pwsh's real cwd, not only what the tab recorded.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _};
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{sleep, wait_for_tab_terminal},
    selftest_work::WORKSPACE,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);
const MARK: &str = "fm-cwd=";

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, cx).await;
    view.update(cx, |v, _| v.shell_args = saved);
    close_all(view, cx);
    result
}

async fn check(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    close_all(view, cx);
    let (work_root, first_root) = view.read_with(cx, |v, _| {
        let root = |name: &str| v.workspaces.iter().find(|w| w.name == name).map(|w| w.root.clone());
        (root(WORKSPACE), v.workspaces.first().map(|w| w.root.clone()))
    });
    let (Some(work_root), Some(first_root)) = (work_root, first_root) else {
        bail!("the registry has no {WORKSPACE} or no first workspace");
    };
    if same(&work_root, &first_root) {
        bail!("{WORKSPACE} is the first workspace, so this step could not tell the two rules apart");
    }

    view.update(cx, |v, cx| v.open_work(WORKSPACE, cx));
    let in_work = new_shell_cwd(view, cx).await.context("with the Work tab active")?;
    if !same(Path::new(&in_work), &work_root) {
        bail!("with Work · {WORKSPACE} active the new shell is in {in_work}, not {}", work_root.display());
    }

    close_all(view, cx);
    let no_tab = new_shell_cwd(view, cx).await.context("with no tab")?;
    if !same(Path::new(&no_tab), &first_root) {
        bail!("with no tab the new shell is in {no_tab}, not the first workspace's {}", first_root.display());
    }
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    if exe_dir.is_some_and(|d| same(&d, Path::new(&no_tab)) || same(&d, Path::new(&in_work))) {
        bail!("a new shell started in the exe's folder");
    }
    Ok(format!("new shell with Work · {WORKSPACE} active in {in_work}; with no tab in the first workspace's {no_tab}"))
}

/// Clicks the tab bar's `+` (as `new_shell`) and reads the folder its pwsh reports.
async fn new_shell_cwd(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let id = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let terminal = wait_for_tab_terminal(view, id, WITHIN, cx).await?;
    terminal.update(cx, |t, _| t.input(format!("Write-Host ('{MARK}' + (Get-Location).Path)\r").into_bytes()));
    let started = Instant::now();
    loop {
        let content = terminal.read_with(cx, |t, _| t.get_content());
        // the echoed command has `'` right after the mark; the printed line has the path
        if let Some(path) = content.lines().filter_map(|l| l.trim().strip_prefix(MARK)).find(|p| !p.starts_with('\'')) {
            return Ok(path.trim().to_string());
        }
        if started.elapsed() > WITHIN {
            bail!("tab {id} never printed its cwd: {content:?}");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn close_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) {
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| ids.into_iter().for_each(|id| v.close_tab(id, cx)));
}

/// `P:/a/b` and `p:\a\b\` are one folder.
fn same(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    key(a) == key(b)
}

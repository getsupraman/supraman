//! `--selftest` step 7b (m5/12): managing workspaces through the window. Add workspace on a scratch folder
//! ([`FOLDER`] under `<cargo root>/target/`, two fake repos; [`FleetView::pick_folder`] stands in for the OS
//! picker): the review lists both, one is unticked, the name is edited with keys, Enter saves; the tree shows
//! both repos (the unticked one disabled). The same folder again is refused inline. Then the right-click
//! menus: the gate session's row (opened and dismissed with Esc, nothing run), the new workspace's (dismissed
//! by a click outside it), the disabled repo's Enable, and the workspace's Remove… confirmed: it leaves the
//! registry and the tree, and the folder is still there. With `FOREMAN_SHOTS=<dir>` each state is
//! screenshotted there (`scripts/shot.ps1`).

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use foreman_claude::Session;
use foreman_core::Registry;
use gpui::{
    AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, point, px,
};

use crate::{
    selftest::{cargo_root, click, press, sleep},
    view::FleetView,
};

pub const FOLDER: &str = "selftest-add";
/// What the review's name is edited into.
const ADDED: &str = "selftest-added";
const WITHIN: Duration = Duration::from_secs(5);

/// Polls `view` every 50 ms until `f` has an answer, or fails with `what` after [`WITHIN`].
async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = view.read_with(cx, |v, _| f(v)) {
            return Ok(t);
        }
        if started.elapsed() > WITHIN {
            bail!("{what}: not within {} s", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.hits.borrow().contains_key(key)
}

fn typed(cx: &mut AsyncWindowContext, key: &str, key_char: Option<&str>) -> anyhow::Result<()> {
    let keystroke = Keystroke { modifiers: Modifiers::default(), key: key.into(), key_char: key_char.map(Into::into) };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx);
    })?;
    Ok(())
}

fn fake_repo(root: &Path, name: &str) -> anyhow::Result<()> {
    let git = root.join(name).join(".git");
    std::fs::create_dir_all(&git)?;
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    Ok(())
}

/// `scripts/shot.ps1` on this process's window, into `$FOREMAN_SHOTS/<name>.png`, when that is set.
pub(crate) async fn shot(name: &str, cx: &mut AsyncWindowContext) {
    shot_window(name, None, cx).await
}

/// [`shot`] of this process's top-level window titled `title` (the settings window), or its main one.
pub(crate) async fn shot_window(name: &str, title: Option<&str>, cx: &mut AsyncWindowContext) {
    let title = title.map(str::to_string);
    let Some(dir) = std::env::var_os("FOREMAN_SHOTS").map(PathBuf::from) else {
        return;
    };
    sleep(cx, Duration::from_millis(300)).await;
    let out = dir.join(format!("{name}.png"));
    let script = cargo_root().join("scripts").join("shot.ps1");
    let pid = std::process::id().to_string();
    let done = cx
        .background_executor()
        .spawn(async move {
            std::fs::create_dir_all(&dir)?;
            std::process::Command::new("pwsh")
                .args(["-NoLogo", "-NoProfile", "-File"])
                .arg(&script)
                .args(["-ProcessId", &pid, "-Out"])
                .arg(&out)
                .args(title.iter().flat_map(|t| ["-Title", t.as_str()]))
                .output()
                .map(|o| (out, o))
        })
        .await;
    match done {
        Ok((out, o)) if o.status.success() => println!("shot: {}", out.display()),
        Ok((out, o)) => println!("shot: {} failed: {}", out.display(), String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => println!("shot: {name} failed: {e}"),
    }
}

/// The menu's item keys, once it is open with `title`.
async fn menu_open(view: &Entity<FleetView>, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<&'static str>> {
    let keys = until(view, cx, &format!("the menu of {title}"), |v| {
        v.menu.as_ref().filter(|m| m.title == title).map(|m| m.items.iter().map(|i| i.key).collect::<Vec<_>>())
    })
    .await
    .map_err(|e| {
        let state = view.read_with(cx, |v, _| {
            format!("menu {:?}, confirm {:?}", v.menu.as_ref().map(|m| m.title.clone()), v.confirm.as_ref().map(|c| c.text.clone()))
        });
        e.context(state)
    })?;
    let first = format!("menu-{}", keys.first().ok_or_else(|| anyhow!("the menu of {title} is empty"))?);
    until(view, cx, &format!("{first} painted"), |v| painted(v, &first).then_some(())).await?;
    Ok(keys)
}

/// [`press`] once `key` is painted (a row can be missing from the frame being drawn).
async fn press_painted(view: &Entity<FleetView>, key: &str, button: MouseButton, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    until(view, cx, &format!("{key} painted"), |v| painted(v, key).then_some(())).await.map_err(|e| {
        let (keys, rows) = view.read_with(cx, |v, _| {
            let mut keys: Vec<String> = v.hits.borrow().keys().filter(|k| k.starts_with("ws-row-")).cloned().collect();
            keys.sort();
            (keys, v.workspaces.iter().map(|w| w.name.clone()).collect::<Vec<_>>())
        });
        e.context(format!("painted rows {keys:?}; workspaces {rows:?}"))
    })?;
    press(view, key, button, cx).await
}

pub async fn run(view: &Entity<FleetView>, own: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let root = cargo_root().join("target").join(FOLDER);
    fake_repo(&root, "alpha")?;
    fake_repo(&root, "beta")?;
    let home = view.read_with(cx, |v, _| v.home.clone()).ok_or_else(|| anyhow!("no Foreman home"))?;
    let registry = Registry::at(&home);

    // Add: the review
    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    let (name, repos) = until(view, cx, "the review's repos", |v| {
        let d = v.adding.as_ref().filter(|d| !d.scanning)?;
        Some((d.name.clone(), d.repos.iter().map(|(r, _)| r.name.clone()).collect::<Vec<_>>()))
    })
    .await?;
    if name != FOLDER || repos != ["alpha", "beta"] {
        bail!("the review offers name {name} with repos {repos:?}, expected {FOLDER} with [alpha, beta]");
    }
    until(view, cx, "Save painted", |v| painted(v, "add-save").then_some(())).await?;
    click(view, "add-repo-beta", cx).await?;
    until(view, cx, "beta unticked", |v| v.adding.as_ref().is_some_and(|d| !d.repos[1].1).then_some(())).await?;
    click(view, "add-name", cx).await?;
    for _ in 0.."add".len() {
        typed(cx, "backspace", None)?;
    }
    for c in "added".chars() {
        let c = c.to_string();
        typed(cx, &c, Some(&c))?;
    }
    let edited = view.read_with(cx, |v, _| v.adding.as_ref().map(|d| d.name.clone()));
    if edited.as_deref() != Some(ADDED) {
        bail!("the name reads {edited:?} after editing, expected {ADDED}");
    }
    shot("add-review", cx).await;
    typed(cx, "enter", None)?;
    until(view, cx, "the review closed on Enter", |v| v.adding.is_none().then_some(())).await?;
    let saved = registry.load(ADDED).context("the registry after Save")?;
    let flags: Vec<(String, bool)> = saved.repos.iter().map(|r| (r.name.clone(), r.enabled)).collect();
    if flags != [("alpha".to_string(), true), ("beta".to_string(), false)] {
        bail!("registered {ADDED} with repos {flags:?}, expected alpha enabled and beta disabled");
    }
    let (alpha, beta) = (format!("repo-row-{ADDED}/alpha"), format!("repo-row-{ADDED}/beta"));
    until(view, cx, "the new workspace's repos in the tree", |v| (painted(v, &alpha) && painted(v, &beta)).then_some(())).await?;
    let mut out = vec![format!("added {ADDED} from {} (alpha enabled, beta disabled), both repos in the tree", root.display())];

    // the same folder again: refused inline
    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    until(view, cx, "the second review", |v| v.adding.as_ref().filter(|d| !d.scanning).map(|_| ())).await?;
    until(view, cx, "Save painted", |v| painted(v, "add-save").then_some(())).await?;
    click(view, "add-save", cx).await?;
    let refused = until(view, cx, "the refusal", |v| v.adding.as_ref().and_then(|d| d.error.clone())).await?;
    if !refused.contains(&format!("already registered, as workspace {ADDED}")) {
        bail!("the same folder again says: {refused}");
    }
    click(view, "add-cancel", cx).await?;
    until(view, cx, "the review closed on Cancel", |v| v.adding.is_none().then_some(())).await?;
    out.push(format!("the same folder again: \"{refused}\""));

    // the gate session's menu: opened and dismissed with Esc, nothing run
    let session_key = format!("session-row-{}", own.session_id);
    press_painted(view, &session_key, MouseButton::Right, cx).await?;
    let keys = menu_open(view, &own.name, cx).await?;
    for want in ["attach", "stop-session", "copy-name", "copy-id"] {
        if !keys.contains(&want) {
            bail!("the menu of {} has {keys:?}, no {want}", own.name);
        }
    }
    shot("menu-session", cx).await;
    typed(cx, "escape", None)?;
    until(view, cx, "the session menu closed on Esc", |v| v.menu.is_none().then_some(())).await?;
    out.push(format!("session menu {keys:?} closed by Esc"));

    // the workspace's menu: dismissed by a click outside it
    let ws_key = format!("ws-row-{ADDED}");
    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    let keys = menu_open(view, ADDED, cx).await?;
    if keys != ["start", "work", "rescan", "open-folder", "open-terminal", "remove"] {
        bail!("the menu of {ADDED} has {keys:?}");
    }
    shot("menu-workspace", cx).await;
    cx.update(|window, cx| {
        // the right panel's bottom-right corner, far from the menu and from any button
        let size = window.viewport_size();
        let outside = point(size.width - px(24.), size.height - px(24.));
        let modifiers = Modifiers::default();
        let down = MouseDownEvent { button: MouseButton::Left, position: outside, modifiers, click_count: 1, first_mouse: false };
        window.dispatch_event(PlatformInput::MouseDown(down), cx);
        let up = MouseUpEvent { button: MouseButton::Left, position: outside, modifiers, click_count: 1 };
        window.dispatch_event(PlatformInput::MouseUp(up), cx);
    })?;
    until(view, cx, "the workspace menu closed by a click outside", |v| v.menu.is_none().then_some(())).await?;
    out.push(format!("workspace menu {keys:?} closed by a click outside"));

    // the disabled repo's Enable
    press_painted(view, &beta, MouseButton::Right, cx).await?;
    let keys = menu_open(view, &format!("{ADDED} · ./beta"), cx).await?;
    if keys != ["open-terminal", "open-folder", "enable"] {
        bail!("the menu of the disabled beta has {keys:?}");
    }
    shot("menu-repo", cx).await;
    click(view, "menu-enable", cx).await?;
    until(view, cx, "beta enabled in the tree", |v| {
        v.tree().workspaces.iter().find(|w| w.name == ADDED)?.repos.iter().find(|r| r.name == "beta" && r.enabled).map(|_| ())
    })
    .await?;
    // painted: the next press must see this frame's layout, not the one before Enable
    let plus = format!("new-claude-{ADDED}/beta");
    until(view, cx, "beta's + painted", |v| painted(v, &plus).then_some(())).await?;
    if !registry.load(ADDED)?.repos.iter().all(|r| r.enabled) {
        bail!("Enable did not reach the registry");
    }
    out.push("beta enabled from its menu".into());

    // Remove… confirmed
    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    menu_open(view, ADDED, cx).await?;
    click(view, "menu-remove", cx).await?;
    until(view, cx, "the Remove question", |v| painted(v, "confirm-yes").then_some(())).await?;
    let question = view.read_with(cx, |v, _| v.confirm.as_ref().map(|c| c.text.clone())).unwrap_or_default();
    if !question.contains("No files are deleted") {
        bail!("the Remove question reads: {question}");
    }
    shot("confirm-remove", cx).await;
    click(view, "confirm-yes", cx).await?;
    until(view, cx, "the workspace gone from the tree", |v| {
        (!v.workspaces.iter().any(|w| w.name == ADDED) && !painted(v, &ws_key)).then_some(())
    })
    .await?;
    if registry.exists(ADDED)? {
        bail!("{ADDED} is still registered after Remove");
    }
    if !root.join("alpha").join(".git").join("HEAD").is_file() || !root.join("beta").is_dir() {
        bail!("Remove touched the folder {}", root.display());
    }
    out.push(format!("removed via its menu; {} and its repos still on disk", root.display()));
    Ok(out.join("; "))
}

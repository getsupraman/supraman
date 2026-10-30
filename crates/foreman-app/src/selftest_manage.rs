//! `--selftest` step 7b: managing workspaces through the window.

use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use foreman_claude::Session;
use foreman_core::Registry;
use gpui::{
    AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point,
    px,
};

use crate::{
    selftest::{cargo_root, click, press, sleep, until, until_app},
    view::FleetView,
};

pub const FOLDER: &str = "selftest-add";
pub const SINGLE: &str = "selftest-single";
const ADDED: &str = "selftest-added";
const WITHIN: Duration = Duration::from_secs(5);

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

/// As the OS queues a real click or key: through gpui's window procedure (and `TranslateMessage`), not straight to `dispatch_event`.
fn post(cx: &mut AsyncWindowContext, messages: &[(u32, usize, isize)]) -> anyhow::Result<()> {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::PostMessageW,
    };
    let hwnd = cx.update(|window, _| crate::tray::hwnd(window))?.ok_or_else(|| anyhow!("no HWND"))?;
    for &(message, wparam, lparam) in messages {
        unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), LPARAM(lparam)) }.map_err(|e| anyhow!("PostMessage: {e}"))?;
    }
    Ok(())
}

async fn post_click(view: &Entity<FleetView>, key: &str, dx: f32, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    let bounds = crate::selftest::scrolled_into_view(view, key, cx).await?;
    let scale = cx.update(|window, _| window.scale_factor())?;
    let x = ((f32::from(bounds.left()) + dx) * scale) as isize;
    let y = (f32::from(bounds.center().y) * scale) as isize;
    let at = (y << 16) | (x & 0xffff);
    post(cx, &[(WM_MOUSEMOVE, 0, at), (WM_LBUTTONDOWN, 1, at), (WM_LBUTTONUP, 0, at)])?;
    sleep(cx, Duration::from_millis(150)).await;
    Ok(())
}

/// A key that types goes on to `TranslateMessage`'s WM_CHAR, queued behind it: read what it typed with [`until_app`], never right after this returns (F-04).
pub(crate) async fn post_keys(keys: &[&str], cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    use windows::Win32::UI::{
        Input::KeyboardAndMouse::{
            GetKeyState, MAPVK_VK_TO_VSC, MapVirtualKeyW, VK_BACK, VK_CONTROL, VK_END, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT,
            VkKeyScanW,
        },
        WindowsAndMessaging::{WM_KEYDOWN, WM_KEYUP},
    };
    // gpui takes a key's modifiers from GetKeyState, which a posted message does not set: a held modifier would turn End into Alt+End.
    let held = || {
        [(VK_CONTROL, "Ctrl"), (VK_MENU, "Alt"), (VK_SHIFT, "Shift"), (VK_LWIN, "Win"), (VK_RWIN, "Win")]
            .into_iter()
            .filter(|(vk, _)| unsafe { GetKeyState(vk.0 as i32) } < 0)
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
    };
    let started = Instant::now();
    while !held().is_empty() {
        if started.elapsed() > WITHIN {
            bail!("{:?} held in this thread's key state for {} s: posted keys would carry it (another process's input?)", held(), WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    for key in keys {
        let vk = match *key {
            "end" => VK_END.0 as u32,
            "backspace" => VK_BACK.0 as u32,
            "enter" => VK_RETURN.0 as u32,
            "escape" => VK_ESCAPE.0 as u32,
            c => (unsafe { VkKeyScanW(c.encode_utf16().next().unwrap_or_default()) } & 0xff) as u32,
        };
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as isize;
        post(cx, &[(WM_KEYDOWN, vk as usize, 1 | (scan << 16)), (WM_KEYUP, vk as usize, 1 | (scan << 16) | (0b11 << 30))])?;
    }
    sleep(cx, Duration::from_millis(150)).await;
    Ok(())
}

async fn edit_name(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let name = |v: &FleetView, cx: &gpui::App| {
        v.adding.as_ref().map(|d| {
            let n = d.name.read(cx);
            (n.text().to_string(), n.line.cursor())
        })
    };
    let now = |cx: &mut AsyncWindowContext| view.read_with(cx, |v, cx| name(v, cx));
    post_click(view, "add-name-text", 1., cx).await?;
    let at_start = Some((FOLDER.to_string(), 0));
    until_app(view, cx, WITHIN, "the caret at the start of the name after a click there", |v, cx| (name(v, cx) == at_start).then_some(()))
        .await
        .map_err(|e| e.context(format!("(name, caret) {:?}, expected ({FOLDER}, 0)", now(cx))))?;
    let (mut lit, mut unlit) = (0, 0);
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(2500) && (lit < 2 || unlit < 2) {
        match view.read_with(cx, |v, cx| v.adding.as_ref().map(|d| d.name.read(cx).caret_painted.get())) {
            Some(true) => lit += 1,
            Some(false) => unlit += 1,
            None => bail!("the review closed while its caret was watched"),
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
    if lit == 0 || unlit == 0 {
        bail!("the name's caret was painted lit {lit} and unlit {unlit} times in {} ms: it does not blink", started.elapsed().as_millis());
    }
    let seen: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let observer = cx.update(|_, cx| {
        let (seen, view) = (seen.clone(), view.clone());
        cx.observe_keystrokes(move |e, window, cx| {
            let focused = view.read(cx).input_focused(window, cx);
            let action = e.action.as_ref().map_or("-".to_string(), |a| a.name().to_string());
            seen.borrow_mut().push(format!("{} [{action}{}]", e.keystroke.unparse(), if focused { "" } else { ", input not focused" }));
        })
    })?;
    post_keys(&["end", "backspace", "backspace", "backspace", "a", "d", "d", "e", "d"], cx).await?;
    let added = Some((ADDED.to_string(), ADDED.len()));
    let edited = until_app(view, cx, WITHIN, "the name edited by posted End, 3 Backspace and `added`", |v, cx| (name(v, cx) == added).then_some(())).await;
    drop(observer);
    edited.map_err(|e| e.context(format!("(name, caret) {:?}, expected ({ADDED}, {}); keystrokes {:?}", now(cx), ADDED.len(), seen.borrow())))?;
    view.update(cx, |v, _| v.adding.as_mut().map(|d| d.scanning = true));
    post_click(view, "add-save", 4., cx).await?;
    let said = view.update(cx, |v, _| {
        let d = v.adding.as_mut()?;
        d.scanning = false;
        d.error.take()
    });
    let Some(said) = said else {
        bail!("Save while the folder was still read said nothing");
    };
    Ok(format!("name by posted messages: click → caret 0, caret {lit} lit / {unlit} unlit, edited to {ADDED}; Save while reading: \"{said}\""))
}

async fn single_repo(view: &Entity<FleetView>, registry: &Registry, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let root = cargo_root().join("target").join(SINGLE);
    let git = root.join(".git");
    std::fs::create_dir_all(&git)?;
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-from-folder", cx).await?;
    let (repos, error) = until(view, cx, WITHIN, "the single repo's review", |v| {
        let d = v.adding.as_ref().filter(|d| !d.scanning)?;
        Some((d.repos.iter().map(|(r, _)| (r.name.clone(), r.path.clone())).collect::<Vec<_>>(), d.error.clone()))
    })
    .await?;
    if repos != [(SINGLE.to_string(), root.clone())] || error.is_some() {
        bail!("the review of a folder that is a repo lists {repos:?} and says {error:?}, expected [({SINGLE}, {})]", root.display());
    }
    shot("add-review-single-repo", cx).await;
    post_click(view, "add-save", 4., cx).await?;
    until(view, cx, WITHIN, "the single repo's review closed on Save", |v| v.adding.is_none().then_some(())).await?;
    let saved = registry.load(SINGLE).context("the registry after Save")?;
    let paths: Vec<_> = saved.folders.iter().map(|r| (r.name.as_str(), r.path.as_path())).collect();
    if paths != [(SINGLE, root.as_path())] {
        bail!("registered {SINGLE} with repos {paths:?}");
    }
    let row = format!("repo-row-{SINGLE}/{SINGLE}");
    until(view, cx, WITHIN, "the single repo's row in the tree", |v| painted(v, &row).then_some(())).await?;
    registry.remove(SINGLE)?;
    view.update(cx, |v, _| v.reload_workspaces());
    Ok(format!("{SINGLE} (a folder that is a repo): one repo, the root, registered by a posted Save and shown"))
}

/// The move is dispatched: a posted WM_MOUSEMOVE is undone by the WM_MOUSELEAVE the real cursor's position sends at once.
async fn cut_label_tooltip(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let key = format!("ws-root-{ADDED}");
    let full = view.read_with(cx, |v, _| v.workspaces.iter().find(|w| w.name == ADDED).and_then(|w| w.home().map(|h| h.display().to_string())));
    let full = full.ok_or_else(|| anyhow!("{ADDED} is not in the tree"))?;
    let row = crate::selftest::scrolled_into_view(view, &format!("ws-row-{ADDED}"), cx).await?;
    let started = Instant::now();
    let bounds = loop {
        let inside = |b: &gpui::Bounds<gpui::Pixels>| b.size.width > px(40.) && b.center().y > row.top() && b.center().y < row.bottom();
        if let Some(b) = crate::clipped::cut_at(&key).filter(inside) {
            break b;
        }
        if started.elapsed() > WITHIN {
            bail!("{ADDED}'s root path {full:?} was not painted cut, 40 px or wider: {:?}", crate::clipped::cut_at(&key));
        }
        sleep(cx, Duration::from_millis(50)).await;
    };
    crate::view::icon::LAST_SHOWN.with(|l| l.borrow_mut().take());
    let position = bounds.center();
    let started = Instant::now();
    loop {
        cx.update(|window, cx| {
            let event = MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() };
            window.dispatch_event(PlatformInput::MouseMove(event), cx);
        })?;
        sleep(cx, Duration::from_millis(100)).await;
        let shown = crate::view::icon::LAST_SHOWN.with(|l| l.borrow().clone());
        if shown.as_deref() == Some(full.as_str()) {
            break;
        }
        if started.elapsed() > WITHIN {
            bail!("hovering {ADDED}'s cut root path at {position:?} showed the tooltip {shown:?}, expected {full:?}");
        }
    }
    shot("cut-label-tooltip", cx).await;
    Ok(format!("{ADDED}'s root path painted cut ({:.0} px); hovered, its tooltip reads the whole path", f32::from(bounds.size.width)))
}

/// The workspace and session rows' hover cards, as [`foreman_ui::hover_card::HoverCard::text`].
async fn hover_cards(view: &Entity<FleetView>, own: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let ws = card_on(view, &format!("ws-row-{ADDED}"), cx).await?;
    for want in [format!("{ADDED}\n"), "\nfolders  alpha, beta (disabled)\n".into(), "\nstate  never started\n".into(), "\nsession  not wanted\n".into()] {
        if !ws.starts_with(ADDED) || !format!("{ws}\n").contains(&want) {
            bail!("the card of {ADDED}'s row reads {ws:?}, without {want:?}");
        }
    }
    shot("m5-59-workspace-card", cx).await;
    let session = card_on(view, &format!("session-row-{}", own.session_id), cx).await?;
    let cwd = format!("\ncwd  {}\n", own.cwd.display());
    if session.lines().next() != Some(own.name.as_str()) || !session.contains("\nstate  ") || !format!("{session}\n").contains(&cwd) {
        bail!("the card of {}'s row reads {session:?}, not its name, state and {cwd:?}", own.name);
    }
    shot("m5-59-session-card", cx).await;
    Ok(format!("hover cards: {ADDED}'s row {:?}; {}'s row {:?}", ws.replace('\n', " | "), own.name, session.replace('\n', " | ")))
}

/// Rests the pointer near the row's left edge, off its icons and labels, until its card shows.
async fn card_on(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let bounds = crate::selftest::scrolled_into_view(view, key, cx).await?;
    let position = point(bounds.left() + px(4.), bounds.top() + px(6.));
    crate::selftest_left::move_to(point(px(1.), px(1.)), cx)?;
    sleep(cx, Duration::from_millis(100)).await;
    foreman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take());
    let started = Instant::now();
    loop {
        crate::selftest_left::move_to(position, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
        if let Some(text) = foreman_ui::hover_card::LAST_CARD.with(|l| l.borrow().clone()) {
            return Ok(text);
        }
        if started.elapsed() > WITHIN {
            bail!("resting on {key} at {position:?} showed no card");
        }
    }
}

fn fake_repo(root: &Path, name: &str) -> anyhow::Result<()> {
    let git = root.join(name).join(".git");
    std::fs::create_dir_all(&git)?;
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    Ok(())
}

pub(crate) async fn shot(name: &str, cx: &mut AsyncWindowContext) {
    shot_window(name, None, cx).await
}

pub(crate) async fn shot_window(name: &str, title: Option<&str>, cx: &mut AsyncWindowContext) {
    let Some(dir) = std::env::var_os("FOREMAN_SHOTS").map(PathBuf::from) else {
        return;
    };
    sleep(cx, Duration::from_millis(300)).await;
    match capture(dir.join(format!("{name}.png")), title, cx).await {
        Ok(out) => println!("shot: {}", out.display()),
        Err(e) => println!("shot: {name} failed: {e:#}"),
    }
}

/// A PNG of this process's main window, or its window titled `title`, as it is on screen.
pub(crate) async fn capture(out: PathBuf, title: Option<&str>, cx: &mut AsyncWindowContext) -> anyhow::Result<PathBuf> {
    let title = title.map(str::to_string);
    let script = cargo_root().join("scripts").join("shot.ps1");
    let pid = std::process::id().to_string();
    cx.background_executor()
        .spawn(async move {
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let o = foreman_core::process::run(
                foreman_core::process::command("pwsh")
                    .args(["-NoLogo", "-NoProfile", "-File"])
                    .arg(&script)
                    .args(["-ProcessId", &pid, "-Out"])
                    .arg(&out)
                    .args(title.iter().flat_map(|t| ["-Title", t.as_str()])),
                Duration::from_secs(60),
                foreman_core::process::Kill::Tree,
            )?;
            if !o.success() {
                bail!("{} failed: {}", out.display(), o.stderr.trim());
            }
            Ok(out)
        })
        .await
}

pub(crate) async fn menu_open(view: &Entity<FleetView>, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<&'static str>> {
    let keys = until(view, cx, WITHIN, &format!("the menu of {title}"), |v| {
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
    until(view, cx, WITHIN, &format!("{first} painted"), |v| painted(v, &first).then_some(())).await?;
    Ok(keys)
}

/// [`press`] once `key` is painted (a row can be missing from the frame being drawn).
pub(crate) async fn press_painted(view: &Entity<FleetView>, key: &str, button: MouseButton, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    until(view, cx, WITHIN, &format!("{key} painted"), |v| painted(v, key).then_some(())).await.map_err(|e| {
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

    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-from-folder", cx).await?;
    let repos = until(view, cx, WITHIN, "the review's repos", |v| {
        let d = v.adding.as_ref().filter(|d| !d.scanning)?;
        Some(d.repos.iter().map(|(r, _)| r.name.clone()).collect::<Vec<_>>())
    })
    .await?;
    let name = view.read_with(cx, |v, cx| v.adding.as_ref().map(|d| d.name_text(cx)).unwrap_or_default());
    if name != FOLDER || repos != ["alpha", "beta"] {
        bail!("the review offers name {name} with repos {repos:?}, expected {FOLDER} with [alpha, beta]");
    }
    until(view, cx, WITHIN, "Save painted", |v| painted(v, "add-save").then_some(())).await?;
    click(view, "add-repo-beta", cx).await?;
    until(view, cx, WITHIN, "beta unticked", |v| v.adding.as_ref().is_some_and(|d| !d.repos[1].1).then_some(())).await?;
    let name_edit = edit_name(view, cx).await?;
    shot("add-review", cx).await;
    let hold = view.read_with(cx, |v, _| v.poll_hold.clone());
    hold.armed.store(true, Ordering::SeqCst);
    until(view, cx, WITHIN, "a poll held after its registry read", |_| hold.holding.load(Ordering::SeqCst).then_some(())).await?;
    post_keys(&["enter"], cx).await?;
    until(view, cx, WITHIN, "the review closed on Enter", |v| v.adding.is_none().then_some(())).await?;
    let saved = registry.load(ADDED).context("the registry after Save")?;
    let flags: Vec<(String, bool)> = saved.folders.iter().map(|r| (r.name.clone(), r.enabled)).collect();
    if flags != [("alpha".to_string(), true), ("beta".to_string(), false)] {
        bail!("registered {ADDED} with repos {flags:?}, expected alpha enabled and beta disabled");
    }
    let alpha = format!("repo-row-{ADDED}/alpha");
    let in_tree = |v: &FleetView, repo: &str| v.tree().workspaces.iter().find(|w| w.name == ADDED).is_some_and(|w| w.repos.iter().any(|r| r.name == repo));
    until(view, cx, WITHIN, "alpha in the tree, the disabled beta not", |v| (painted(v, &alpha) && !in_tree(v, "beta")).then_some(())).await?;
    let applied = view.read_with(cx, |v, _| v.refreshes);
    hold.holding.store(false, Ordering::SeqCst);
    until(view, cx, WITHIN, "the held poll applied", |v| (v.refreshes > applied).then_some(())).await?;
    if !view.read_with(cx, |v, _| v.workspaces.iter().any(|w| w.name == ADDED)) {
        bail!("a poll that read the registry before Save took {ADDED} out of the tree when it applied");
    }
    let mut out = vec![format!("added {ADDED} from {} (alpha enabled, beta disabled), only alpha in the tree", root.display()), name_edit];
    out.push(cut_label_tooltip(view, cx).await?);

    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-from-folder", cx).await?;
    until(view, cx, WITHIN, "the second review", |v| v.adding.as_ref().filter(|d| !d.scanning).map(|_| ())).await?;
    until(view, cx, WITHIN, "Save painted", |v| painted(v, "add-save").then_some(())).await?;
    let again = view.read_with(cx, |v, cx| v.adding.as_ref().map(|d| d.name_text(cx))).unwrap_or_default();
    click(view, "add-save", cx).await?;
    until(view, cx, WITHIN, "the second review closed on Save", |v| v.adding.is_none().then_some(())).await?;
    let second = registry.load(&again).with_context(|| format!("the same folder again, saved as {again}"))?;
    let flags: Vec<(String, bool)> = second.folders.iter().map(|r| (r.name.clone(), r.enabled)).collect();
    if flags != [("alpha".to_string(), false), ("beta".to_string(), true)] {
        bail!("the same folder again as {again}: folders {flags:?}, expected alpha disabled (supervised in {ADDED}) and beta enabled");
    }
    let in_again = |v: &FleetView, repo: &str| v.tree().workspaces.iter().find(|w| w.name == again).is_some_and(|w| w.repos.iter().any(|r| r.name == repo));
    until(view, cx, WITHIN, "beta in the second workspace, alpha (supervised in the first) not", |v| {
        (painted(v, &format!("repo-row-{again}/beta")) && !in_again(v, "alpha") && !painted(v, &format!("supervised-in-{again}/alpha"))).then_some(())
    })
    .await?;
    view.update(cx, |v, cx| v.search.update(cx, |s, cx| s.set_text("alpha", cx)));
    let found = format!("result-supervised-in-{again}/alpha");
    until(view, cx, WITHIN, "search finds alpha in the second workspace, supervised in the first", |v| painted(v, &found).then_some(())).await?;
    view.update(cx, |v, cx| v.clear_search(cx));
    registry.remove(&again)?;
    until(view, cx, Duration::from_secs(15), "the second workspace gone", |v| (!v.workspaces.iter().any(|w| w.name == again)).then_some(())).await?;
    out.push(format!("the same folder again registers as {again} with alpha disabled, out of its tree and found by search as supervised in {ADDED}; removed"));
    out.push(single_repo(view, &registry, cx).await?);
    out.push(hover_cards(view, own, cx).await?);

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
    until(view, cx, WITHIN, "the session menu closed on Esc", |v| v.menu.is_none().then_some(())).await?;
    out.push(format!("session menu {keys:?} closed by Esc"));

    let ws_key = format!("ws-row-{ADDED}");
    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    let keys = menu_open(view, ADDED, cx).await?;
    if keys != ["start", "work", "rescan", "edit-folders", "open-folder", "open-terminal", "remove"] {
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
    until(view, cx, WITHIN, "the workspace menu closed by a click outside", |v| v.menu.is_none().then_some(())).await?;
    out.push(format!("workspace menu {keys:?} closed by a click outside"));

    // a menu closed but still painted takes the next press as a click outside it
    until(view, cx, WITHIN, "a frame without the closed menu", |v| (!painted(v, "menu-remove")).then_some(())).await?;
    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    menu_open(view, ADDED, cx).await?;
    click(view, "menu-edit-folders", cx).await?;
    until(view, cx, WITHIN, "Edit folders… with beta's box", |v| painted(v, "edit-enable-beta").then_some(())).await?;
    shot("edit-folders", cx).await;
    click(view, "edit-enable-beta", cx).await?;
    until(view, cx, WITHIN, "beta enabled in the tree", |v| {
        v.tree().workspaces.iter().find(|w| w.name == ADDED)?.repos.iter().find(|r| r.name == "beta" && r.enabled).map(|_| ())
    })
    .await?;
    // painted: the next press must see this frame's layout, not the one before Enable
    let plus = format!("new-claude-{ADDED}/beta");
    until(view, cx, WITHIN, "beta's + painted", |v| painted(v, &plus).then_some(())).await?;
    if !registry.load(ADDED)?.folders.iter().all(|r| r.enabled) {
        bail!("Enable did not reach the registry");
    }
    click(view, "edit-done", cx).await?;
    until(view, cx, WITHIN, "Edit folders… closed by Done", |v| v.editing.is_none().then_some(())).await?;
    out.push("beta enabled in Edit folders…, then in the tree with its +".into());

    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    menu_open(view, ADDED, cx).await?;
    click(view, "menu-remove", cx).await?;
    until(view, cx, WITHIN, "the Remove question", |v| painted(v, "confirm-yes").then_some(())).await?;
    let question = view.read_with(cx, |v, _| v.confirm.as_ref().map(|c| c.text.clone())).unwrap_or_default();
    if !question.contains("No files are deleted") {
        bail!("the Remove question reads: {question}");
    }
    shot("confirm-remove", cx).await;
    click(view, "confirm-yes", cx).await?;
    until(view, cx, WITHIN, "the workspace gone from the tree", |v| {
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

//! `--selftest` step 7b: Add Folder, a top-level folder's row, menus and Details driven through the window procedure; FleetView is not buildable in a gpui test.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Mutex,
    thread::ThreadId,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail};
use supraman_claude::Session;
use gpui::{
    AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, SharedString,
    point, px,
};

use crate::{
    selftest::{cargo_root, click, press, sleep, until, until_app, win},
    view::{FleetView, MenuAction},
};

pub const FOLDER: &str = "selftest-add";
const WITHIN: Duration = Duration::from_secs(5);

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
}

fn typed(cx: &mut AsyncWindowContext, key: &str, key_char: Option<&str>) -> anyhow::Result<()> {
    let keystroke = Keystroke { modifiers: Modifiers::default(), key: key.into(), key_char: key_char.map(Into::into) };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx);
    })?;
    Ok(())
}

fn post(cx: &mut AsyncWindowContext, messages: &[(u32, usize, isize)]) -> anyhow::Result<()> {
    let hwnd = cx.update(|window, _| crate::tray::hwnd(window))?.ok_or_else(|| anyhow!("no HWND"))?;
    win::post(hwnd, messages)
}

async fn post_click(view: &Entity<FleetView>, key: &str, dx: f32, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let bounds = crate::selftest::scrolled_into_view(view, key, cx).await?;
    let scale = cx.update(|window, _| window.scale_factor())?;
    let x = ((f32::from(bounds.left()) + dx) * scale) as isize;
    let y = (f32::from(bounds.center().y) * scale) as isize;
    post(cx, &win::click_messages(x, y))?;
    sleep(cx, Duration::from_millis(150)).await;
    Ok(())
}

/// A key that types goes on to `TranslateMessage`'s WM_CHAR: read what it typed with [`until_app`], never right after this returns.
pub(crate) async fn post_keys(keys: &[&str], cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    // a held modifier would turn End into Alt+End
    let stale = win::held_modifiers();
    let caps = win::caps_lock_on();
    if !stale.is_empty() || caps {
        let live = win::pressed_modifiers();
        win::clear_key_state()?;
        if !stale.is_empty() {
            println!("keys: {stale:?} held in the UI thread's key state (held on the keyboard: {live:?}); cleared there before posting {keys:?}");
        }
        if caps {
            println!("keys: Caps Lock on in the UI thread's key state; turned off there before posting {keys:?}");
        }
    }
    let started = Instant::now();
    while !win::held_modifiers().is_empty() {
        if started.elapsed() > WITHIN {
            bail!("{:?} held in this thread's key state for {} s: posted keys would carry it (another process's input?)", win::held_modifiers(), WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    for key in keys {
        post(cx, &win::key_messages(key))?;
    }
    sleep(cx, Duration::from_millis(150)).await;
    Ok(())
}

/// Resting on each kind of row shows no card but its icon's tooltip; Details opens the row's Details tab, then focuses it.
async fn row_details(view: &Entity<FleetView>, own: &Session, root: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let folder_row = format!("repo-row-{FOLDER}");
    let session_row = format!("session-row-{}", own.session_id);
    let rows = [(folder_row.clone(), format!("work-{FOLDER}")), (session_row.clone(), format!("star-{}", own.session_id))];
    for (row, icon) in &rows {
        no_card_on(view, row, icon, cx).await?;
    }

    let home = view.read_with(cx, |v, _| v.home.clone()).ok_or_else(|| anyhow!("no Supraman home"))?;
    let (folder_tab, page) = details_tab(view, &folder_row, FOLDER, cx).await?;
    let path = format!("\nPath  {}\n", supraman_core::paths::normalize(root));
    for want in [format!("{FOLDER}\n"), "\nFolder\n".into(), format!("\nName  {FOLDER}\n"), path] {
        if !format!("{page}\n").contains(&want) {
            bail!("the Details tab of {FOLDER} reads {page:?}, without {want:?}");
        }
    }
    crate::selftest_schedules::themed_shots(&home, "sched2-5-details-folder", cx).await;
    let (session_tab, session) = details_tab(view, &session_row, &own.name, cx).await?;
    let name = format!("\nName  {}\n", own.name);
    let cwd = format!("\nWorking directory  {}\n", supraman_core::paths::normalize(&own.cwd));
    let folder_line = session.lines().find(|l| l.starts_with("Folder  ")).unwrap_or_default().to_string();
    if !session.contains(&name) || !session.contains(&cwd) || folder_line.is_empty() || folder_line.contains("no listed folder") || !session.contains("\nState  ") {
        bail!("the Details tab of {} reads {session:?}, not its name, its folder and {cwd:?}", own.name);
    }
    if own.id != own.session_id && !session.contains(&format!("\nUUID  {}\n", own.session_id)) {
        bail!("the Details tab of {} reads {session:?}, without its UUID", own.name);
    }
    crate::selftest_schedules::themed_shots(&home, "sched2-5-details-session", cx).await;
    let tabs = view.read_with(cx, |v, _| v.tabs.len());
    let (again, _) = details_tab(view, &session_row, &own.name, cx).await?;
    if again != session_tab || view.read_with(cx, |v, _| v.tabs.len()) != tabs {
        bail!("a second Details of {} opened tab {again} ({} tabs), not its tab {session_tab} again ({tabs})", own.name, view.read_with(cx, |v, _| v.tabs.len()));
    }
    let starred = view.read_with(cx, |v, _| v.stars.ids().contains(&own.session_id));
    let star = format!("details-session-{}-star", own.session_id);
    for want in [!starred, starred] {
        crate::selftest::scrolled_into_view(view, &star, cx).await?;
        click(view, &star, cx).await?;
        until(view, cx, WITHIN, &format!("the page's Star made {} starred {want}", own.name), |v| (v.stars.ids().contains(&own.session_id) == want).then_some(())).await?;
        let label = if want { "Unstar" } else { "Star" };
        until(view, cx, Duration::from_secs(15), &format!("the page's action reads {label} after a poll"), |v| {
            let page = v.tab(session_tab).and_then(|t| match &t.kind { crate::view::TabKind::Details(p) => p.seen.as_ref(), _ => None })?;
            (page.starred == want && painted(v, &star)).then_some(())
        })
        .await?;
    }
    if let Some(card) = supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take()) {
        bail!("a hover card showed during the Details checks: {card:?}");
    }
    view.update(cx, |v, cx| {
        v.close_tab(session_tab, cx);
        v.close_tab(folder_tab, cx);
    });
    Ok(format!(
        "no card on {FOLDER}'s row or {}'s row, their icons' tooltips shown; Details opened tab {folder_tab}: {:?}; tab {session_tab}: {:?}; a second Details focused tab {session_tab}; its Star starred and unstarred it",
        own.name,
        page.replace('\n', " | "),
        session.replace('\n', " | ")
    ))
}

/// Details in the row's menu opens or focuses a Details tab; its id and text once it is the active tab.
async fn details_tab(view: &Entity<FleetView>, row: &str, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<(u64, String)> {
    press_painted(view, row, MouseButton::Right, cx).await?;
    let keys = menu_open(view, title, cx).await?;
    if !keys.iter().any(|k| k == "details") {
        bail!("the menu of {title} has {keys:?}, no details");
    }
    click(view, "menu-details", cx).await?;
    let found = until_app(view, cx, WITHIN, &format!("the Details tab of {title}"), |v, cx| {
        let tab = v.active.and_then(|id| v.tab(id))?;
        let crate::view::TabKind::Details(page) = &tab.kind else { return None };
        let text = v.details_text(&page.target, cx)?;
        (v.dialogs.confirm.is_none() && text.starts_with(title)).then_some((tab.id, text))
    })
    .await?;
    Ok(found)
}

/// Resting off the row's icons and labels past the tooltip delay shows no card; resting on `icon` shows its tooltip.
async fn no_card_on(view: &Entity<FleetView>, row: &str, icon: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let bounds = crate::selftest::scrolled_into_view(view, row, cx).await?;
    let position = point(bounds.left() + px(4.), bounds.top() + px(6.));
    crate::selftest_left::move_to(point(px(1.), px(1.)), cx)?;
    sleep(cx, Duration::from_millis(100)).await;
    supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take());
    let started = Instant::now();
    // past the card's 500 ms delay; no gpui test builds FleetView's sidebar, so only the window can show a card's absence
    while started.elapsed() < Duration::from_millis(800) {
        crate::selftest_left::move_to(position, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
    }
    if let Some(card) = supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take()) {
        bail!("resting on {row} at {position:?} showed the card {card:?}");
    }
    let at = crate::selftest::scrolled_into_view(view, icon, cx).await?.center();
    supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow_mut().take());
    let started = Instant::now();
    loop {
        crate::selftest_left::move_to(at, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
        if supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow().is_some()) {
            break;
        }
        if started.elapsed() > WITHIN {
            bail!("resting on {icon} at {at:?} showed no tooltip");
        }
    }
    crate::selftest_left::move_to(point(px(1.), px(1.)), cx)?;
    if let Some(card) = supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take()) {
        bail!("resting on {icon} showed the card {card:?}");
    }
    Ok(())
}

pub(crate) fn fake_repo(root: &Path, name: &str) -> anyhow::Result<()> {
    let git = root.join(name).join(".git");
    std::fs::create_dir_all(&git)?;
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    Ok(())
}

/// `SUPRAMAN_SHOTS` is set: the run takes screenshots, and the rests that only serve them are taken too.
pub(crate) fn shooting() -> bool {
    crate::selftest_shots::dir().is_some()
}

pub(crate) async fn shot(name: &str, cx: &mut AsyncWindowContext) {
    shot_window(name, None, cx).await
}

pub(crate) async fn shot_window(name: &str, title: Option<&str>, cx: &mut AsyncWindowContext) {
    let Some(dir) = crate::selftest_shots::dir() else {
        return;
    };
    sleep(cx, Duration::from_millis(300)).await;
    match capture(dir.join(format!("{name}.png")), title, cx).await {
        Ok(out) => println!("shot: {}", out.display()),
        Err(e) => println!("shot: {name} failed: {e:#}"),
    }
}

/// A PNG of this process's main window, or its window titled `title`, as the window paints itself.
pub(crate) async fn capture(out: PathBuf, title: Option<&str>, cx: &mut AsyncWindowContext) -> anyhow::Result<PathBuf> {
    let title = title.map(str::to_string);
    cx.background_executor()
        .spawn(async move {
            let hwnd = crate::selftest::win::window(title.as_deref())?;
            crate::selftest::win::print_window(hwnd, &out)?;
            Ok(out)
        })
        .await
}

pub(crate) async fn menu_open(view: &Entity<FleetView>, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<SharedString>> {
    let keys = until(view, cx, WITHIN, &format!("the menu of {title}"), |v| {
        v.dialogs.menu.as_ref().filter(|m| m.title == title).map(|m| m.items.iter().map(|i| i.key.clone()).collect::<Vec<_>>())
    })
    .await
    .map_err(|e| {
        let state = view.read_with(cx, |v, _| {
            format!("menu {:?}, confirm {:?}", v.dialogs.menu.as_ref().map(|m| m.title.clone()), v.dialogs.confirm.as_ref().map(|c| c.text.clone()))
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
            let mut keys: Vec<String> = v.hits.borrow().keys().filter(|k| k.starts_with("group-row-") || k.starts_with("repo-row-")).cloned().collect();
            keys.sort();
            (keys, v.tree().entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>())
        });
        e.context(format!("painted rows {keys:?}; entries {rows:?}"))
    })?;
    // a menu closed just now still occludes the row in the last frame's hitboxes
    crate::selftest::drawn(cx).await?;
    press(view, key, button, cx).await
}

static OPENED_ON: Mutex<Option<ThreadId>> = Mutex::new(None);

/// Dispatches this thread's messages first, as ShellExecuteW does while Explorer answers, then opens for real.
fn pumping_open(path: &Path) -> Result<(), String> {
    *OPENED_ON.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current().id());
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until {
        win::pump_messages();
        std::thread::sleep(Duration::from_millis(5));
    }
    supraman_ui::open::open(path.as_os_str())
}

/// Open folder on a missing folder while foreground tasks arrive: the refusal comes back as the action error.
async fn open_folder(view: &Entity<FleetView>, row: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let missing = cargo_root().join("target").join("selftest-open-folder-missing");
    let _ = std::fs::remove_dir_all(&missing);
    press_painted(view, row, MouseButton::Right, cx).await?;
    menu_open(view, FOLDER, cx).await?;
    let real = view.update(cx, |v, cx| {
        for item in v.dialogs.menu.iter_mut().flat_map(|m| m.items.iter_mut()).filter(|i| i.key == "open-folder") {
            item.action = MenuAction::OpenFolder(missing.clone());
        }
        v.action_error = None;
        cx.notify();
        std::mem::replace(&mut v.open_with, pumping_open)
    });
    // the click reaches the closures of the next frame
    crate::selftest::drawn(cx).await?;

    let (ticks, stop) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(false)));
    let busy = (view.clone(), ticks.clone(), stop.clone());
    let foreground = cx.spawn(async move |cx| {
        while !busy.2.get() {
            cx.background_executor().timer(Duration::from_millis(1)).await;
            busy.0.read_with(cx, |_, _| ());
            busy.1.set(busy.1.get() + 1);
        }
    });
    post_click(view, "menu-open-folder", 4., cx).await?;
    let want = format!("Cannot open {}: ShellExecuteW returned 2", missing.display());
    let error = until(view, cx, WITHIN, "Open folder's refusal as the action error", |v| v.action_error.clone()).await;
    stop.set(true);
    foreground.await;
    let error = error?;
    view.update(cx, |v, _| {
        v.open_with = real;
        v.action_error = None;
    });
    if error != want {
        bail!("Open folder of a missing folder reads {error:?}, expected {want:?}");
    }
    let opened_on = OPENED_ON.lock().unwrap_or_else(|e| e.into_inner()).take();
    let ui = std::thread::current().id();
    if opened_on.is_none_or(|t| t == ui) {
        bail!("Open folder ran its ShellExecuteW on {opened_on:?}, the UI thread is {ui:?}");
    }
    Ok(format!(
        "Open folder in the folder menu, its ShellExecuteW pumping messages off the UI thread ({opened_on:?}, UI {ui:?}) with {} foreground tasks run meanwhile: alive, \"{error}\"",
        ticks.get()
    ))
}

pub async fn run(view: &Entity<FleetView>, own: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let root = cargo_root().join("target").join(FOLDER);
    fake_repo(&cargo_root().join("target"), FOLDER)?;
    let home = view.read_with(cx, |v, _| v.home.clone()).ok_or_else(|| anyhow!("no Supraman home"))?;

    view.update(cx, |v, _| v.dialogs.pick_folder = Some(root.clone()));
    click(view, "add-folder", cx).await?;
    let folder_key = format!("repo-row-{FOLDER}");
    until(view, cx, WITHIN, "the added folder's row at the top level", |v| painted(v, &folder_key).then_some(())).await?;
    let listed = supraman_core::folders::load(&home)?;
    let top = listed.list.last().and_then(|e| match e {
        supraman_core::folders::Entry::Folder(f) => Some(f.path.clone()),
        supraman_core::folders::Entry::Group { .. } => None,
    });
    if top.as_deref() != Some(root.as_path()) {
        bail!("Add Folder of {} left folders.toml's last entry {:?}", root.display(), listed.list.last());
    }
    let toast = view.read_with(cx, |v, _| v.toast.text().map(str::to_string));
    let mut out = vec![format!("Add Folder put {} at the top level's end, its row painted; toast {toast:?}", root.display())];
    view.update(cx, |v, _| v.dialogs.pick_folder = Some(root.clone()));
    click(view, "add-folder", cx).await?;
    let refused = until(view, cx, WITHIN, "the second Add Folder of it refused", |v| v.action_error.clone()).await?;
    if !refused.contains("listed there already") {
        bail!("adding {} twice reads {refused:?}", root.display());
    }
    view.update(cx, |v, _| v.action_error = None);
    out.push(format!("the same folder again: {refused:?}"));

    out.push(row_details(view, own, &root, cx).await?);

    let session_key = format!("session-row-{}", own.session_id);
    press_painted(view, &session_key, MouseButton::Right, cx).await?;
    let keys = menu_open(view, &own.name, cx).await?;
    shot("menu-session", cx).await;
    typed(cx, "escape", None)?;
    until(view, cx, WITHIN, "the session menu closed on Esc", |v| v.dialogs.menu.is_none().then_some(())).await?;
    out.push(format!("session menu {keys:?} closed by Esc"));

    press_painted(view, &folder_key, MouseButton::Right, cx).await?;
    let keys = menu_open(view, FOLDER, cx).await?;
    shot("menu-folder", cx).await;
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
    until(view, cx, WITHIN, "the folder menu closed by a click outside", |v| v.dialogs.menu.is_none().then_some(())).await?;
    out.push(format!("folder menu {keys:?} closed by a click outside"));
    // a menu closed but still painted takes the next press as a click outside it
    until(view, cx, WITHIN, "a frame without the closed menu", |v| (!painted(v, "menu-open-folder")).then_some(())).await?;
    out.push(open_folder(view, &folder_key, cx).await?);

    let removed = root.clone();
    let task = cx.background_executor().spawn(async move {
        supraman_core::folders::edit(&home, |e| {
            let at = e.list.iter().position(|x| matches!(x, supraman_core::folders::Entry::Folder(f) if f.path == removed)).ok_or_else(|| supraman_core::CoreError::NoEntry("selftest-add".into()))?;
            e.remove_folder(supraman_core::folders::Pos::top(at))
        })
    });
    task.await?;
    until(view, cx, Duration::from_secs(15), "the added folder gone after the next read", |v| (!painted(v, &folder_key)).then_some(())).await?;
    if !root.join(".git").join("HEAD").is_file() {
        bail!("taking {} out of folders.toml touched the folder", root.display());
    }
    out.push(format!("taken out of folders.toml; {} still on disk", root.display()));
    Ok(out.join("; "))
}

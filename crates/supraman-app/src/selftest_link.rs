use std::sync::Mutex;
use std::time::Duration;

use anyhow::bail;
use gpui::{
    App, AsyncWindowContext, Entity, Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, PlatformInput, Point, point,
};
use supraman_term::Terminal;
use supraman_ui::term_element;

use crate::selftest::{sleep, until_with};
use crate::view::FleetView;
use supraman_ui::open::Link;

const WITHIN: Duration = Duration::from_secs(10);
const URL: &str = "https://example.com/sched2-11?a=1&b=2";
const TEXT: &str = "the text";
const REFUSED: &str = "ms-settings:privacy";

static OPENED: Mutex<Vec<Link>> = Mutex::new(Vec::new());

fn recording_open(link: &Link, _: &mut App) {
    OPENED.lock().unwrap_or_else(|e| e.into_inner()).push(link.clone());
}

fn opened() -> Vec<Link> {
    OPENED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Step 6d: a pwsh tab prints an OSC 8 link; it is underlined, its URL shows on hover, Ctrl turns the pointer to
/// a hand, Ctrl+click opens exactly its URI (through `open_link_with`), and a `ms-settings:` link is refused.
pub(crate) async fn run(view: &Entity<FleetView>, tab: u64, terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    OPENED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let real = view.update(cx, |v, _| std::mem::replace(&mut v.open_link_with, recording_open));
    term_element::record_painted_glyphs(true);
    let result = steps(view, tab, terminal, cx).await;
    term_element::record_painted_glyphs(false);
    cx.update(|window, cx| window.dispatch_event(PlatformInput::ModifiersChanged(ModifiersChangedEvent::default()), cx)).ok();
    view.update(cx, |v, _| v.open_link_with = real);
    result
}

/// The display cells (line, column) whose OSC 8 URI is `uri`.
fn link_cells(terminal: &Entity<Terminal>, uri: &str, cx: &mut AsyncWindowContext) -> Vec<(i32, usize)> {
    terminal.read_with(cx, |t, _| {
        let content = t.last_content();
        let offset = content.display_offset as i32;
        content
            .cells
            .iter()
            .filter(|c| c.cell.hyperlink().is_some_and(|h| h.uri() == uri))
            .map(|c| (c.point.line + offset, c.point.column))
            .collect()
    })
}

fn cell_center(terminal: &Entity<Terminal>, (line, column): (i32, usize), cx: &mut AsyncWindowContext) -> Point<Pixels> {
    terminal.read_with(cx, |t, _| {
        let b = t.last_content().terminal_bounds;
        point(
            b.bounds.origin.x + b.cell_width * (column as f32 + 0.5),
            b.bounds.origin.y + b.line_height * (line as f32 + 0.5),
        )
    })
}

fn input(event: PlatformInput, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, cx| {
        window.dispatch_event(event, cx);
    })
}

fn move_to(position: Point<Pixels>, modifiers: Modifiers, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    input(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers }), cx)
}

fn ctrl(on: bool, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let modifiers = if on { Modifiers::control() } else { Modifiers::default() };
    input(PlatformInput::ModifiersChanged(ModifiersChangedEvent { modifiers, capslock: Default::default() }), cx)
}

fn click(position: Point<Pixels>, modifiers: Modifiers, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let button = MouseButton::Left;
    input(PlatformInput::MouseDown(MouseDownEvent { button, position, modifiers, click_count: 1, first_mouse: false }), cx)?;
    input(PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count: 1 }), cx)
}

async fn steps(view: &Entity<FleetView>, tab: u64, terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    terminal.update(cx, |t, _| t.scroll_to_bottom());
    // step 6c leaves pasted text at the prompt; Ctrl+C drops it
    let before = terminal.read_with(cx, |t, _| t.get_content().matches("PS ").count());
    terminal.update(cx, |t, _| t.input(b"\x03".to_vec()));
    until_with(cx, WITHIN, "a fresh prompt after Ctrl+C", |cx| {
        let content = terminal.read_with(cx, |t, _| t.get_content());
        (content.matches("PS ").count() > before && content.trim_end().ends_with('>')).then_some(())
    })
    .await?;
    let command = format!(
        "$e=[char]27; Write-Host \"osc8 ${{e}}]8;;{URL}${{e}}\\{TEXT}${{e}}]8;;${{e}}\\ end\"; Write-Host \"osc8 ${{e}}]8;;{REFUSED}${{e}}\\no text${{e}}]8;;${{e}}\\ end\"\r"
    );
    terminal.update(cx, |t, _| t.input(command.into_bytes()));
    let what = format!("the {} cells of {TEXT:?} linked to {URL} and a {REFUSED} link", TEXT.len());
    let found = until_with(cx, WITHIN, &what, |cx| {
        let (link, refused) = (link_cells(terminal, URL, cx), link_cells(terminal, REFUSED, cx));
        (link.len() == TEXT.len() && !refused.is_empty()).then_some((link, refused))
    })
    .await;
    let (link, refused) = match found {
        Ok(found) => found,
        Err(e) => {
            let (uris, tail) = terminal.read_with(cx, |t, _| {
                let mut uris: Vec<String> = t.last_content().cells.iter().filter_map(|c| c.cell.hyperlink().map(|h| h.uri().to_owned())).collect();
                uris.dedup();
                let content = t.get_content();
                let tail: Vec<String> = content.lines().filter(|l| !l.trim().is_empty()).rev().take(4).map(str::to_owned).collect();
                (uris, tail)
            });
            bail!("{e}; links on screen {uris:?}; last lines {tail:?}");
        }
    };

    let underlined = until_with(cx, WITHIN, "the link's glyphs painted underlined", |_| {
        let glyphs = term_element::painted_glyphs();
        let of_link: Vec<_> = glyphs.iter().filter(|g| link.contains(&(g.line, g.column as usize)) && g.ch != ' ').collect();
        (!of_link.is_empty() && of_link.iter().all(|g| g.underlined)).then_some(of_link.len())
    })
    .await?;

    // a plain click on the prompt line gives the window the keys, so Ctrl reaches it
    let (line, _) = link[0];
    let off_link = cell_center(terminal, (line, 0), cx);
    click(off_link, Modifiers::default(), cx)?;
    let on_link = cell_center(terminal, link[2], cx);
    let hint = format!("terminal-link-{tab}");
    move_to(on_link, Modifiers::default(), cx)?;
    until_with(cx, WITHIN, &format!("hovering {TEXT:?} shows {URL}"), |cx| {
        // the prompt may redraw after the click and move the link: point at where it is now
        if let Some(&cell) = link_cells(terminal, URL, cx).get(2) {
            let _ = move_to(cell_center(terminal, cell, cx), Modifiers::default(), cx);
        }
        let shown = view.read_with(cx, |v, _| v.hits.borrow().contains_key(&hint));
        let link = terminal.read_with(cx, |t, _| t.hovered_link().map(str::to_owned));
        (shown && link.as_deref() == Some(URL)).then_some(())
    })
    .await?;
    if terminal.read_with(cx, |t, _| t.hand_painted) {
        bail!("the pointer is a hand over the link without Ctrl");
    }
    move_to(off_link, Modifiers::default(), cx)?;
    until_with(cx, WITHIN, "the URL hint gone off the link", |cx| {
        let shown = view.read_with(cx, |v, _| v.hits.borrow().contains_key(&hint));
        (!shown && terminal.read_with(cx, |t, _| t.hovered_link().is_none())).then_some(())
    })
    .await?;

    let now_on_link = |cx: &mut AsyncWindowContext| link_cells(terminal, URL, cx).get(2).map(|&cell| cell_center(terminal, cell, cx)).unwrap_or(on_link);
    let at = now_on_link(cx);
    move_to(at, Modifiers::default(), cx)?;
    sleep(cx, Duration::from_millis(150)).await;
    ctrl(true, cx)?;
    until_with(cx, WITHIN, "Ctrl over the link: a hand", |cx| {
        let _ = move_to(now_on_link(cx), Modifiers::control(), cx);
        terminal.read_with(cx, |t, _| t.hand_painted.then_some(()))
    })
    .await?;
    let at = now_on_link(cx);
    click(at, Modifiers::control(), cx)?;
    until_with(cx, WITHIN, &format!("Ctrl+click opens {URL}"), |_| (!opened().is_empty()).then_some(())).await?;
    if opened() != [Link::Url(URL.into())] {
        bail!("Ctrl+click opened {:?}, not exactly {URL}", opened());
    }
    ctrl(false, cx)?;
    until_with(cx, WITHIN, "Ctrl released: no hand", |cx| terminal.read_with(cx, |t, _| (!t.hand_painted).then_some(()))).await?;

    let refused_now = link_cells(terminal, REFUSED, cx).get(1).copied().unwrap_or(refused[1]);
    let at = cell_center(terminal, refused_now, cx);
    ctrl(true, cx)?;
    move_to(at, Modifiers::control(), cx)?;
    until_with(cx, WITHIN, "Ctrl over the refused link: a hand", |cx| terminal.read_with(cx, |t, _| t.hand_painted.then_some(()))).await?;
    click(at, Modifiers::control(), cx)?;
    let why = supraman_ui::open::refused("ms-settings");
    until_with(cx, WITHIN, &format!("the toast {why:?}"), |cx| {
        view.read_with(cx, |v, _| (v.pane_toast.text() == Some(why.as_str())).then_some(()))
    })
    .await?;
    sleep(cx, Duration::from_millis(200)).await;
    if opened().len() != 1 {
        bail!("the {REFUSED} link reached the opener: {:?}", opened());
    }
    ctrl(false, cx)?;
    Ok(format!(
        "OSC 8 {TEXT:?} → {URL}: {underlined} glyphs underlined; hover shows the URL, Ctrl shows a hand; Ctrl+click opened exactly it; {REFUSED} refused: {why:?}"
    ))
}

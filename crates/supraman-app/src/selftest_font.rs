use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use supraman_core::{Settings, default_terminal_font_family};
use supraman_term::Terminal;
use supraman_ui::term_element::{self, PaintedGlyph, TermFont};
use gpui::{AsyncWindowContext, Entity, TextRun, px};

use crate::{
    selftest::{sleep, wait_for_tab_terminal},
    selftest_manage::{shot, shot_window},
    selftest_zoom::laid_out_with,
    settings_window::{SettingsView, page},
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);
const PRINTED: &str = "◯x a->b";
const TRIANGLES: &str = "⏵⏵ auto mode on";
const CASCADIA: &str = "Cascadia Code";

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, home, cx).await;
    view.update(cx, |v, _| v.shell_args = saved);
    term_element::record_painted_glyphs(false);
    // whatever failed, the next run starts at the default
    view.update(cx, |v, cx| v.set_term_font(None, true, cx)).ok();
    view.update(cx, |v, cx| v.close_settings(cx));
    close_all(view, cx);
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    close_all(view, cx);
    let (start, installed) = view.read_with(cx, |v, _| (v.term_font(), v.installed_fonts.clone()));
    let default = default_terminal_font_family(&installed);
    if start.family != default || !start.ligatures {
        bail!("the terminal font starts as {start:?}, not {default} with ligatures");
    }
    let tab = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let terminal = wait_for_tab_terminal(view, tab, WITHIN, cx).await?;
    laid_out_with(view, tab, &terminal, &start, cx).await?;
    terminal.update(cx, |t, _| t.input(b"Write-Host ([char]0x25EF + 'x a-' + '>b')\r".to_vec()));
    let samples = "Write-Host ([char]0x23F5 + [char]0x23F5 + ' auto mode on'); Write-Host ([char]0x25EF + 'general-purpose'); \
                   Write-Host ('a ' + [char]0x2192 + ' b'); Write-Host ([char]0x2265 + ' ' + [char]0x2264)\r";
    terminal.update(cx, |t, _| t.input(samples.as_bytes().to_vec()));
    term_element::record_painted_glyphs(true);

    let ligated = (start.family == CASCADIA).then_some(true);
    let first = painted_fits(view, &terminal, &start, ligated, cx).await.context(start.family.to_string())?;
    shot("m5-49-term-fallback-glyphs", cx).await;
    let triangles = triangles_fit(view, &terminal, &start, cx).await.context(start.family.to_string())?;
    shot("m5-47-term-default-font", cx).await;

    let handle = open_appearance(view, cx).await?;
    let mono = mono_fonts(&handle, cx).await?;
    shot_window("m5-47-settings-terminal-font", Some("Supraman Settings"), cx).await;
    for want in ["Consolas", default] {
        if !mono.iter().any(|f| f == want) {
            bail!("Appearance does not offer {want} among {} monospace fonts", mono.len());
        }
    }
    let before = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds);
    handle.update(cx, |s, _, cx| s.set_term_font(Some("Consolas"), true, cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let consolas = view.read_with(cx, |v, _| v.term_font());
    if consolas.family != "Consolas" {
        bail!("Appearance → Consolas left the terminals in {consolas:?}");
    }
    let ms = laid_out_with(view, tab, &terminal, &consolas, cx).await.context("the open tab after Appearance → Consolas")?;
    let after = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds);
    if (after.cell_width, after.num_columns()) == (before.cell_width, before.num_columns()) {
        bail!("Consolas kept the cell width {:?} and {} columns of {}", after.cell_width, after.num_columns(), start.family);
    }
    if Settings::load(home)?.terminal.font_family.as_deref() != Some("Consolas") {
        bail!("settings.toml does not hold [terminal] font_family = \"Consolas\"");
    }
    let second = painted_fits(view, &terminal, &consolas, None, cx).await.context("Consolas")?;
    shot("m5-47-term-consolas", cx).await;

    handle.update(cx, |s, _, cx| s.set_term_font(Some(&start.family), false, cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let plain = view.read_with(cx, |v, _| v.term_font());
    let third = painted_fits(view, &terminal, &plain, Some(false), cx).await.context("ligatures off")?;

    handle.update(cx, |s, _, cx| s.set_term_font(None, true, cx)).map_err(|e| anyhow!("settings window: {e}"))?;
    let text = std::fs::read_to_string(Settings::path(home)).unwrap_or_default();
    if text.contains("font_family") || text.contains("ligatures") {
        bail!("Default left the font in settings.toml:\n{text}");
    }
    if view.read_with(cx, |v, _| v.term_font()) != start {
        bail!("Default did not put the terminals back in {start:?}");
    }
    Ok(format!(
        "{first}; {triangles}; Appearance → Consolas: cells {:?} → {:?}, {} → {} columns, laid out in {ms} ms, saved; {second}; ligatures off: {third}; Default: unset again",
        before.cell_width,
        after.cell_width,
        before.num_columns(),
        after.num_columns()
    ))
}

async fn painted_fits(view: &Entity<FleetView>, terminal: &Entity<Terminal>, font: &TermFont, ligated: Option<bool>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let started = Instant::now();
    let (line, glyphs, cell) = loop {
        view.update(cx, |_, cx| cx.notify());
        sleep(cx, Duration::from_millis(50)).await;
        let (line, cell) = terminal.read_with(cx, |t, _| (printed_line(t, PRINTED), t.last_content().terminal_bounds.cell_width));
        let glyphs = term_element::painted_glyphs();
        if let Some(line) = line {
            let row: Vec<PaintedGlyph> = glyphs.into_iter().filter(|g| g.line == line).collect();
            let at_size = row.iter().any(|g| g.ch == 'x' && g.font_size == font.size);
            if at_size && ['◯', 'x', '-', '>', 'b'].iter().all(|c| row.iter().any(|g| g.ch == *c)) {
                break (line, row, cell);
            }
        }
        if started.elapsed() > Duration::from_secs(10) {
            bail!("no paint of the `{PRINTED}` row (on screen: {})", line.is_some());
        }
    };
    let glyph = |ch: char| glyphs.iter().find(|g| g.ch == ch).cloned().ok_or_else(|| anyhow!("no `{ch}` glyph on line {line}"));
    let circle = glyph('◯')?;
    let lacks = cx.update(|_, cx| {
        let ts = cx.text_system();
        term_element::drawn_by_fallback(ts, ts.resolve_font(&font.font()), '◯')
    })?;
    let (left, right) = circle.ink.ok_or_else(|| anyhow!("no ink bounds for `◯`"))?;
    let (cell_left, cell_right) = (cell * circle.column as f32, cell * (circle.column + 1) as f32);
    if left < cell_left - px(0.5) || right > cell_right + px(0.5) {
        bail!("`◯` inks {left:?}..{right:?}, outside its cell {cell_left:?}..{cell_right:?} (fitted: {})", circle.fitted);
    }
    if lacks && !circle.fitted {
        bail!("{} has no `◯` but it was not fitted", font.family);
    }
    let on_cell = |g: &PaintedGlyph| -> anyhow::Result<()> {
        let want = cell * g.column as f32;
        if (g.x - want).abs() > px(1.) {
            bail!("`{}` of cell {} is at {:?}, its cell at {want:?}", g.ch, g.column, g.x);
        }
        Ok(())
    };
    let (x, b) = (glyph('x')?, glyph('b')?);
    on_cell(&x)?;
    on_cell(&b)?;
    if x.column != circle.column + 1 || b.column != circle.column + 6 {
        bail!("`x` in cell {} and `b` in cell {}, after `◯` in {}", x.column, b.column, circle.column);
    }
    let painted_arrow = [glyph('-')?.glyph, glyph('>')?.glyph];
    let plain_arrow = cx.update(|window, _| {
        let style = TextRun { len: "a->b".len(), font: gpui::Font { features: term_element::no_ligatures(), ..font.font() }, ..Default::default() };
        let shaped = window.text_system().shape_line("a->b".into(), font.size, &[style], None);
        let ids: Vec<(usize, u32)> = shaped.runs.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.index, g.id.0))).collect();
        [1, 2].map(|i| ids.iter().find(|(at, _)| *at == i).map_or(u32::MAX, |(_, id)| *id))
    })?;
    let is_ligated = painted_arrow != plain_arrow;
    if ligated.is_some_and(|want| want != is_ligated) {
        bail!("`->` glyphs {painted_arrow:?}, without ligatures {plain_arrow:?}: ligated {is_ligated}, expected {ligated:?}");
    }
    Ok(format!(
        "{} {:?} cells: `◯` inks {:.1}..{:.1} of {:.1}..{:.1} (fitted {}, at {:?}), `x` at {:.1}, `b` at {:.1}, `->` ligated {is_ligated}",
        font.family,
        cell,
        f32::from(left),
        f32::from(right),
        f32::from(cell_left),
        f32::from(cell_right),
        circle.fitted,
        circle.font_size,
        f32::from(x.x),
        f32::from(b.x)
    ))
}

async fn triangles_fit(view: &Entity<FleetView>, terminal: &Entity<Terminal>, font: &TermFont, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let started = Instant::now();
    let (row, cell) = loop {
        view.update(cx, |_, cx| cx.notify());
        sleep(cx, Duration::from_millis(50)).await;
        let (line, cell) = terminal.read_with(cx, |t, _| (printed_line(t, TRIANGLES), t.last_content().terminal_bounds.cell_width));
        if let Some(line) = line {
            let mut row: Vec<PaintedGlyph> = term_element::painted_glyphs().into_iter().filter(|g| g.line == line).collect();
            row.sort_by_key(|g| g.column);
            if row.iter().filter(|g| g.ch == '⏵').count() == 2 && row.iter().any(|g| g.ch == 'a') {
                break (row, cell);
            }
        }
        if started.elapsed() > Duration::from_secs(10) {
            bail!("no paint of the `{TRIANGLES}` row (on screen: {})", line.is_some());
        }
    };
    let natural = cx.update(|window, cx| {
        let style = TextRun { len: '⏵'.len_utf8(), font: font.font(), ..Default::default() };
        let shaped = window.text_system().shape_line("⏵".into(), font.size, &[style], None);
        let id = shaped.runs.first().map(|r| r.font_id)?;
        cx.text_system().typographic_bounds(id, font.size, '⏵').ok().map(|b| b.size.height)
    })?;
    let natural = natural.ok_or_else(|| anyhow!("no ink bounds for an unscaled `⏵`"))?;
    let triangles: Vec<&PaintedGlyph> = row.iter().filter(|g| g.ch == '⏵').collect();
    let a = row.iter().find(|g| g.ch == 'a').ok_or_else(|| anyhow!("no `a` on the row"))?;
    let mut shown = Vec::new();
    for (t, stop) in [(triangles[0], triangles[1].column), (triangles[1], a.column)] {
        let (left, right) = t.ink.ok_or_else(|| anyhow!("no ink bounds for `⏵` in cell {}", t.column))?;
        let height = t.ink_height.unwrap_or_default();
        if height < natural * 0.85 {
            bail!("`⏵` in cell {} inks {height:?} high, under 85% of its unscaled {natural:?} (fitted at {:?})", t.column, t.font_size);
        }
        let (from, to) = (cell * t.column as f32, cell * stop as f32);
        if left < from - px(0.5) || right > to + px(0.5) {
            bail!("`⏵` in cell {} inks {left:?}..{right:?}, outside {from:?}..{to:?} (fitted at {:?})", t.column, t.font_size);
        }
        shown.push(format!(
            "{:.1}..{:.1} of {:.1}..{:.1}, {:.0}% high",
            f32::from(left),
            f32::from(right),
            f32::from(from),
            f32::from(to),
            height / natural * 100.
        ));
    }
    Ok(format!("`⏵⏵ a` inks {}", shown.join(" and ")))
}

fn printed_line(terminal: &Terminal, prefix: &str) -> Option<i32> {
    let content = terminal.last_content();
    let mut lines: Vec<(i32, String)> = Vec::new();
    for cell in &content.cells {
        let line = cell.point.line + content.display_offset as i32;
        match lines.last_mut() {
            Some((l, text)) if *l == line => text.push(cell.character()),
            _ => lines.push((line, cell.character().to_string())),
        }
    }
    lines.into_iter().find(|(_, text)| text.starts_with(prefix)).map(|(line, _)| line)
}

async fn open_appearance(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<gpui::WindowHandle<SettingsView>> {
    view.update(cx, |v, cx| v.open_settings_at(Some(page::APPEARANCE), cx));
    for _ in 0..100 {
        if let Some(handle) = view.read_with(cx, |v, _| v.settings_window).filter(|h| h.read_with(cx, |_, _| ()).is_ok()) {
            return Ok(handle);
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    bail!("Settings did not open within 5 s")
}

async fn mono_fonts(handle: &gpui::WindowHandle<SettingsView>, cx: &mut AsyncWindowContext) -> anyhow::Result<std::sync::Arc<Vec<String>>> {
    let started = Instant::now();
    loop {
        if let Some(found) = handle.read_with(cx, |s, _| s.mono_fonts()).map_err(|e| anyhow!("settings window: {e}"))? {
            return Ok(found);
        }
        if started.elapsed() > WITHIN {
            bail!("Appearance found no monospace fonts within {} s", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn close_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) {
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| ids.into_iter().for_each(|id| v.close_tab(id, cx)));
}

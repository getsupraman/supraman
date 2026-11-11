//! `--selftest` step 7g: the Git tab in the window; FleetView is not buildable in a gpui test, nor are real fonts.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use gpui::{AsyncWindowContext, Bounds, Entity, Pixels, Size, point, px, size};
use supraman_core::paths::path_key;
use supraman_core::{Folder, Registry, Workspace};
use supraman_gitman::fixture::Fixture;

use crate::git_view::probe::Probe;
use crate::git_view::{GitView, Selected};
use crate::selftest::{click, until, until_app, until_with};
use crate::selftest_tabs::drag_between;
use crate::view::{FleetView, MIN_LEFT_WIDTH, TabKind};

pub(crate) const WS: &str = "selftest-git";
pub(crate) const FOLDER: &str = "gitman";
pub(crate) const WITHIN: Duration = Duration::from_secs(10);
/// The narrowest window the layout allows: the sidebar's minimum and the tab area's.
const MIN_WINDOW: f32 = 600.;
const PANEL_MIN: f32 = 260.;
const TOOLBAR: [&str; 4] = ["commit-sort", "commit-view-path", "commit-view-tree", "commit-all-files"];

pub(crate) fn git_tabs(v: &FleetView, cx: &gpui::App, dir: &Path) -> Vec<(u64, Entity<GitView>)> {
    let key = path_key(dir);
    v.tabs
        .iter()
        .filter_map(|t| match &t.kind {
            TabKind::Git(g) if path_key(&g.read(cx).folder) == key => Some((t.id, g.clone())),
            _ => None,
        })
        .collect()
}

pub(crate) fn painted(v: &FleetView, key: &str) -> bool {
    v.hits.borrow().contains_key(key)
}

async fn bounds(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Bounds<Pixels>> {
    until(view, cx, WITHIN, &format!("{key} painted"), |v| v.hits.borrow().get(key).map(|h| h.bounds)).await
}

pub(crate) fn probe(view: &Entity<FleetView>, git: &Entity<GitView>, cx: &mut AsyncWindowContext) -> Probe {
    view.read_with(cx, |_, cx| git.read(cx).probe(cx))
}

/// Waits until `f` holds of the tab's probe and the window's hits; the last probe is in the error.
pub(crate) async fn until_probe<T>(view: &Entity<FleetView>, git: &Entity<GitView>, what: &str, cx: &mut AsyncWindowContext, f: impl Fn(&Probe, &FleetView) -> Option<T>) -> anyhow::Result<T> {
    until_app(view, cx, WITHIN, what, |v, cx| f(&git.read(cx).probe(cx), v)).await.map_err(|e| {
        let p = probe(view, git, cx);
        e.context(format!("selected {:?}, commit {:?}, diff {:?}, {} rows, hidden {:?}", p.selected, p.commit, p.diff, p.rows.len(), p.hidden))
    })
}

/// A click on `key` once a frame drawn after the last change painted it: a listener holds the state of its frame.
pub(crate) async fn tap(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    view.update(cx, |v, cx| {
        v.hits.borrow_mut().clear();
        cx.notify();
    });
    until(view, cx, WITHIN, &format!("{key} painted"), |v| v.shown(key).then_some(())).await?;
    click(view, key, cx).await
}

fn row_key(git: &Probe, sha: &str) -> anyhow::Result<String> {
    git.row_of(sha).map(|ix| format!("git-row-{ix}")).ok_or_else(|| anyhow!("no row of {sha}"))
}

async fn viewport(cx: &mut AsyncWindowContext) -> anyhow::Result<Size<Pixels>> {
    Ok(cx.update(|window, _| window.viewport_size())?)
}

pub(crate) async fn resize(view: &Entity<FleetView>, to: Size<Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, _| window.resize(to))?;
    let what = format!("the window at {:.0} px wide", f32::from(to.width));
    until_with(cx, WITHIN, &what, |cx| cx.update(|window, _| ((window.viewport_size().width - to.width).abs() < px(1.)).then_some(())).ok().flatten()).await?;
    view.update(cx, |v, cx| {
        v.hits.borrow_mut().clear();
        cx.notify();
    });
    Ok(())
}

pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let started = Instant::now();
    let parent = home.join("git-fixture");
    std::fs::create_dir_all(&parent)?;
    let fixture = Arc::new(cx.background_executor().spawn(async move { Fixture::new_in(&parent) }).await);
    let registry = Registry::at(home);
    registry.save(&Workspace::new(WS, &fixture.dir, vec![Folder::new(FOLDER, &fixture.dir)]), true)?;
    let (window, left) = (viewport(cx).await?, view.read_with(cx, |v, _| v.sidebar.left_width));
    let result = check(view, &fixture, cx).await;
    view.update(cx, |v, cx| {
        v.sidebar.left_width = left;
        let tabs: Vec<u64> = git_tabs(v, cx, &fixture.dir).into_iter().map(|(id, _)| id).collect();
        tabs.into_iter().for_each(|id| v.close_tab(id, cx));
    });
    let restored = resize(view, window, cx).await;
    registry.remove(WS)?;
    let result = result.and_then(|line| restored.map(|_| line));
    result.map(|line| format!("{line}; {:.1} s", started.elapsed().as_secs_f32()))
}

async fn check(view: &Entity<FleetView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (git, opened) = open(view, f, cx).await?;
    let media = crate::selftest_git_media::check(view, &git, f, cx).await?;
    let narrow = narrow(view, &git, f, cx).await?;
    let panel = panel_min(view, &git, cx).await?;
    Ok([opened, media, narrow, panel].join("; "))
}

async fn open(view: &Entity<FleetView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<(Entity<GitView>, String)> {
    let icon = format!("git-graph-{WS}/{FOLDER}");
    until(view, cx, WITHIN, "the fixture's git-graph icon", |v| v.shown(&icon).then_some(())).await?;
    tap(view, &icon, cx).await?;
    let git = until_app(view, cx, WITHIN, "the Git tab open and active", |v, cx| {
        let tabs = git_tabs(v, cx, &f.dir);
        (tabs.len() == 1 && v.active == Some(tabs[0].0)).then(|| tabs[0].1.clone())
    })
    .await?;
    let rows = until_probe(view, &git, "the graph's rows painted", cx, |p, v| (p.rows.len() >= 7 && painted(v, "git-row-0")).then_some(p.rows.len())).await?;
    Ok((git, format!("{icon} opens Git · {FOLDER} with {rows} rows")))
}

async fn show(view: &Entity<FleetView>, git: &Entity<GitView>, sha: &str, parents: usize, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    until_probe(view, git, &format!("the panel on {sha} with {parents} parents"), cx, |p, v| {
        let (shown, ps) = p.commit.as_ref()?;
        let keys = (0..parents).all(|i| painted(v, &format!("commit-parent-{i}"))) && !painted(v, &format!("commit-parent-{parents}"));
        (p.selected == Some(Selected::Commit(sha.into())) && shown == sha && ps.len() == parents && keys && painted(v, "commit-sha")).then_some(())
    })
    .await
}

/// Every key painted with `prefix` or in `whole` lies inside the tab; `whole` ones also unclipped.
fn outside(v: &FleetView, left: Pixels, right: Pixels, prefixes: &[&str], whole: &[&str]) -> Vec<String> {
    let hits = v.hits.borrow();
    let mut bad: Vec<String> = Vec::new();
    for (key, hit) in hits.iter() {
        let full = whole.contains(&key.as_str());
        if !full && !prefixes.iter().any(|p| key.starts_with(p)) {
            continue;
        }
        let shown = if full { hit.bounds } else { hit.bounds.intersect(&hit.visible) };
        let clipped = full && !hit.fully_visible();
        if clipped || (shown.size.width > px(0.) && (shown.left() < left - px(0.5) || shown.right() > right + px(0.5))) {
            bad.push(format!("{key} {:.0}..{:.0}{}", f32::from(hit.bounds.left()), f32::from(hit.bounds.right()), if clipped { " clipped" } else { "" }));
        }
    }
    for key in whole.iter().filter(|k| !hits.contains_key(**k)) {
        bad.push(format!("{key} not painted"));
    }
    bad.sort();
    bad
}

async fn narrow(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    tap(view, &row_key(&probe(view, git, cx), f.sha("ahead"))?, cx).await?;
    show(view, git, f.sha("ahead"), 1, cx).await?;
    let (window, saved) = (viewport(cx).await?, view.read_with(cx, |v, _| v.sidebar.left_width));
    view.update(cx, |v, _| v.sidebar.left_width = MIN_LEFT_WIDTH);
    resize(view, size(px(MIN_WINDOW), window.height), cx).await?;
    let (left, right) = (MIN_LEFT_WIDTH, px(MIN_WINDOW));
    let fit = until(view, cx, WITHIN, "the Git tab inside the window at its minimum width", |v| {
        let bad = outside(v, left, right, &["git-col-", "git-refs-", "git-more-"], &TOOLBAR);
        (painted(v, "git-col-refs") && bad.is_empty()).then_some(())
    })
    .await;
    let fit = fit.map_err(|e| e.context(format!("outside the tab: {:?}", view.read_with(cx, |v, _| outside(v, left, right, &["git-col-", "git-refs-", "git-more-"], &TOOLBAR)))));
    view.update(cx, |v, _| v.sidebar.left_width = saved);
    resize(view, window, cx).await?;
    fit?;
    Ok(format!("at the {MIN_WINDOW:.0} px window (sidebar {:.0} px) the header cells, ref pills and the commit toolbar stay inside the tab", f32::from(MIN_LEFT_WIDTH)))
}

async fn panel_min(view: &Entity<FleetView>, git: &Entity<GitView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let window = viewport(cx).await?;
    until(view, cx, WITHIN, "the graph's header at full width", |v| painted(v, "git-col-sha").then_some(())).await?;
    let sha = bounds(view, "git-col-sha", cx).await?;
    let from = point(sha.right() + px(0.5), sha.center().y);
    drag_between(from, point(window.width - px(4.), from.y), None, cx).await?;
    let width = until_probe(view, git, "the commit panel dragged to its minimum", cx, |p, _| (p.panel_width <= px(PANEL_MIN + 1.)).then_some(p.panel_width)).await?;
    let left = window.width - width;
    let fit = until(view, cx, WITHIN, "the commit toolbar inside the panel at its minimum", |v| outside(v, left - px(1.), window.width, &[], &TOOLBAR).is_empty().then_some(())).await;
    fit.map_err(|e| e.context(format!("outside the panel: {:?}", view.read_with(cx, |v, _| outside(v, left - px(1.), window.width, &[], &TOOLBAR)))))?;
    Ok(format!("the commit panel dragged to its minimum ({:.0} px) keeps its toolbar inside", f32::from(width)))
}


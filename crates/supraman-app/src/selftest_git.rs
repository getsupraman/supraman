//! `--selftest` step 7g: the Git tab in the window. A unit test cannot do it: FleetView is not buildable in a gpui
//! test, so nothing there clicks the sidebar's git-graph icon, follows a real repo's watchers or lays out real fonts
//! at the window's minimum width. The fixture repo lives in the selftest home (C151); only the step commits to it.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Bounds, Entity, Pixels, Size, point, px, size};
use supraman_core::paths::path_key;
use supraman_core::{Folder, Registry, Workspace};
use supraman_gitman::fixture::{Fixture, MAIN_STASH};
use supraman_ui::themes::{self, NAME_FLOOR, SECONDARY_FLOOR, TerminalColors};
use theme::ActiveTheme as _;

use crate::git_view::diff::{Layout, Mode};
use crate::git_view::probe::{Probe, worst_contrast};
use crate::git_view::{GitView, Selected};
use crate::selftest::{click, until, until_app, until_with};
use crate::selftest_left::move_to;
use crate::selftest_manage::post_keys;
use crate::selftest_tabs::drag_between;
use crate::view::{FleetView, MIN_LEFT_WIDTH, TabKind};

const WS: &str = "selftest-git";
const FOLDER: &str = "gitman";
const WITHIN: Duration = Duration::from_secs(10);
/// The narrowest window the layout allows: the sidebar's minimum and the tab area's.
const MIN_WINDOW: f32 = 600.;
const PANEL_MIN: f32 = 260.;
const TOOLBAR: [&str; 4] = ["commit-sort", "commit-view-path", "commit-view-tree", "commit-all-files"];
const LIGHT: &str = "Selftest Light";

fn git_tabs(v: &FleetView, cx: &gpui::App, dir: &Path) -> Vec<(u64, Entity<GitView>)> {
    let key = path_key(dir);
    v.tabs
        .iter()
        .filter_map(|t| match &t.kind {
            TabKind::Git(g) if path_key(&g.read(cx).folder) == key => Some((t.id, g.clone())),
            _ => None,
        })
        .collect()
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.hits.borrow().contains_key(key)
}

async fn bounds(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Bounds<Pixels>> {
    until(view, cx, WITHIN, &format!("{key} painted"), |v| v.hits.borrow().get(key).map(|h| h.bounds)).await
}

fn probe(view: &Entity<FleetView>, git: &Entity<GitView>, cx: &mut AsyncWindowContext) -> Probe {
    view.read_with(cx, |_, cx| git.read(cx).probe(cx))
}

/// Waits until `f` holds of the tab's probe and the window's hits; the last probe is in the error.
async fn until_probe<T>(view: &Entity<FleetView>, git: &Entity<GitView>, what: &str, cx: &mut AsyncWindowContext, f: impl Fn(&Probe, &FleetView) -> Option<T>) -> anyhow::Result<T> {
    until_app(view, cx, WITHIN, what, |v, cx| f(&git.read(cx).probe(cx), v)).await.map_err(|e| {
        let p = probe(view, git, cx);
        e.context(format!("selected {:?}, commit {:?}, wip {:?}, diff {:?}, {} rows", p.selected, p.commit, p.wip, p.diff, p.rows.len()))
    })
}

/// A click on `key` once a frame drawn after the last change painted it: a listener holds the state of its frame.
async fn tap(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
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

async fn resize(view: &Entity<FleetView>, to: Size<Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
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
    let (window, left) = (viewport(cx).await?, view.read_with(cx, |v, _| v.left_width));
    let result = check(view, &fixture, home, cx).await;
    view.update(cx, |v, cx| {
        v.left_width = left;
        let tabs: Vec<u64> = git_tabs(v, cx, &fixture.dir).into_iter().map(|(id, _)| id).collect();
        tabs.into_iter().for_each(|id| v.close_tab(id, cx));
    });
    let restored = resize(view, window, cx).await;
    registry.remove(WS)?;
    let result = result.and_then(|line| restored.map(|_| line));
    result.map(|line| format!("{line}; {:.1} s", started.elapsed().as_secs_f32()))
}

async fn check(view: &Entity<FleetView>, f: &Arc<Fixture>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (git, opened) = open(view, f, cx).await?;
    let refs = refs(view, &git, cx).await?;
    let selected = select(view, &git, f, cx).await?;
    let files = files(view, &git, f, cx).await?;
    let diff = diff(view, &git, f, cx).await?;
    let stash = stash(view, &git, f, cx).await?;
    let keys = keys(view, &git, f, cx).await?;
    let wip = wip(view, &git, f, cx).await?;
    let live = live(view, &git, f, cx).await?;
    let narrow = narrow(view, &git, f, cx).await?;
    let panel = panel_min(view, &git, cx).await?;
    let themed = themed(view, &git, home, cx).await?;
    Ok([opened, refs, selected, files, diff, stash, keys, wip, live, narrow, panel, themed].join("; "))
}

async fn open(view: &Entity<FleetView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<(Entity<GitView>, String)> {
    let icon = format!("git-graph-{WS}/{FOLDER}");
    until(view, cx, WITHIN, "the fixture's git-graph icon", |v| v.shown(&icon).then_some(())).await?;
    tap(view, &icon, cx).await?;
    let (id, git) = until_app(view, cx, WITHIN, "the Git tab open and active", |v, cx| {
        let tabs = git_tabs(v, cx, &f.dir);
        (tabs.len() == 1 && v.active == Some(tabs[0].0)).then(|| tabs[0].clone())
    })
    .await?;
    let rows = until_probe(view, &git, "the graph's rows painted", cx, |p, v| (p.rows.len() >= 7 && painted(v, "git-row-0")).then_some(p.rows.len())).await?;
    view.update(cx, |v, cx| {
        v.active = None;
        cx.notify();
    });
    tap(view, &icon, cx).await?;
    let (count, active) = view.read_with(cx, |v, cx| (git_tabs(v, cx, &f.dir).len(), v.active));
    if (count, active) != (1, Some(id)) {
        bail!("a second click on {icon}: {count} Git tabs, tab {active:?} active, not the one tab {id}");
    }
    Ok((git, format!("{icon} opens Git · {FOLDER} with {rows} rows, a second click focuses it (one tab)")))
}

async fn refs(view: &Entity<FleetView>, git: &Entity<GitView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let p = probe(view, git, cx);
    let stacked = p.rows.iter().position(|r| r.refs > 1).ok_or_else(|| anyhow!("no row with stacked refs"))?;
    move_to(bounds(view, &format!("git-refs-{stacked}"), cx).await?.center(), cx)?;
    let last = format!("git-reflist-{stacked}-{}", p.rows[stacked].refs - 1);
    let hovered = format!("git-reflist-{stacked}-0");
    until_probe(view, git, "the stacked pill's list on hover", cx, |p, v| (p.listed == Some(stacked) && painted(v, &last) && painted(v, &hovered)).then_some(())).await?;
    let pill = bounds(view, &format!("git-pill-{stacked}"), cx).await?;
    let mut layer = Vec::new();
    for i in 0..p.rows[stacked].refs {
        layer.push(bounds(view, &format!("git-reflist-{stacked}-{i}"), cx).await?);
    }
    let top = layer[0];
    if (top.left() - pill.left()).abs() > px(0.5) || (top.top() - pill.top()).abs() > px(0.5) {
        bail!("the layer {top:?} does not lie on the row's pill {pill:?}");
    }
    for (above, below) in layer.iter().zip(&layer[1..]) {
        if (below.top() - above.bottom()).abs() > px(0.5) || (below.left() - top.left()).abs() > px(0.5) {
            bail!("the layer's groups are not one block under the pill: {above:?} then {below:?}");
        }
    }
    if layer.iter().any(|b| (b.size.width - top.size.width).abs() > px(0.5)) || top.size.width < pill.size.width - px(0.5) {
        bail!("the layer's groups are not one width at least the pill's {pill:?}: {layer:?}");
    }
    let ghost = p.rows.iter().position(|r| r.refs == 0 && r.ghost).ok_or_else(|| anyhow!("no ref-less row with a ghost"))?;
    move_to(bounds(view, &format!("git-row-{ghost}"), cx).await?.center(), cx)?;
    let (pill, cell) = (format!("git-ghost-{ghost}"), format!("git-refs-{ghost}"));
    until_probe(view, git, "the ghost pill in its row's ref cell", cx, |p, v| {
        let hits = v.hits.borrow();
        let (g, c) = (hits.get(&pill)?.bounds, hits.get(&cell)?.bounds);
        (p.listed.is_none() && g.left() >= c.left() && g.right() <= c.right() + px(0.5)).then_some(())
    })
    .await?;
    move_to(point(px(1.), px(1.)), cx)?;
    Ok(format!("row {stacked}'s stacked pill lists its {} groups on hover in one layer over it, one width, at least the pill's; ref-less row {ghost} paints its descendant's ghost pill in its cell", p.rows[stacked].refs))
}

async fn show(view: &Entity<FleetView>, git: &Entity<GitView>, sha: &str, parents: usize, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    until_probe(view, git, &format!("the panel on {sha} with {parents} parents"), cx, |p, v| {
        let (shown, ps) = p.commit.as_ref()?;
        let keys = (0..parents).all(|i| painted(v, &format!("commit-parent-{i}"))) && !painted(v, &format!("commit-parent-{parents}"));
        (p.selected == Some(Selected::Commit(sha.into())) && shown == sha && ps.len() == parents && keys && painted(v, "commit-sha")).then_some(())
    })
    .await
}

async fn select(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    for (label, parents) in [("ahead", 1), ("merge", 2)] {
        tap(view, &row_key(&probe(view, git, cx), f.sha(label))?, cx).await?;
        show(view, git, f.sha(label), parents, cx).await?;
    }
    tap(view, "commit-parent-1", cx).await?;
    show(view, git, f.sha("rename"), 1, cx).await?;
    tap(view, &row_key(&probe(view, git, cx), f.sha("root"))?, cx).await?;
    show(view, git, f.sha("root"), 0, cx).await?;
    Ok("a commit, a merge (two parents; its second parent clicked selects it) and the root (none): the panel's header and parents follow".into())
}

async fn files(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    tap(view, &row_key(&probe(view, git, cx), f.sha("ahead"))?, cx).await?;
    show(view, git, f.sha("ahead"), 1, cx).await?;
    let steps: [(&str, &str, &str); 4] = [
        ("commit-view-path", "commit-files-row-a.txt", "Path's row"),
        ("commit-view-tree", "commit-tree-row-a.txt", "Tree's row"),
        ("commit-all-files", "commit-tree-row-bin.dat", "View all files' unchanged file"),
        ("commit-filter", "commit-filter", "the filter box focused"),
    ];
    for (key, shows, what) in steps {
        tap(view, key, cx).await?;
        until(view, cx, WITHIN, what, |v| painted(v, shows).then_some(())).await?;
    }
    post_keys(&["n", "e", "w"], cx).await?;
    until(view, cx, WITHIN, "the filter on new", |v| (painted(v, "commit-tree-row-new.txt") && !painted(v, "commit-tree-row-a.txt")).then_some(())).await?;
    tap(view, "commit-all-files", cx).await?;
    until(view, cx, WITHIN, "the changed files back", |v| (painted(v, "commit-tree-row-a.txt") && !painted(v, "commit-filter")).then_some(())).await?;
    Ok("Path, Tree, View all files and its filter (\"new\" leaves new.txt) each paint their rows".into())
}

async fn diff(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let ahead = Selected::Commit(f.sha("ahead").into());
    tap(view, "commit-tree-row-a.txt", cx).await?;
    until_probe(view, git, "a.txt's diff with its change", cx, |p, v| p.diff.as_ref().filter(|d| d.path == "a.txt" && d.changes > 0 && painted(v, "diff-next")).map(|_| ())).await?;
    let steps: [(&str, Mode, Layout, Option<usize>); 5] = [
        ("diff-layout-inline", Mode::Diff, Layout::Inline, None),
        ("diff-layout-split", Mode::Diff, Layout::Split, None),
        ("diff-next", Mode::Diff, Layout::Split, Some(0)),
        ("diff-mode-file", Mode::File, Layout::Split, None),
        ("diff-mode-diff", Mode::Diff, Layout::Split, None),
    ];
    for (key, mode, layout, current) in steps {
        tap(view, key, cx).await?;
        until_probe(view, git, key, cx, |p, _| p.diff.as_ref().filter(|d| d.mode == mode && d.layout == layout && (current.is_none() || d.current == current)).map(|_| ())).await?;
    }
    tap(view, "diff-close", cx).await?;
    until_probe(view, git, "the graph back with the commit selected", cx, |p, v| (p.diff.is_none() && p.selected.as_ref() == Some(&ahead) && painted(v, "git-row-0")).then_some(())).await?;
    tap(view, "commit-tree-row-a.txt", cx).await?;
    until_probe(view, git, "a.txt's diff open again", cx, |p, v| p.diff.as_ref().filter(|d| d.path == "a.txt" && painted(v, "diff-close")).map(|_| ())).await?;
    tap(view, "commit-tree-row-a.txt", cx).await?;
    until_probe(view, git, "a second click on a.txt back to the graph", cx, |p, v| (p.diff.is_none() && p.selected.as_ref() == Some(&ahead) && painted(v, "git-row-0")).then_some(())).await?;
    tap(view, &row_key(&probe(view, git, cx), f.sha("rename"))?, cx).await?;
    show(view, git, f.sha("rename"), 1, cx).await?;
    Ok("a.txt's diff: Inline, Split, next change (1st), File View and back, × back to the graph with the commit still selected; a.txt clicked twice opens then closes its diff, the commit kept, and the next commit selects".into())
}

async fn stash(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (index, sha, base) = (f.sha("stash_message_index"), f.sha("stash_message"), f.sha("merge"));
    let p = probe(view, git, cx);
    let ix = p.row_of(index).ok_or_else(|| anyhow!("no stash row"))?;
    if !(p.rows[ix].stash && p.rows[ix].summary == MAIN_STASH) {
        bail!("row {ix} is not the stash {MAIN_STASH:?}: {:?}", p.rows[ix]);
    }
    tap(view, &format!("git-row-{ix}"), cx).await?;
    until_probe(view, git, "the panel on the stash, its base and its file", cx, |p, v| {
        let stashed = matches!(&p.selected, Some(Selected::Stash(s)) if s.sha == sha);
        let shown = p.commit.as_ref().is_some_and(|(shown, ps)| shown == sha && *ps == [base.to_string()]);
        (stashed && shown && painted(v, "commit-tree-row-a.txt") && !painted(v, "commit-parent-1")).then_some(())
    })
    .await?;
    tap(view, "commit-tree-row-a.txt", cx).await?;
    until_probe(view, git, "the stashed a.txt's diff", cx, |p, v| p.diff.as_ref().filter(|d| d.path == "a.txt" && d.changes > 0 && painted(v, "diff-close")).map(|_| ())).await?;
    tap(view, "diff-close", cx).await?;
    Ok(format!("stash row {ix} shows {MAIN_STASH:?}; selected, the panel shows the stash against its base with a.txt, whose diff opens"))
}

/// Keys posted to the window as the keyboard sends them: Right, Left and Page Down on the graph's rows.
async fn keys(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let root = Selected::Commit(f.sha("root").into());
    tap(view, &row_key(&probe(view, git, cx), f.sha("root"))?, cx).await?;
    show(view, git, f.sha("root"), 0, cx).await?;
    post_keys(&["right"], cx).await?;
    until_probe(view, git, "Right: the root's first file at its first change", cx, |p, v| {
        p.diff.as_ref().filter(|d| d.path == "a.txt" && d.mode == Mode::Diff && d.current == Some(0) && painted(v, "diff-close")).map(|_| ())
    })
    .await?;
    post_keys(&["left"], cx).await?;
    let row = until_probe(view, git, "Left: back to the root's row", cx, |p, v| {
        let ix = p.row_of(f.sha("root"))?;
        (p.diff.is_none() && p.selected.as_ref() == Some(&root) && painted(v, &format!("git-row-{ix}"))).then_some(ix)
    })
    .await?;
    tap(view, "git-row-0", cx).await?;
    until_probe(view, git, "the WIP row selected", cx, |p, _| (p.selected == Some(Selected::Wip)).then_some(())).await?;
    post_keys(&["pagedown"], cx).await?;
    let paged = until_probe(view, git, "Page Down: a page further", cx, |p, _| match &p.selected {
        Some(Selected::Commit(sha)) => p.row_of(sha).filter(|ix| *ix > 0),
        Some(Selected::Stash(s)) => p.row_of(&s.index),
        _ => None,
    })
    .await?;
    Ok(format!("Right on the root opens a.txt, its first file, at its first change; Left goes back to row {row}; Page Down from the WIP row selects row {paged}"))
}

async fn wip(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    tap(view, "git-row-0", cx).await?;
    let (unstaged, staged) = until_probe(view, git, "the WIP panel's lists", cx, |p, v| {
        let (u, s) = p.wip.clone()?;
        let ok = p.selected == Some(Selected::Wip) && u.iter().any(|f| f == "u.txt") && s.iter().any(|f| f == "s.txt");
        (ok && painted(v, "wip-unstaged-header") && painted(v, "wip-staged-header")).then_some((u.len(), s.len()))
    })
    .await?;
    tap(view, "wip-unstaged-files-row-a.txt", cx).await?;
    until_probe(view, git, "a.txt's working-tree diff", cx, |p, v| p.diff.as_ref().filter(|d| d.path == "a.txt" && painted(v, "diff-close")).map(|_| ())).await?;
    tap(view, "diff-close", cx).await?;
    tap(view, &row_key(&probe(view, git, cx), f.sha("ahead"))?, cx).await?;
    until(view, cx, WITHIN, "the dirty tree's banner over the commit", |v| painted(v, "wip-banner-view-change").then_some(())).await?;
    tap(view, "wip-banner-view-change", cx).await?;
    until_probe(view, git, "View Change selecting the WIP row", cx, |p, _| (p.selected == Some(Selected::Wip)).then_some(())).await?;
    Ok(format!("the WIP row lists {unstaged} unstaged and {staged} staged files, opens a working-tree diff; the banner's View Change selects it"))
}

async fn live(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let fixture = f.clone();
    let sha = cx
        .background_executor()
        .spawn(async move {
            fixture.git(&["commit", "-q", "--no-verify", "-m", "Selftest live commit"]);
            fixture.git(&["rev-parse", "HEAD"])
        })
        .await;
    let committed = Instant::now();
    until_probe(view, git, "the commit made with git in the graph", cx, |p, _| p.row_of(&sha)).await?;
    Ok(format!("a commit made with git on the fixture shows in the graph after {} ms by itself", committed.elapsed().as_millis()))
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
    let (window, saved) = (viewport(cx).await?, view.read_with(cx, |v, _| v.left_width));
    view.update(cx, |v, _| v.left_width = MIN_LEFT_WIDTH);
    resize(view, size(px(MIN_WINDOW), window.height), cx).await?;
    let (left, right) = (MIN_LEFT_WIDTH, px(MIN_WINDOW));
    let fit = until(view, cx, WITHIN, "the Git tab inside the window at its minimum width", |v| {
        let bad = outside(v, left, right, &["git-col-", "git-refs-", "git-more-"], &TOOLBAR);
        (painted(v, "git-col-refs") && bad.is_empty()).then_some(())
    })
    .await;
    let fit = fit.map_err(|e| e.context(format!("outside the tab: {:?}", view.read_with(cx, |v, _| outside(v, left, right, &["git-col-", "git-refs-", "git-more-"], &TOOLBAR)))));
    view.update(cx, |v, _| v.left_width = saved);
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

async fn themed(view: &Entity<FleetView>, git: &Entity<GitView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let dir = themes::themes_dir(home);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("selftest-light.json"), crate::selftest_schedules::LIGHT)?;
    let apply = |name: Option<&str>, cx: &mut AsyncWindowContext| {
        cx.update(|_, cx| {
            themes::apply(cx, name, TerminalColors::Theme, Some(home));
        })
    };
    apply(Some(LIGHT), cx)?;
    let repainted = |view: &Entity<FleetView>, cx: &mut AsyncWindowContext| {
        view.update(cx, |v, cx| {
            v.hits.borrow_mut().clear();
            cx.notify();
        })
    };
    repainted(view, cx);
    let light = until_probe(view, git, "the Git tab repainted in the light theme", cx, |p, v| (p.rows.len() > 1 && painted(v, "git-row-1")).then_some(())).await;
    let contrast = cx.update(|_, cx| (cx.theme().name.to_string(), worst_contrast(cx)));
    apply(None, cx)?;
    std::fs::remove_file(dir.join("selftest-light.json")).ok();
    repainted(view, cx);
    light?;
    let (name, (text, secondary)) = contrast?;
    if name != LIGHT || text < NAME_FLOOR || secondary < SECONDARY_FLOOR {
        bail!("in {name:?} the Git tab's text reads {text:.1}:1 on a band (floor {NAME_FLOOR}) and {secondary:.1}:1 on a pill or muted (floor {SECONDARY_FLOOR})");
    }
    until(view, cx, WITHIN, "the Git tab repainted in the default theme", |v| painted(v, "git-row-1").then_some(())).await?;
    Ok(format!("in {LIGHT} it repaints with text at {text:.1}:1 on its bands and {secondary:.1}:1 on pills, then back"))
}

//! `--selftest` step 7g's reopen: a ref hidden by its mark and the graph column narrowed by its divider, the Git tab
//! closed and opened again from the sidebar: its columns, panels and hidden refs come back from the layout file. A
//! unit test cannot close a FleetView tab and click the sidebar's icon to open it again.

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, point, px};
use supraman_gitman::fixture::Fixture;

use crate::git_view::GitView;
use crate::selftest::{until, until_app};
use crate::selftest_git::{git_tabs, painted, probe, tap, until_probe, FOLDER, WITHIN, WS};
use crate::selftest_tabs::drag_between;
use crate::view::FleetView;

const HIDDEN: &str = "refs/heads/feature";

pub async fn check(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    if !probe(view, git, cx).refs.hidden.iter().any(|h| h == HIDDEN) {
        tap(view, "refpanel-mark-local/feature", cx).await?;
    }
    until_probe(view, git, "feature hidden", cx, |p, _| p.refs.hidden.iter().any(|h| h == HIDDEN).then_some(())).await?;
    let before = view.read_with(cx, |_, cx| git.read(cx).sizes(cx));
    let divider = until(view, cx, WITHIN, "the graph column's divider", |v| v.hits.borrow().get("git-divider-graph").map(|h| h.bounds.center())).await?;
    drag_between(divider, point(divider.x - px(40.), divider.y), None, cx).await?;
    let kept = until_app(view, cx, WITHIN, "the graph column narrowed", |_, cx| {
        let sizes = git.read(cx).sizes(cx);
        (sizes.widths.get(1) < before.widths.get(1)).then_some(sizes)
    })
    .await?;
    let id = view.read_with(cx, |v, cx| git_tabs(v, cx, &f.dir).first().map(|(id, _)| *id));
    view.update(cx, |v, cx| id.into_iter().for_each(|id| v.close_tab(id, cx)));
    until_app(view, cx, WITHIN, "the Git tab closed", |v, cx| git_tabs(v, cx, &f.dir).is_empty().then_some(())).await?;
    tap(view, &format!("git-graph-{WS}/{FOLDER}"), cx).await?;
    let again = until_app(view, cx, WITHIN, "the Git tab open again", |v, cx| git_tabs(v, cx, &f.dir).first().map(|(_, g)| g.clone())).await?;
    until_probe(view, &again, "the reopened graph painted", cx, |p, v| (p.rows.len() > 1 && painted(v, "git-row-1")).then_some(())).await?;
    let back = view.read_with(cx, |_, cx| again.read(cx).sizes(cx));
    if back != kept {
        bail!("reopened at {back:?}, closed at {kept:?}");
    }
    let width = |s: &crate::git_view::GitSizes| s.widths.get(1).copied().unwrap_or_default();
    Ok(format!(
        "closed and opened again, the tab keeps its columns (graph {} px, narrowed from {}), its panels ({:?} px) and {HIDDEN} hidden",
        width(&back),
        width(&before),
        back.panel.unwrap_or_default()
    ))
}

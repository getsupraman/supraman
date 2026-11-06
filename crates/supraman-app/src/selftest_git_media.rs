//! `--selftest` step 7g's media diffs: a changed png painted on both sides in Split and Inline by the window's
//! own renderer and atlas.

use std::sync::Arc;

use gpui::{AsyncWindowContext, Entity};
use supraman_gitman::fixture::Fixture;

use crate::git_view::GitView;
use crate::git_view::diff::Target;
use crate::selftest_git::{painted, tap, until_probe};
use crate::view::FleetView;

async fn open(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, path: &'static str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let job = f.clone();
    let change = cx.background_executor().spawn(async move { supraman_gitman::changed_files(&job.repo(), job.sha("media")) }).await?.into_iter().find(|c| c.path == path);
    let target = Target::commit(f.sha("media").into(), Some(f.sha("media_base").into()), path.into(), change);
    view.update(cx, |_, cx| git.update(cx, |g, cx| g.open_diff(target, cx)));
    until_probe(view, git, &format!("{path}'s diff"), cx, |p, _| p.diff.as_ref().filter(|d| d.path == path && d.shown != "loading").map(|_| ())).await
}

pub(crate) async fn check(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    open(view, git, f, "changed.png", cx).await?;
    for key in ["diff-layout-inline", "diff-layout-split"] {
        tap(view, key, cx).await?;
        let what = format!("{key}: changed.png's two images painted");
        until_probe(view, git, &what, cx, |p, v| p.diff.as_ref().filter(|d| d.shown == "image 4×3 → 8×6" && painted(v, "diff-image-old") && painted(v, "diff-image-new")).map(|_| ())).await?;
    }
    tap(view, "diff-close", cx).await?;
    until_probe(view, git, "the graph back after the media diffs", cx, |p, _| p.diff.is_none().then_some(())).await?;
    Ok("changed.png's diff paints both images (4×3 → 8×6) in Inline and Split".into())
}

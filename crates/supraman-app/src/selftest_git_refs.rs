//! `--selftest` step 7g's ref panel: its sections and counts from the fixture, a branch's row selecting its commit,
//! a branch hidden by its eye leaving the graph with the commit only it reaches, and Ctrl+J collapsing the panel to
//! its strip. A unit test cannot click the real window's rows and eyes or send Ctrl+J through the window's keymap.

use std::sync::Arc;

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, Keystroke};
use supraman_gitman::fixture::Fixture;

use crate::git_view::{GitView, Selected};
use crate::selftest_git::{painted, probe, tap, until_probe};
use crate::view::FleetView;

const SOLO: &str = "topic/solo";

pub async fn check(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let counts = counts(view, git, f, cx).await?;
    let picked = pick(view, git, f, cx).await?;
    let hidden = hide(view, git, f, cx).await?;
    let strip = strip(view, git, f, cx).await?;
    Ok([counts, picked, hidden, strip].join("; "))
}

async fn counts(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let repo = f.repo();
    let refs = cx.background_executor().spawn(async move { supraman_gitman::refs(&repo) }).await?;
    let want = [refs.branches.len(), refs.remotes.len(), refs.worktrees.len(), refs.stashes.len(), refs.tags.len()].map(|n| (n, n));
    let sections = ["refpanel-row-local", "refpanel-row-remote", "refpanel-row-worktrees", "refpanel-row-stashes", "refpanel-row-tags"];
    until_probe(view, git, "the ref panel's five sections with the fixture's counts", cx, |p, v| {
        (p.refs.counts == want && sections.iter().all(|k| painted(v, k)) && painted(v, "refpanel-row-local/main")).then_some(())
    })
    .await?;
    Ok(format!("the ref panel shows LOCAL, REMOTE, WORKTREES, STASHES and TAGS counting {want:?}"))
}

async fn pick(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let feature = Selected::Commit(f.sha("rename").into());
    tap(view, "refpanel-row-local/feature", cx).await?;
    until_probe(view, git, "feature's row selecting its commit", cx, |p, _| (p.selected.as_ref() == Some(&feature)).then_some(())).await?;
    Ok("a click on feature's row selects its commit in the graph".into())
}

async fn hide(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let fixture = f.clone();
    let merge = f.sha("merge").to_string();
    let only = cx
        .background_executor()
        .spawn(async move {
            let sha = fixture.git(&["commit-tree", "-p", &merge, "-m", "Selftest solo", &format!("{merge}^{{tree}}")]);
            fixture.git(&["branch", SOLO, &sha]);
            sha
        })
        .await;
    until_probe(view, git, "topic/solo's folder and its commit", cx, |p, _| (p.row_of(&only).is_some() && p.refs.rows.iter().any(|r| r == "local/topic")).then_some(())).await?;
    if !probe(view, git, cx).refs.rows.iter().any(|r| r == "local/topic/solo") {
        tap(view, "refpanel-row-local/topic", cx).await?;
    }
    let row = format!("refpanel-row-local/{SOLO}");
    until_probe(view, git, "the topic folder open", cx, |_, v| painted(v, &row).then_some(())).await?;
    let local = probe(view, git, cx).refs.counts[0];
    tap(view, &format!("refpanel-eye-local/{SOLO}"), cx).await?;
    let key = format!("refs/heads/{SOLO}");
    until_probe(view, git, "topic/solo hidden and its commit out of the graph", cx, |p, _| {
        (p.row_of(&only).is_none() && p.row_of(f.sha("merge")).is_some() && p.refs.hidden == [key.clone()] && p.refs.counts[0] == (local.0 - 1, local.1)).then_some(())
    })
    .await?;
    tap(view, "refpanel-show-all", cx).await?;
    view.update(cx, |v, cx| {
        v.hits.borrow_mut().clear();
        cx.notify();
    });
    until_probe(view, git, "Show all bringing topic/solo's commit back", cx, |p, v| (p.row_of(&only).is_some() && p.refs.hidden.is_empty() && !painted(v, "refpanel-show-all")).then_some(())).await?;
    Ok(format!("its eye hides {SOLO}: the commit only it reaches leaves the graph, LOCAL counts {}/{}; Show all brings it back", local.0 - 1, local.1))
}

async fn strip(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let ahead = probe(view, git, cx).row_of(f.sha("ahead")).ok_or_else(|| anyhow::anyhow!("no row of ahead"))?;
    tap(view, &format!("git-row-{ahead}"), cx).await?;
    cx.update(|window, cx| window.dispatch_keystroke(Keystroke::parse("ctrl-j").expect("a keystroke"), cx))?;
    view.update(cx, |v, cx| {
        v.hits.borrow_mut().clear();
        cx.notify();
    });
    let strip = ["local", "remote", "worktrees", "stashes", "tags"].map(|s| format!("refpanel-strip-{s}"));
    until_probe(view, git, "Ctrl+J collapsing the panel to its strip", cx, |p, v| {
        (p.refs.shut && painted(v, "refpanel-expand") && strip.iter().all(|k| painted(v, k)) && !painted(v, "refpanel-filter")).then_some(())
    })
    .await?;
    let width = view.read_with(cx, |v, _| v.hits.borrow().get("refpanel-strip-local").map(|h| h.bounds.size.width));
    if width.is_none_or(|w| w > crate::git_view::ref_panel::STRIP) {
        bail!("the strip's section button is {width:?} wide");
    }
    tap(view, "refpanel-strip-tags", cx).await?;
    until_probe(view, git, "a section's icon opening the panel on it", cx, |p, v| (!p.refs.shut && painted(v, "refpanel-row-tags/v1")).then_some(())).await?;
    Ok("Ctrl+J collapses it to a strip of the sections' icons; the tags' icon opens it on TAGS".into())
}

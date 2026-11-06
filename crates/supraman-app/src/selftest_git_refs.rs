//! `--selftest` step 7g's ref panel: its sections and counts from the fixture, a branch's row selecting its commit,
//! a branch soloed from its right-click menu leaving only its commits and HEAD's, a branch hidden by its mark leaving the graph with the commit
//! only it reaches, Ctrl+J right after Show all collapsing the panel to its strip, and a section's overlay picking a
//! ref. A unit test cannot click the real window's rows, marks and menus or send Ctrl+J through the window's keymap
//! after the window's own focus handling.

use std::sync::Arc;

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, Keystroke, MouseButton};
use supraman_gitman::fixture::Fixture;

use crate::git_view::{GitView, Selected};
use crate::selftest_git::{painted, probe, tap, until_probe};
use crate::selftest_manage::press_painted;
use crate::view::FleetView;

const SOLO: &str = "topic/solo";

pub async fn check(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let counts = counts(view, git, f, cx).await?;
    let picked = pick(view, git, f, cx).await?;
    let only = solo_branch(view, git, f, cx).await?;
    let soloed = solo(view, git, f, &only, cx).await?;
    let hidden = hide(view, git, f, &only, cx).await?;
    let strip = strip(view, git, cx).await?;
    let overlay = overlay(view, git, f, cx).await?;
    Ok([counts, picked, soloed, hidden, strip, overlay].join("; "))
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

/// `topic/solo`, one commit of its own on the merge, its row shown; that commit.
async fn solo_branch(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Arc<Fixture>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
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
    Ok(only)
}

async fn solo(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, only: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (key, mark) = (format!("refs/heads/{SOLO}"), format!("refpanel-mark-local/{SOLO}"));
    let stashes = probe(view, git, cx).rows.iter().filter(|r| r.stash).count();
    press_painted(view, &format!("refpanel-name-local/{SOLO}"), MouseButton::Right, cx).await?;
    tap(view, "menu-ref-solo", cx).await?;
    until_probe(view, git, "topic/solo soloed: only its commit and HEAD's history walked", cx, |p, _| {
        let walked = p.row_of(only).is_some() && p.row_of(f.sha("ahead")).is_some() && p.rows.iter().all(|r| !r.stash);
        (p.refs.solo == [key.clone()] && walked && p.refs.counts[0].0 == 2).then_some(())
    })
    .await?;
    tap(view, &mark, cx).await?;
    until_probe(view, git, "the solo cleared and the stashes back", cx, |p, _| (p.refs.solo.is_empty() && p.rows.iter().filter(|r| r.stash).count() == stashes).then_some(())).await?;
    Ok(format!("its menu's Solo walks {SOLO} and HEAD alone: its commit stays, the {stashes} stashes leave, LOCAL counts 2 shown; clearing it brings them back"))
}

async fn hide(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, only: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let local = probe(view, git, cx).refs.counts[0];
    tap(view, &format!("refpanel-mark-local/{SOLO}"), cx).await?;
    let key = format!("refs/heads/{SOLO}");
    until_probe(view, git, "topic/solo hidden and its commit out of the graph", cx, |p, _| {
        (p.row_of(only).is_none() && p.row_of(f.sha("merge")).is_some() && p.refs.hidden == [key.clone()] && p.refs.counts[0] == (local.0 - 1, local.1)).then_some(())
    })
    .await?;
    tap(view, "refpanel-show-all", cx).await?;
    view.update(cx, |v, cx| {
        v.hits.borrow_mut().clear();
        cx.notify();
    });
    until_probe(view, git, "Show all bringing topic/solo's commit back", cx, |p, v| (p.row_of(only).is_some() && p.refs.hidden.is_empty() && !painted(v, "refpanel-show-all")).then_some(())).await?;
    Ok(format!("its mark hides {SOLO}: the commit only it reaches leaves the graph, LOCAL counts {}/{}; Show all brings it back", local.0 - 1, local.1))
}

/// Ctrl+J pressed with nothing clicked since Show all.
async fn strip(view: &Entity<FleetView>, git: &Entity<GitView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    cx.update(|window, cx| window.dispatch_keystroke(Keystroke::parse("ctrl-j").expect("a keystroke"), cx))?;
    view.update(cx, |v, cx| {
        v.hits.borrow_mut().clear();
        cx.notify();
    });
    let strip = ["local", "remote", "worktrees", "stashes", "tags"].map(|s| format!("refpanel-strip-{s}"));
    until_probe(view, git, "Ctrl+J right after Show all collapsing the panel to its strip", cx, |p, v| {
        (p.refs.shut && painted(v, "refpanel-expand") && strip.iter().all(|k| painted(v, k)) && !painted(v, "refpanel-filter")).then_some(())
    })
    .await?;
    let width = view.read_with(cx, |v, _| v.hits.borrow().get("refpanel-strip-local").map(|h| h.bounds.size.width));
    if width.is_none_or(|w| w > crate::git_view::ref_panel::STRIP) {
        bail!("the strip's section button is {width:?} wide");
    }
    Ok("Ctrl+J right after Show all collapses it to a strip of the sections' icons".into())
}

async fn overlay(view: &Entity<FleetView>, git: &Entity<GitView>, f: &Fixture, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    tap(view, "refpanel-strip-local", cx).await?;
    until_probe(view, git, "LOCAL's overlay with its refs", cx, |p, v| {
        let local = p.refs.overlay.as_ref().is_some_and(|(s, rows)| *s == "local" && rows.iter().any(|r| r == "stale"));
        (p.refs.shut && local && painted(v, "refpanel-overlay-row-stale")).then_some(())
    })
    .await?;
    let (card, strip) = view.read_with(cx, |v, _| {
        let hits = v.hits.borrow();
        (hits.get("refpanel-overlay").map(|h| h.bounds), hits.get("refpanel-strip-local").map(|h| h.bounds))
    });
    let (Some(card), Some(strip)) = (card, strip) else { bail!("the overlay {card:?} or the strip {strip:?} not painted") };
    if card.left() < strip.right() {
        bail!("the overlay {card:?} is not beside the strip {strip:?}");
    }
    tap(view, "refpanel-overlay-row-stale", cx).await?;
    let stale = Selected::Commit(f.sha("main_work").into());
    until_probe(view, git, "stale picked in the overlay: its commit selected, the overlay closed", cx, |p, _| {
        (p.selected.as_ref() == Some(&stale) && p.refs.overlay.is_none() && p.refs.shut).then_some(())
    })
    .await?;
    tap(view, "refpanel-expand", cx).await?;
    until_probe(view, git, "the panel open again", cx, |p, v| (!p.refs.shut && painted(v, "refpanel-row-local/main")).then_some(())).await?;
    Ok("LOCAL's icon opens its overlay beside the strip with the panel collapsed; stale's row there selects its commit and closes it".into())
}

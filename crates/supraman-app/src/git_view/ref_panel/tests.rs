use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, Generated};
use supraman_gitman::{Branch, Head, Refs, RemoteBranch, Stash, Tag, Upstream, Worktree};
use supraman_ui::hit::HitTargets;

use super::*;
use crate::git_view::tests::{open, redraw};
use crate::git_view::{GitView, Selected};

fn branch(name: &str, sha: &str) -> Branch {
    Branch { name: name.into(), sha: sha.into(), upstream: None }
}

fn stash(sha: &str, branch: &str, message: &str) -> Stash {
    Stash { sha: sha.into(), index: format!("{sha}-index"), base: "b".into(), branch: branch.into(), message: message.into(), time: 0 }
}

fn worktree(path: &str, branch: Option<&str>) -> Worktree {
    Worktree { path: PathBuf::from(path), head: Some(format!("{path}-head")), branch: branch.map(Into::into), detached: branch.is_none(), ..Worktree::default() }
}

fn refs() -> Refs {
    let mut main = branch("main", "m");
    main.upstream = Some(Upstream { name: "origin/main".into(), ahead: 2, behind: 1, gone: false });
    let names = ["zeta", "fix/b", "fix/a", "Alpha", "feature/login/ui", "feature/api", "mobile/x"];
    Refs {
        head: Head::Branch { name: "main".into(), sha: Some("m".into()) },
        branches: std::iter::once(main).chain(names.iter().map(|n| branch(n, &format!("{n}-sha")))).collect(),
        remotes: ["upstream/dev", "origin/main", "origin/fix/a"].iter().map(|n| RemoteBranch { name: (*n).into(), sha: format!("{n}-sha") }).collect(),
        tags: ["v2", "release/r1", "v1"].iter().map(|n| Tag { name: (*n).into(), sha: format!("{n}-tag"), annotated: false, commit: Some(format!("{n}-sha")) }).collect(),
        worktrees: vec![worktree("C:/src/repo", Some("main")), worktree("C:/src/wt", None), worktree("C:/src/side", Some("side"))],
        stashes: vec![stash("s0", "main", ""), stash("s1", "feature/api", "Try this")],
    }
}

fn panel(cx: &mut TestAppContext, hidden: &[&str]) -> Entity<RefPanel> {
    let hidden: Vec<String> = hidden.iter().map(|h| h.to_string()).collect();
    let panel = cx.update(|cx| cx.new(|cx| RefPanel::new(HitTargets::default(), None, false, &hidden, cx)));
    panel.update(cx, |p, cx| p.set_refs(&refs(), cx));
    panel
}

fn visible(panel: &Entity<RefPanel>, cx: &mut TestAppContext) -> Vec<String> {
    panel.read_with(cx, |p, _| p.visible().iter().map(|p| p.to_string()).collect())
}

#[gpui::test]
fn ref_panel_groups_by_folders_and_sorts_folders_among_branches(cx: &mut TestAppContext) {
    let panel = panel(cx, &[]);
    assert_eq!(
        visible(&panel, cx),
        [
            "local", "local/Alpha", "local/feature", "local/feature/api", "local/feature/login", "local/feature/login/ui", "local/fix", "local/fix/a", "local/fix/b", "local/main",
            "local/mobile", "local/mobile/x", "local/zeta",
            "remote", "remote/origin", "remote/origin/fix", "remote/origin/fix/a", "remote/origin/main", "remote/upstream", "remote/upstream/dev",
            "worktrees", "worktrees/0", "worktrees/1", "worktrees/2",
            "stashes", "stashes/0", "stashes/1",
            "tags", "tags/release", "tags/release/r1", "tags/v1", "tags/v2",
        ],
        "the sections in turn; A–Z ignoring case at each depth with folders among the refs (mobile after main); stashes newest first"
    );
    let detached = Refs { head: Head::Detached { sha: "d".into() }, ..refs() };
    let all = entries(&detached);
    assert_eq!((all[0].0.as_str(), &all[0].1), ("local/HEAD", &Entry::Detached { sha: "d".into() }), "a detached HEAD first");
    assert!(all.iter().all(|(_, e)| !matches!(e, Entry::Branch { current: true, .. })), "no branch is current");
    let main = entries(&refs()).into_iter().find(|(p, _)| p == "local/main").unwrap().1;
    assert_eq!(main, Entry::Branch { name: "main".into(), sha: "m".into(), current: true, ahead: 2, behind: 1 }, "the current branch, ahead and behind its upstream");
    assert_eq!(main.hide_key(), None, "the current branch never hides");
}

#[gpui::test]
fn ref_panel_counts_shown_and_filtered_refs_per_section(cx: &mut TestAppContext) {
    let panel = panel(cx, &["refs/heads/zeta", "refs/remotes/upstream/dev", "stash/s0"]);
    let words = |c: Count| (c.shown, c.total);
    let counts = panel.read_with(cx, |p, _| p.counts().map(words));
    assert_eq!(counts, [(7, 8), (2, 3), (3, 3), (1, 2), (3, 3)], "shown/total per section; a hidden ref is counted but not shown");
    panel.update(cx, |p, cx| p.set_filter("fx", cx));
    assert_eq!(panel.read_with(cx, |p, _| p.counts().map(words)), [(2, 2), (1, 1), (0, 0), (0, 0), (0, 0)], "fx: fix/a, fix/b and origin/fix/a");
    assert_eq!(visible(&panel, cx), ["local", "local/fix", "local/fix/a", "local/fix/b", "remote", "remote/origin", "remote/origin/fix", "remote/origin/fix/a"]);
    panel.update(cx, |p, cx| p.set_filter("ORIGIN/m", cx));
    assert_eq!(visible(&panel, cx), ["remote", "remote/origin", "remote/origin/main"], "a remote branch matches with its remote's name");
    panel.update(cx, |p, cx| p.set_filter("src", cx));
    assert!(visible(&panel, cx).is_empty(), "a worktree's folder name, not its path");
    panel.update(cx, |p, cx| p.set_filter("side", cx));
    assert_eq!(visible(&panel, cx), ["worktrees", "worktrees/2"]);
}

#[test]
fn ref_panel_highlights_the_filters_matches_in_the_label() {
    let ui = Entry::Branch { name: "feature/login/ui".into(), sha: "s".into(), current: false, ahead: 0, behind: 0 };
    assert_eq!(ui.label(), "ui");
    assert_eq!(ui.marks("flu"), [0..1], "f and l fall on the folders' names, u on the label");
    assert_eq!(ui.marks("ui"), [0..2]);
    assert!(ui.marks("").is_empty() && ui.marks("zz").is_empty());
    let origin = Entry::Remote { name: "origin/main".into(), sha: "s".into() };
    assert_eq!((origin.label().as_str(), origin.marks("oma")), ("main", vec![0..2]));
    let s = Entry::Stash { sha: "s".into(), index: "i".into(), label: "Try this".into(), branch: "main".into() };
    assert_eq!(s.marks("this"), [4..8]);
    assert!(s.matches("on: main") && s.marks("on: main").is_empty(), "the branch matches, the label shows no mark");
}

#[test]
fn ref_panel_labels_worktrees_and_stashes() {
    let all = entries(&refs());
    let at = |path: &str| all.iter().find(|(p, _)| p == path).map(|(_, e)| e.clone()).unwrap();
    assert_eq!(at("worktrees/0"), Entry::Worktree { label: "main".into(), folder: Some("repo".into()), main: true, sha: Some("C:/src/repo-head".into()) }, "the main worktree: its branch, its folder beside");
    assert_eq!(at("worktrees/1"), Entry::Worktree { label: "wt".into(), folder: None, main: false, sha: Some("C:/src/wt-head".into()) }, "detached: its folder's name");
    assert_eq!(at("worktrees/2"), Entry::Worktree { label: "side".into(), folder: None, main: false, sha: Some("C:/src/side-head".into()) }, "no folder where it is the branch's name");
    let wip = at("stashes/0");
    assert_eq!((wip.label().as_str(), wip.commit(), wip.hide_key()), ("WIP on main", Some("s0-index"), Some("stash/s0".into())), "a stash without a message; its row is its index commit");
    assert_eq!(at("stashes/1").label(), "Try this");
    assert_eq!(at("tags/release/r1").label(), "r1");
}

#[test]
fn hidden_refs_leave_the_walk_but_the_current_branch_stays() {
    let hidden: BTreeSet<String> = ["refs/heads/main", "refs/heads/fix/a", "refs/remotes/origin/main", "stash/s1", "refs/heads/side"].iter().map(|s| s.to_string()).collect();
    let walked = without(&refs(), &hidden);
    let names: Vec<&str> = walked.branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"main") && !names.contains(&"fix/a") && names.len() == 7, "{names:?}");
    assert_eq!(walked.remotes.len(), 2);
    assert_eq!(walked.stashes.iter().map(|s| s.sha.as_str()).collect::<Vec<_>>(), ["s0"]);
    assert_eq!(walked.worktrees.len(), 2, "the worktree on a hidden branch is no tip either");
    assert_eq!(without(&refs(), &BTreeSet::new()), refs());
}

/// The fixture with `topic/solo`, one commit of its own on `main`'s merge: hiding it drops that commit.
fn solo(f: &Fixture) -> String {
    let sha = f.git(&["commit-tree", "-p", f.sha("merge"), "-m", "Solo", &format!("{}^{{tree}}", f.sha("merge"))]);
    f.git(&["branch", "topic/solo", &sha]);
    sha
}

fn has(view: &Entity<GitView>, sha: &str, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.rows.iter().any(|r| r.sha.as_ref() == sha))
}

#[gpui::test]
fn hiding_a_branch_prunes_its_own_commits_from_the_graph_and_show_all_restores_them(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let only = solo(&f);
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    assert!(has(&view, &only, cx) && has(&view, f.sha("stash_feature_index"), cx));
    panel.update(cx, |p, cx| p.toggle_hidden(&["refs/heads/topic/solo".to_string()], cx));
    cx.run_until_parked();
    assert!(!has(&view, &only, cx), "a commit only the hidden branch reaches leaves the graph");
    assert!(has(&view, f.sha("merge"), cx), "its parent, reachable from main, stays");
    let keys = panel.read_with(cx, |p, _| p.tree.aggregate("stashes").unwrap().0);
    panel.update(cx, |p, cx| p.toggle_hidden(&keys, cx));
    cx.run_until_parked();
    assert!(!has(&view, f.sha("stash_feature_index"), cx) && view.read_with(cx, |v, _| v.rows.iter().all(|r| r.stash().is_none())), "a folder's eye hides every stash");
    let saved = view.read_with(cx, |v, cx| v.sizes(cx).hidden);
    assert_eq!(saved.len(), 4, "the hidden set is the tab's to save: {saved:?}");
    assert_eq!(panel.read_with(cx, |p, _| p.counts()[3]), Count { shown: 0, total: 3 });
    panel.update(cx, |p, cx| p.show_all(cx));
    cx.run_until_parked();
    assert!(has(&view, &only, cx) && has(&view, f.sha("stash_feature_index"), cx), "Show all walks from every ref again");
}

#[gpui::test]
fn picking_a_ref_selects_its_commit_and_loads_pages_until_it_is_there(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    panel.update(cx, |p, cx| p.picked(&"local/feature".into(), cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(f.sha("rename").into())));
    panel.update(cx, |p, cx| p.picked(&"local".into(), cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(f.sha("rename").into())), "a section's header picks nothing");

    let g = Generated::new(4000);
    let deep = g.repo();
    let t1 = supraman_gitman::refs(&deep).unwrap().tags.into_iter().find(|t| t.name == "t1").and_then(|t| t.commit).unwrap();
    view.update(cx, |v, cx| {
        v.folder = g.dir.clone();
        v.reload(cx);
    });
    cx.run_until_parked();
    assert!(!has(&view, &t1, cx), "t1 lies past the first page");
    panel.update(cx, |p, cx| p.picked(&"tags/t1".into(), cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(t1.into())), "pages load until its row is there");
}

#[gpui::test]
fn ctrl_j_collapses_the_panel_to_its_strip_and_a_section_icon_opens_it_there(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let shown = |key: &str| hits.borrow().contains_key(key);
    assert!(shown("refpanel-row-local") && shown("refpanel-row-local/main") && !shown("refpanel-strip-local"));
    assert!(shown("refpanel-eye-local/feature") && shown("refpanel-eye-stashes/0") && shown("refpanel-eye-remote/origin") && !shown("refpanel-eye-local/main"), "an eye on every ref that hides; none on the current branch");
    let feature = hits.borrow().get("refpanel-row-local/feature").unwrap().bounds.center();
    cx.simulate_click(feature, gpui::Modifiers::none());
    redraw(&view, cx);
    cx.simulate_keystrokes("ctrl-j");
    hits.borrow_mut().clear();
    redraw(&view, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    assert!(panel.read_with(cx, |p, _| p.shut()));
    assert!(Section::ALL.iter().all(|s| shown(&format!("refpanel-strip-{}", s.key()))) && shown("refpanel-expand") && !shown("refpanel-row-local"), "the strip: expand, then a section's icon each");
    let strip = hits.borrow().get("refpanel-strip-local").unwrap().bounds;
    assert!(strip.size.width <= STRIP, "{strip:?}");
    panel.update(cx, |p, cx| {
        p.tree.toggle("tags");
        p.open_section(Section::Tags, cx)
    });
    hits.borrow_mut().clear();
    redraw(&view, cx);
    assert!(panel.read_with(cx, |p, _| !p.shut() && p.tree.is_open("tags") && p.tree.selected().as_deref() == Some("tags")));
    assert!(shown("refpanel-row-tags/v1"), "the panel open on its tags");
    view.update_in(cx, |v, window, cx| window.focus(v.table.focus_handle(), cx));
    cx.simulate_keystrokes("ctrl-alt-f");
    redraw(&view, cx);
    cx.simulate_input("lig");
    redraw(&view, cx);
    let rows: Vec<String> = panel.read_with(cx, |p, _| p.visible().iter().map(|p| p.to_string()).collect());
    assert_eq!(rows, ["tags", "tags/light"], "Ctrl+Alt+F types into the filter");
}

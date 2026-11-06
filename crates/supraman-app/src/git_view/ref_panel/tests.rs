use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, px};
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
    let panel = cx.update(|cx| cx.new(|cx| RefPanel::new(HitTargets::default(), cx.focus_handle(), cx)));
    panel.update(cx, |p, cx| {
        p.restore(&GitSizes { hidden, ..GitSizes::default() }, cx);
        p.set_refs(&refs(), cx);
    });
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
    let walked = without(&refs(), &Walk { hidden, ..Walk::default() });
    let names: Vec<&str> = walked.branches.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"main") && !names.contains(&"fix/a") && names.len() == 7, "{names:?}");
    assert_eq!(walked.remotes.len(), 2);
    assert_eq!(walked.stashes.iter().map(|s| s.sha.as_str()).collect::<Vec<_>>(), ["s0"]);
    assert_eq!(walked.worktrees.len(), 2, "the worktree on a hidden branch is no tip either");
    assert_eq!(without(&refs(), &Walk::default()), refs());
}

fn keys(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|k| k.to_string()).collect()
}

#[gpui::test]
fn soloing_walks_only_the_soloed_refs_and_head_and_counts_them(cx: &mut TestAppContext) {
    let walk = Walk { hidden: keys(&["refs/heads/fix/a", "refs/heads/zeta"]), solo: keys(&["refs/heads/fix/a", "refs/remotes/upstream/dev"]) };
    let walked = without(&refs(), &walk);
    let names: Vec<&str> = walked.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["main", "fix/a"], "the soloed branch, hidden or not, and the current one");
    assert_eq!(walked.remotes.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["upstream/dev"]);
    assert!(walked.stashes.is_empty(), "no stash is soloed");
    assert_eq!(walked.worktrees.iter().map(|w| w.path.to_string_lossy().into_owned()).collect::<Vec<_>>(), ["C:/src/repo"], "only the worktree on a walked branch");
    assert_eq!(walked.tags.len(), 3, "the tags still label");
    let tips = tips(&walked, &walk);
    assert_eq!(tips, ["m", "fix/a-sha", "upstream/dev-sha", "C:/src/repo-head"], "HEAD and the soloed refs; no tag is a tip");
    let panel = panel(cx, &["refs/heads/zeta"]);
    panel.update(cx, |p, cx| p.toggle_solo(&["refs/heads/fix/a".to_string(), "refs/remotes/upstream/dev".to_string()], cx));
    let words = |c: Count| (c.shown, c.total);
    assert_eq!(panel.read_with(cx, |p, _| p.counts().map(words)), [(2, 8), (1, 3), (3, 3), (0, 2), (3, 3)], "shown: the refs in the walk");
    let folder = panel.read_with(cx, |p, _| p.tree.aggregate("local/fix").unwrap().solo);
    panel.update(cx, |p, cx| p.toggle_solo(&folder, cx));
    assert_eq!(panel.read_with(cx, |p, _| p.walk().solo.clone()), keys(&["refs/heads/fix/a", "refs/heads/fix/b", "refs/remotes/upstream/dev"]), "a folder's solo solos all its refs");
    panel.update(cx, |p, cx| p.toggle_solo(&folder, cx));
    panel.update(cx, |p, cx| p.toggle_solo(&["refs/remotes/upstream/dev".to_string()], cx));
    assert!(!panel.read_with(cx, |p, _| p.walk().soloing()));
    assert_eq!(panel.read_with(cx, |p, _| p.counts()[0]), Count { shown: 7, total: 8 }, "the last solo cleared: the hidden set again");
    panel.update(cx, |p, cx| {
        p.toggle_solo(&["refs/heads/fix/a".to_string()], cx);
        p.show_all(cx);
    });
    assert_eq!(panel.read_with(cx, |p, _| p.walk().clone()), Walk::default(), "Show all clears both");
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
    let keys = panel.read_with(cx, |p, _| p.tree.aggregate("stashes").unwrap().hide);
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
    panel.update(cx, |p, cx| p.picked("local/feature", cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(f.sha("rename").into())));
    panel.update(cx, |p, cx| p.picked("local", cx));
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
    panel.update(cx, |p, cx| p.picked("tags/t1", cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(t1.into())), "pages load until its row is there");
}

fn click(key: &str, view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let at = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} not painted")).bounds.center();
    cx.simulate_click(at, gpui::Modifiers::none());
    hits.borrow_mut().clear();
    redraw(view, cx);
}

fn press(key: &str, view: &Entity<GitView>, button: gpui::MouseButton, click_count: usize, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let position = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} not painted")).bounds.center();
    let modifiers = gpui::Modifiers::none();
    cx.simulate_event(gpui::MouseDownEvent { position, button, modifiers, click_count, first_mouse: false });
    cx.simulate_event(gpui::MouseUpEvent { position, button, modifiers, click_count });
    hits.borrow_mut().clear();
    redraw(view, cx);
}

fn right_click(key: &str, view: &Entity<GitView>, cx: &mut VisualTestContext) {
    press(key, view, gpui::MouseButton::Right, 1, cx);
}

/// A branch with one commit of its own on `main`'s merge.
fn branch_off_merge(f: &Fixture, name: &str) -> String {
    let sha = f.git(&["commit-tree", "-p", f.sha("merge"), "-m", name, &format!("{}^{{tree}}", f.sha("merge"))]);
    f.git(&["branch", name, &sha]);
    sha
}

#[gpui::test]
fn a_solo_walks_only_its_branch_and_head_and_persists_with_the_layout(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (only, other) = (solo(&f), branch_off_merge(&f, "topic/other"));
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    assert!(has(&view, &only, cx) && has(&view, &other, cx) && has(&view, f.sha("stash_feature_index"), cx));
    right_click("refpanel-name-local/topic/solo", &view, cx);
    click("menu-ref-solo", &view, cx);
    assert!(has(&view, &only, cx) && has(&view, f.sha("ahead"), cx), "the soloed branch and HEAD walk");
    assert!(!has(&view, &other, cx), "another branch's own commit leaves the graph");
    assert!(view.read_with(cx, |v, _| v.rows.iter().all(|r| r.stash().is_none())), "and every stash");
    assert_eq!(view.read_with(cx, |v, cx| v.sizes(cx).solo), ["refs/heads/topic/solo"], "the solo is the tab's to save");
    let local = panel.read_with(cx, |p, _| p.counts()[0]);
    assert_eq!(local.shown, 2, "LOCAL counts main and topic/solo: {local:?}");
    let sizes = view.read_with(cx, |v, cx| v.sizes(cx));
    let back = serde_json::from_value::<GitSizes>(serde_json::to_value(&sizes).unwrap()).unwrap();
    let restored = cx.update(|_, cx| cx.new(|cx| RefPanel::new(HitTargets::default(), cx.focus_handle(), cx)));
    restored.update(cx, |p, cx| p.restore(&back, cx));
    assert_eq!(restored.read_with(cx, |p, _| p.walk().clone()), panel.read_with(cx, |p, _| p.walk().clone()), "the solo comes back from the saved sizes");
    click("refpanel-mark-local/topic/solo", &view, cx);
    assert!(has(&view, &other, cx) && has(&view, f.sha("stash_feature_index"), cx), "the last solo cleared: every ref walks again");
}

#[gpui::test]
fn ctrl_j_right_after_show_all_collapses_the_panel(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    click("refpanel-mark-local/feature", &view, cx);
    cx.update(|window, cx| window.blur(cx));
    click("refpanel-show-all", &view, cx);
    cx.simulate_keystrokes("ctrl-j");
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    assert!(panel.read_with(cx, |p, _| p.shut()), "Ctrl+J reaches the tab after a click on Show all");
}

#[gpui::test]
fn ctrl_j_collapses_the_panel_to_its_strip(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let shown = |key: &str| hits.borrow().contains_key(key);
    assert!(shown("refpanel-row-local") && shown("refpanel-row-local/main") && !shown("refpanel-strip-local"));
    assert!(["local/feature", "local/main", "stashes/0", "remote/origin"].iter().all(|p| shown(&format!("refpanel-mark-{p}"))), "a mark on every ref and folder that hides or solos");
    assert!(!shown("refpanel-mark-tags/v1"), "none on a tag, which does neither");
    click("refpanel-row-local/feature", &view, cx);
    cx.simulate_keystrokes("ctrl-j");
    hits.borrow_mut().clear();
    redraw(&view, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    assert!(panel.read_with(cx, |p, _| p.shut()));
    assert!(Section::ALL.iter().all(|s| shown(&format!("refpanel-strip-{}", s.key()))) && shown("refpanel-expand") && !shown("refpanel-row-local"), "the strip: expand, then a section's icon each");
    let strip = hits.borrow().get("refpanel-strip-local").unwrap().bounds;
    assert!(strip.size.width <= STRIP, "{strip:?}");
    view.update_in(cx, |v, window, cx| window.focus(v.table.focus_handle(), cx));
    cx.simulate_keystrokes("ctrl-alt-f");
    redraw(&view, cx);
    cx.simulate_input("lig");
    redraw(&view, cx);
    let rows: Vec<String> = panel.read_with(cx, |p, _| p.visible().iter().map(|p| p.to_string()).collect());
    assert_eq!(rows, ["tags", "tags/light"], "Ctrl+Alt+F opens the panel and types into the filter");
}

#[gpui::test]
fn a_strip_icon_opens_its_sections_overlay_over_the_graph_and_a_pick_selects_and_closes_it(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    panel.update(cx, |p, cx| p.toggle_shut(cx));
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let overlay = |cx: &mut VisualTestContext| panel.read_with(cx, |p, _| p.overlay());
    click("refpanel-strip-local", &view, cx);
    assert_eq!(overlay(cx), Some(Section::Local));
    let rows: Vec<String> = panel.read_with(cx, |p, _| p.overlay_rows().iter().map(|r| r.to_string()).collect());
    assert_eq!(rows, ["feature", "main", "side", "stale"], "just LOCAL's refs, below its header");
    let card = hits.borrow().get("refpanel-overlay").unwrap().bounds;
    let graph = hits.borrow().get("git-row-0").unwrap().bounds;
    assert!(card.left() >= STRIP && card.left() < graph.right() && card.intersects(&graph), "beside the strip, over the graph: {card:?}");
    assert!(panel.read_with(cx, |p, _| p.shut()) && hits.borrow().contains_key("refpanel-overlay-mark-feature"), "the panel stays collapsed; the rows have their marks");
    click("refpanel-overlay-row-feature", &view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(f.sha("rename").into())), "a pick selects its commit");
    assert_eq!(overlay(cx), None, "and closes the overlay");
    click("refpanel-strip-tags", &view, cx);
    assert_eq!(overlay(cx), Some(Section::Tags));
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay(cx), None, "Esc closes it");
    cx.simulate_keystrokes("ctrl-j");
    assert!(!panel.read_with(cx, |p, _| p.shut()), "the keyboard back in the tab");
    panel.update(cx, |p, cx| p.toggle_shut(cx));
    click("refpanel-strip-remote", &view, cx);
    let row = hits.borrow().get("git-row-2").unwrap().bounds;
    cx.simulate_click(gpui::point(row.right() - px(8.), row.center().y), gpui::Modifiers::none());
    assert_eq!(overlay(cx), None, "a click outside closes it");
    click("refpanel-strip-remote", &view, cx);
    click("refpanel-strip-remote", &view, cx);
    assert_eq!(overlay(cx), None, "a second click on its icon closes it");
}

#[gpui::test]
fn ref_panel_and_a_sections_overlay_paint_their_rows_whole_in_their_first_frame(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let width = |key: &str| hits.borrow().get(key).map(|h| h.bounds.size.width).unwrap_or_else(|| panic!("{key} not painted"));
    assert!(width("refpanel-name-local/feature") > px(30.), "the open panel's names whole");
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    panel.update(cx, |p, cx| p.toggle_shut(cx));
    redraw(&view, cx);
    panel.update_in(cx, |p, window, cx| p.toggle_overlay(Section::Local, window, cx));
    let hits = crate::git_view::details::tests::first_frame(&view, cx, |cx| cx.run_until_parked());
    let width = |key: &str| hits.borrow().get(key).map(|h| h.bounds.size.width).unwrap_or_else(|| panic!("{key} not painted"));
    assert!(width("refpanel-overlay-name-feature") > px(30.), "the overlay's names whole in the frame it first shows");
}

#[gpui::test]
fn a_sections_overlay_shows_nothing_until_its_tree_is_built(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    panel.update(cx, |p, cx| p.toggle_shut(cx));
    redraw(&view, cx);
    panel.update_in(cx, |p, window, cx| p.toggle_overlay(Section::Local, window, cx));
    assert!(panel.read_with(cx, |p, _| p.overlay_rows().is_empty()), "built off the UI thread");
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    cx.update(|window, cx| {
        let drawn = window.draw(cx);
        drawn.clear(cx);
    });
    assert!(!hits.borrow().contains_key("refpanel-overlay"), "no card without its rows");
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |p, _| p.overlay_rows().len()), 4, "then the card with all its rows");
}

#[gpui::test]
fn a_sections_border_drags_height_between_its_neighbours_and_the_heights_persist(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let at = |key: &str| hits.borrow().get(key).unwrap_or_else(|| panic!("{key} not painted")).bounds;
    let (local, remote) = (at("refpanel-row-local"), at("refpanel-row-remote"));
    let start = at("refpanel-sections-divider-1").center();
    assert!((start.y - remote.top()).abs() <= px(4.), "the border above REMOTE's header");
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(start + gpui::point(px(0.), px(20.)), gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(start + gpui::point(px(0.), px(48.)), gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_up(start + gpui::point(px(0.), px(48.)), gpui::MouseButton::Left, gpui::Modifiers::none());
    redraw(&view, cx);
    let moved = at("refpanel-row-remote");
    assert!((moved.top() - remote.top() - px(48.)).abs() <= px(1.), "REMOTE's header follows the border: {remote:?} to {moved:?}");
    assert_eq!(at("refpanel-row-local").top(), local.top(), "LOCAL's header stays");
    let sizes = view.read_with(cx, |v, cx| v.sizes(cx));
    assert_eq!(sizes.sections.len(), 5, "the tab keeps every section's height: {:?}", sizes.sections);
    let back = serde_json::from_value::<GitSizes>(serde_json::to_value(&sizes).unwrap()).unwrap();
    assert_eq!(back.sections, sizes.sections, "they round-trip through layout.json");

    click("refpanel-row-remote", &view, cx);
    assert!(!hits.borrow().contains_key("refpanel-row-remote/origin"), "a closed section is its header alone");
    let header = at("refpanel-row-remote");
    let next = Section::ALL.iter().skip(2).find_map(|s| hits.borrow().get(&format!("refpanel-row-{}", s.key())).map(|h| h.bounds));
    assert!(next.is_some_and(|n| (n.top() - header.bottom()).abs() <= px(1.)), "the next section's header right under it");
}

#[gpui::test]
fn every_ref_rows_mark_sits_at_the_rows_left_edge_whatever_its_depth(cx: &mut TestAppContext) {
    let f = Fixture::new();
    solo(&f);
    let (view, cx) = open(&f.dir, cx);
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    redraw(&view, cx);
    let bounds = |key: &str| hits.borrow().get(key).unwrap_or_else(|| panic!("{key} not painted")).bounds;
    let (deep, top, folder) = (bounds("refpanel-mark-local/topic/solo"), bounds("refpanel-mark-local/feature"), bounds("refpanel-mark-local/topic"));
    let row = bounds("refpanel-row-local/topic/solo");
    assert!(deep.left() == top.left() && deep.left() == folder.left(), "one column: {deep:?} {top:?} {folder:?}");
    assert!((deep.left() - row.left()).abs() <= px(1.), "at the row's left edge: {deep:?} in {row:?}");
    assert!(bounds("refpanel-name-local/topic/solo").left() > bounds("refpanel-name-local/feature").left(), "the names keep their indentation");
}

#[gpui::test]
fn a_rows_mark_is_the_current_tick_or_hides_or_shows_or_stops_soloing(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    let walk = |cx: &mut VisualTestContext| panel.read_with(cx, |p, _| p.walk().clone());
    click("refpanel-mark-local/main", &view, cx);
    assert_eq!(walk(cx), Walk::default(), "the current branch's tick does nothing");
    click("refpanel-mark-local/feature", &view, cx);
    assert_eq!(walk(cx).hidden, keys(&["refs/heads/feature"]), "an eye hides its ref");
    click("refpanel-mark-local/feature", &view, cx);
    assert_eq!(walk(cx), Walk::default(), "a hidden ref's mark shows it again");
    panel.update(cx, |p, cx| p.toggle_solo(&["refs/heads/feature".into()], cx));
    click("refpanel-mark-local/feature", &view, cx);
    assert_eq!(walk(cx), Walk::default(), "a soloed ref's mark stops soloing");
}

#[gpui::test]
fn a_refs_menu_solos_and_hides_and_has_no_hide_for_the_current_branch(cx: &mut TestAppContext) {
    let panel = panel(cx, &["refs/heads/zeta"]);
    let words = |path: &str, cx: &mut TestAppContext| panel.read_with(cx, |p, _| p.menu_items(path).iter().map(|i| i.label.to_string()).collect::<Vec<_>>());
    assert_eq!(words("local/main", cx), ["Solo"], "no Hide for the current branch");
    assert_eq!(words("local/zeta", cx), ["Solo", "Show"]);
    assert_eq!(words("local/fix", cx), ["Solo", "Hide"], "a folder's menu acts on its refs");
    assert!(words("tags/v1", cx).is_empty(), "a tag neither hides nor solos: no menu");
    panel.update(cx, |p, cx| p.toggle_solo(&["refs/heads/Alpha".into()], cx));
    assert_eq!(words("local/Alpha", cx), ["Stop soloing", "Hide"]);
}

#[gpui::test]
fn a_right_click_opens_the_refs_menu_and_its_solo_solos_it(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    right_click("refpanel-name-local/feature", &view, cx);
    click("menu-ref-solo", &view, cx);
    assert_eq!(panel.read_with(cx, |p, _| p.walk().solo.clone()), keys(&["refs/heads/feature"]));
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    assert!(!hits.borrow().contains_key("menu-ref-solo"), "a pick closes the menu");
}

#[gpui::test]
fn a_double_click_on_a_hidden_ref_shows_it_and_selects_its_commit(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let only = solo(&f);
    let (view, cx) = open(&f.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    click("refpanel-mark-local/topic/solo", &view, cx);
    assert!(!has(&view, &only, cx), "hidden: its commit leaves the graph");
    press("refpanel-name-local/topic/solo", &view, gpui::MouseButton::Left, 1, cx);
    press("refpanel-name-local/topic/solo", &view, gpui::MouseButton::Left, 2, cx);
    cx.run_until_parked();
    assert_eq!(panel.read_with(cx, |p, _| p.walk().clone()), Walk::default(), "shown again");
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(only.into())), "and its commit selected");
}

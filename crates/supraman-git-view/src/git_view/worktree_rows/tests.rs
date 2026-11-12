//! The other worktrees' WIP rows through the real tab: where they show, what they say, and Open Worktree.

use std::path::{Path, PathBuf};

use gpui::{Entity, MouseButton, TestAppContext, VisualTestContext};
use supraman_core::paths::path_key;
use supraman_gitman::BackgroundSettings;
use supraman_gitman::fixture::Fixture;

use crate::git_view::background::{self, refresh_wips};
use crate::git_view::test_support::{self, Opts, QUIET, opened, painted, press, redraw, repaint};
use crate::git_view::{GitView, Selected, cells};
use supraman_ui::icon::Icon;
use crate::occupancy;

/// The fixture with a second linked worktree, `extra`, detached at `root`.
fn fixture() -> (Fixture, PathBuf) {
    let f = Fixture::copy();
    let extra = f.temp.path().join("extra");
    f.git(&["worktree", "add", "-q", "--detach", &extra.to_string_lossy(), f.sha("root")]);
    (f, extra)
}

fn open<'a>(f: &Fixture, live: occupancy::Live, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    test_support::open(&f.dir, Opts::default().following(live).background().size(1400., 900.), cx)
}

fn reread(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let id = view.entity_id();
    cx.update(|_, cx| refresh_wips(id, cx));
    cx.run_until_parked();
    redraw(view, cx);
}

fn head(dir: &Path, f: &Fixture) -> String {
    f.at(dir, &["rev-parse", "HEAD"])
}

/// Each worktree row's folder and the sha of the row below it.
fn worktree_rows(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(String, String)> {
    view.read_with(cx, |v, cx| {
        v.pages.read(cx).rows.iter().enumerate().filter_map(|(ix, r)| Some((path_key(r.worktree()?), v.pages.read(cx).rows.get(ix + 1).map(|b| b.sha.to_string()).unwrap_or_default()))).collect()
    })
}

fn session(name: &str, id: &str, cwd: PathBuf) -> occupancy::LiveSession {
    occupancy::LiveSession { name: name.into(), claude_session_id: id.into(), cwd, activity: supraman_ui::marks::Activity::Working, attach: None }
}

fn row_of(view: &Entity<GitView>, dir: &Path, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.worktree().is_some_and(|p| path_key(p) == path_key(dir)))).expect("a row of the worktree")
}

#[gpui::test]
fn a_worktree_row_shows_above_its_head_only_while_changed_or_worked_in_and_not_when_the_setting_is_off(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    let live = occupancy::Live::default();
    let (view, cx) = open(&f, live.clone(), cx);
    assert_eq!(worktree_rows(&view, cx), [], "both worktrees clean, no session in them");

    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    reread(&view, cx);
    assert_eq!(worktree_rows(&view, cx), [(path_key(&f.worktree), head(&f.worktree, &f))], "a change: its row right above its HEAD");

    live.set_sessions(vec![session("fm-alpha", "aa", extra.join("src"))]);
    reread(&view, cx);
    let mut rows = worktree_rows(&view, cx);
    rows.sort();
    let mut expected = vec![(path_key(&f.worktree), head(&f.worktree, &f)), (path_key(&extra), f.sha("root").to_string())];
    expected.sort();
    assert_eq!(rows, expected, "a live session below a clean worktree shows it too");
    let commits = view.read_with(cx, |v, cx| (v.pages.read(cx).commits, v.pages.read(cx).rows.iter().filter(|r| r.commit()).count()));
    assert_eq!(commits.0, commits.1, "the worktree rows count as no commits");

    view.update(cx, |v, cx| {
        let ix = v.pages.read(cx).rows.iter().position(|r| r.worktree().is_some()).unwrap();
        v.table.select(Some(ix));
        v.selection_moved(cx);
    });
    assert!(matches!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Worktree(_))));

    let reads = cx.update(|_, cx| background::wips(view.entity_id(), cx).map(|w| w.read(cx).reads)).unwrap();
    cx.update(|_, cx| background::apply(BackgroundSettings { worktree_wip: Some(false), ..QUIET }, cx));
    cx.run_until_parked();
    assert_eq!(worktree_rows(&view, cx), [], "off: no worktree rows");
    reread(&view, cx);
    assert_eq!(cx.update(|_, cx| background::wips(view.entity_id(), cx).map(|w| w.read(cx).reads)), Some(reads), "off: nothing read");

    cx.update(|_, cx| background::apply(QUIET, cx));
    cx.run_until_parked();
    assert_eq!(worktree_rows(&view, cx).len(), 2, "on again: read and shown");
}

#[gpui::test]
fn a_worktree_row_says_its_counts_and_branch_or_detached_head_and_its_sessions_stand_on_it_as_agent_rows(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    let live = occupancy::Live::default();
    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    std::fs::write(extra.join("a.txt"), b"changed\n").unwrap();
    let (view, cx) = open(&f, live.clone(), cx);
    reread(&view, cx);

    let side = row_of(&view, &f.worktree, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(side, cx)).expect("read");
    assert_eq!((said.label.as_str(), said.counts.as_str()), ("side", "+1"));
    assert!(painted(&view, &format!("git-worktree-wip-{side}"), cx), "muted // WIP");
    assert!(painted(&view, &format!("git-worktree-ref-{side}"), cx));

    live.set_sessions(vec![session("fm-alpha", "aa", extra.clone())]);
    redraw(&view, cx);
    let detached = row_of(&view, &extra, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(detached, cx)).expect("read");
    assert_eq!((said.label.as_str(), said.counts.as_str()), ("Detached HEAD", "✎1"));
    assert_eq!(view.read_with(cx, |v, cx| v.agent_rows(cx)), [(detached - 1, "aa".to_string())], "the session's agent row right above its worktree's");
    assert!(painted(&view, &format!("git-agent-{}", detached - 1), cx) && painted(&view, &format!("git-agent-state-{}", detached - 1), cx), "the agent row's lead with its state");
    assert!(painted(&view, &format!("git-worktree-wip-{detached}"), cx), "the worktree row keeps // WIP");
    assert!(!painted(&view, &format!("git-worktree-session-{detached}-0"), cx), "no lead on the worktree row");
}

#[gpui::test]
fn double_click_and_open_worktree_ask_for_the_worktrees_tab_and_a_detached_ones_double_click_does_nothing(cx: &mut TestAppContext) {
    let (f, extra) = fixture();
    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    std::fs::write(extra.join("a.txt"), b"changed\n").unwrap();
    let (view, cx) = open(&f, occupancy::Live::default(), cx);
    reread(&view, cx);
    let seen = opened(&view, cx);

    let side = row_of(&view, &f.worktree, cx);
    press(&view, &format!("git-worktree-ref-{side}"), MouseButton::Left, 2, cx);
    assert_eq!(*seen.borrow(), [path_key(&f.worktree)], "double-click opens it");

    let detached = row_of(&view, &extra, cx);
    press(&view, &format!("git-worktree-ref-{detached}"), MouseButton::Left, 2, cx);
    assert_eq!(seen.borrow().len(), 1, "a detached worktree's double-click does nothing");
    assert_eq!(view.read_with(cx, |v, _| v.table.selected()), Some(detached), "the click selects it");
    press(&view, &format!("git-worktree-open-{detached}"), MouseButton::Left, 1, cx);
    assert_eq!(seen.borrow().last(), Some(&path_key(&extra)), "Open Worktree opens even a detached one");
}

/// The agent rows right above row `ix`, top down.
fn stacked_on(view: &Entity<GitView>, ix: usize, cx: &mut VisualTestContext) -> Vec<String> {
    let agents = view.read_with(cx, |v, cx| v.agent_rows(cx));
    let mut above: Vec<String> = Vec::new();
    for at in (0..ix).rev() {
        match agents.iter().find(|(i, _)| *i == at) {
            Some((_, id)) => above.insert(0, id.clone()),
            None => break,
        }
    }
    above
}

/// The tab at `own` shows the sessions in `other` as agent rows on its worktree row: their rendering, selection, panel and menu.
fn other_checkouts_sessions_are_agent_rows(own: &Path, other: &Path, cx: &mut TestAppContext) {
    let live = occupancy::Live::default();
    let (view, cx) = test_support::open(own, Opts::default().following(live.clone()).background().size(1400., 900.), cx);
    live.set_sessions(vec![session("fm-beta", "bb", other.join("src")), session("fm-alpha", "aa", other.to_path_buf()), session("fm-here", "hh", own.to_path_buf())]);
    reread(&view, cx);

    let ix = row_of(&view, other, cx);
    assert_eq!(stacked_on(&view, ix, cx), ["aa", "bb"], "a row per session, by name, stacked on the other checkout's row");
    let head = view.read_with(cx, |v, _| v.heads.head.clone()).expect("HEAD");
    let own_row = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.wip() || r.sha == head)).expect("the tab's own row");
    assert_eq!(stacked_on(&view, own_row, cx), ["hh"], "the tab's own session on its own row");
    for n in 0..2 {
        assert!(!painted(&view, &format!("git-worktree-session-{ix}-{n}"), cx), "no lead {n} on the worktree row");
    }
    let hits = repaint(&view, cx);
    let commit = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.commit())).expect("a commit row");
    let (agent, text) = (hits.borrow()[&format!("git-agent-{}", ix - 2)], hits.borrow()[&format!("git-text-{commit}")]);
    assert_eq!(agent.bounds.left(), text.bounds.left(), "the agent's lead and a commit's message start at one x");

    press(&view, &format!("git-agent-{}", ix - 1), MouseButton::Left, 1, cx);
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Agent("bb".into())));
    assert!(painted(&view, "git-agent-section-session", cx), "its panel");

    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let s = seen.clone();
    cx.update(|_, cx| cx.subscribe(&view, move |_, e: &crate::git_view::agent_rows::AgentMenu, _| s.borrow_mut().push(e.session.name.clone())).detach());
    press(&view, &format!("git-agent-{}", ix - 2), MouseButton::Right, 1, cx);
    assert_eq!(*seen.borrow(), ["fm-alpha"], "its right-click is the session's menu");

    live.set_sessions(vec![session("fm-beta", "bb", other.join("src"))]);
    reread(&view, cx);
    let ix = row_of(&view, other, cx);
    assert_eq!(stacked_on(&view, ix, cx), ["bb"], "an ended session's row goes");
    assert_eq!(view.read_with(cx, |v, cx| v.selected(cx)), Some(Selected::Agent("bb".into())), "the selection stays on its session");
}

#[gpui::test]
fn in_the_main_tab_a_linked_worktrees_sessions_are_agent_rows_on_its_row(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    other_checkouts_sessions_are_agent_rows(&f.dir, &f.worktree, cx);
}

#[gpui::test]
fn in_a_worktree_tab_the_main_checkouts_sessions_are_agent_rows_on_its_row(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    other_checkouts_sessions_are_agent_rows(&f.worktree, &f.dir, cx);
}

#[gpui::test]
fn in_a_worktree_tab_the_main_checkouts_row_and_its_branchs_pill_carry_the_home_icon(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.dir.join("new.txt"), b"new\n").unwrap();
    let (view, cx) = test_support::open(&f.worktree, Opts::default().following(occupancy::Live::default()).background().size(1400., 900.), cx);
    reread(&view, cx);

    let main = row_of(&view, &f.dir, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(main, cx)).expect("read");
    assert!(said.main);
    assert_eq!(said.icons(), [Icon::Local, Icon::Home], "the main checkout's row: home, not the tree");
    assert!(painted(&view, &format!("git-worktree-label-{main}-icon-1"), cx));

    let branch = f.at(&f.dir, &["branch", "--show-current"]);
    let icons = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().find_map(|r| r.row.refs.iter().find(|g| g.name == branch).map(cells::kind_icons))).expect("the main checkout's branch's pill");
    assert_eq!(icons.last(), Some(&Icon::Home), "its branch's pill: {icons:?}");
}

#[gpui::test]
fn in_the_main_tab_a_linked_worktrees_row_and_pill_keep_the_tree(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    let (view, cx) = test_support::open(&f.dir, Opts::default().following(occupancy::Live::default()).background().size(1400., 900.), cx);
    reread(&view, cx);

    let side = row_of(&view, &f.worktree, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(side, cx)).expect("read");
    assert_eq!((said.main, said.icons()), (false, vec![Icon::Local, Icon::Worktree]));
    let icons = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().find_map(|r| r.row.refs.iter().find(|g| g.name == "side").map(cells::kind_icons))).expect("side's pill");
    assert_eq!(icons.last(), Some(&Icon::Worktree), "{icons:?}");
}

/// Open Worktree on another checkout's row, neither hovered nor selected.
fn open_worktree_shows_at_rest(own: &Path, other: &Path, cx: &mut TestAppContext) {
    std::fs::write(other.join("new.txt"), b"new\n").unwrap();
    let (view, cx) = test_support::open(own, Opts::default().following(occupancy::Live::default()).background().size(1400., 900.), cx);
    reread(&view, cx);
    let ix = row_of(&view, other, cx);
    assert_ne!(view.read_with(cx, |v, _| v.table.selected()), Some(ix));
    let hits = repaint(&view, cx);
    let open = hits.borrow()[&format!("git-worktree-open-{ix}")].clone();
    assert!(open.fully_visible(), "{open:?}");
    let seen = opened(&view, cx);
    press(&view, &format!("git-worktree-open-{ix}"), MouseButton::Left, 1, cx);
    assert_eq!(*seen.borrow(), [path_key(other)]);
}

#[gpui::test]
fn open_worktree_shows_on_a_linked_worktrees_row_in_the_main_tab(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    open_worktree_shows_at_rest(&f.dir, &f.worktree, cx);
}

#[gpui::test]
fn open_worktree_shows_on_the_main_checkouts_row_in_a_worktree_tab(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    open_worktree_shows_at_rest(&f.worktree, &f.dir, cx);
}

#[gpui::test]
fn a_conflicted_worktree_row_shows_the_conflict_node_and_its_upstreams_ahead_and_behind(cx: &mut TestAppContext) {
    let (f, _) = fixture();
    f.cherry_pick_conflict();
    f.at(&f.worktree, &["branch", "-q", "-u", "origin/main"]);
    let (view, cx) = open(&f, occupancy::Live::default(), cx);
    reread(&view, cx);

    let side = row_of(&view, &f.worktree, cx);
    let said = view.update(cx, |v, cx| v.worktree_view(side, cx)).expect("read");
    assert_eq!(said.conflicted, 1, "a.txt");
    assert!(!said.upstream.is_empty(), "side and origin/main differ");
    for key in [format!("git-conflict-node-{side}"), format!("git-worktree-upstream-{side}")] {
        assert!(painted(&view, &key, cx), "{key}");
    }
}

/// The groups row `ix`'s Branch / Tag cell shows with its list open, ghost included.
fn shown_groups(view: &Entity<GitView>, ix: usize, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        let shown = cells::ref_cell(&v.pages.read(cx).rows[ix].row, true);
        shown.first.into_iter().chain(shown.list).map(|g| g.name).collect()
    })
}

fn group_menu(view: &Entity<GitView>, ix: usize, name: &str, cx: &mut VisualTestContext) -> Vec<String> {
    let group = view.read_with(cx, |v, cx| v.pages.read(cx).rows[ix].row.refs.iter().position(|g| g.name == name)).unwrap_or_else(|| panic!("a pill {name}"));
    view.update(cx, |v, cx| v.pill_menu(ix, group, gpui::point(gpui::px(300.), gpui::px(300.)), cx));
    cx.run_until_parked();
    test_support::menu(&view, cx)
}

#[gpui::test]
fn a_shown_worktree_rows_branch_is_on_that_row_only_and_its_pill_keeps_the_branch_menu(cx: &mut TestAppContext) {
    let (f, _) = fixture();
    let (view, cx) = open(&f, occupancy::Live::default(), cx);
    let side_head = head(&f.worktree, &f);
    let commit = test_support::row_of(&view, &side_head, cx);
    assert!(shown_groups(&view, commit, cx).contains(&"side".to_string()), "no worktree row: the commit shows side");
    let commit_menu = group_menu(&view, commit, "side", cx);
    assert!(commit_menu.iter().any(|l| l == "Remove worktree"), "{commit_menu:?}");

    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    reread(&view, cx);
    let row = row_of(&view, &f.worktree, cx);
    let commit = test_support::row_of(&view, &side_head, cx);
    assert_eq!(commit, row + 1, "the worktree row right above its HEAD");
    assert_eq!(view.read_with(cx, |v, cx| v.pages.read(cx).rows[row].row.refs.iter().map(|g| g.name.clone()).collect::<Vec<_>>()), ["side"]);
    assert!(painted(&view, &format!("git-worktree-ref-{row}"), cx), "the worktree row shows side");
    let shown = shown_groups(&view, commit, cx);
    assert!(!shown.contains(&"side".to_string()), "its commit leaves side out: {shown:?}");
    assert_eq!(view.read_with(cx, |v, cx| v.pages.read(cx).rows[commit].row.more()), shown.len().saturating_sub(1), "the +N counts the groups shown");

    test_support::right_click(&view, &format!("git-worktree-ref-{row}"), cx);
    assert_eq!(test_support::menu(&view, cx), commit_menu, "the worktree row's pill gives the menu the commit's pill gave");

    cx.update(|_, cx| background::apply(BackgroundSettings { worktree_wip: Some(false), ..QUIET }, cx));
    cx.run_until_parked();
    let commit = test_support::row_of(&view, &side_head, cx);
    assert!(shown_groups(&view, commit, cx).contains(&"side".to_string()), "worktree_wip off: the commit shows side again");
}

use std::path::Path;

use gpui::{Entity, VisualTestContext, point, px};
use supraman_core::testing::{eventually, polled};
use supraman_gitman::fixture::Fixture;
use supraman_ui::dock::layout::Layout;
use supraman_ui::split::Axis;

use super::*;
use crate::view::dock::tests::{WITHIN, draw, git_panel, keys, loaded, shell, window};

fn dock(holds: Vec<PanelKey>, room: bool) -> DockFacts {
    DockFacts { tab: 7, holds, room }
}

fn why(item: &MenuItem) -> Option<&str> {
    item.dimmed.as_deref()
}

const SHOP: &str = r"C:\shop";

#[test]
fn tab_menu_add_items_follow_open_workman_and_are_live_with_room() {
    let (folder, top) = (Path::new(r"C:\shop\src"), Repo::Top(SHOP.into()));
    let bound = Work::Bound { target: Target::Folder(SHOP.into()), project: "p-1".into() };
    let details = MenuTarget::Folder("shop".into());
    let free = dock(vec![PanelKey::Terminal(7)], true);
    let adds = [add_git_item(Some(folder), &top, &free), add_work_item(Some(folder), &bound, &free), add_details_item(Some(folder), Some(&details), &free)];
    let items = with_adds(super::super::with_workman(super::super::tab_items(super::super::TabFacts { id: 7, folder: Some(folder), repo: &top, others: true, right: true, reopenable: true }, None), Some(folder), &bound), adds);
    let keys: Vec<&str> = items.iter().map(|i| &*i.key).collect();
    assert_eq!(keys[..5], ["tab-open-git", "tab-open-workman", "tab-add-git", "tab-add-work", "tab-add-details"]);
    assert!(items[2..5].iter().all(|i| i.dimmed.is_none()), "{:?}", items[2..5].iter().map(|i| &i.dimmed).collect::<Vec<_>>());
    assert_eq!(items[2].label, "Add Git shop panel");
    assert_eq!(items[2].action, MenuAction::AddPanel { tab: 7, added: Added::Git(SHOP.into()) });
    assert_eq!(items[3].action, MenuAction::AddPanel { tab: 7, added: Added::Work { target: Target::Folder(SHOP.into()), project: "p-1".into() } });
    assert_eq!(items[4].action, MenuAction::AddPanel { tab: 7, added: Added::Details(details) });
}

#[test]
fn tab_menu_add_items_are_dimmed_with_why() {
    let folder = Some(Path::new(r"C:\shop\src"));
    let free = dock(vec![], true);
    assert_eq!(why(&add_git_item(folder, &Repo::Reading, &free)), Some("Looking for its repository…"));
    assert_eq!(why(&add_git_item(folder, &Repo::Outside, &free)), Some("Not in a git repository"));
    assert_eq!(why(&add_git_item(None, &Repo::Outside, &free)), Some("This tab has no folder"));
    assert_eq!(why(&add_work_item(folder, &Work::Reading, &free)), Some("Looking for its Workman project…"));
    assert_eq!(why(&add_work_item(folder, &Work::Unbound, &free)), Some("Not bound to a Workman project"));
    assert_eq!(why(&add_work_item(folder, &Work::NotListed, &free)), Some("Not in a listed folder"));
    assert_eq!(why(&add_work_item(folder, &Work::Unavailable("Workman is not signed in.".into()), &free)), Some("Workman is not signed in."));
    assert_eq!(why(&add_details_item(folder, None, &free)), Some("Not in a listed folder"));
    assert_eq!(why(&add_details_item(None, None, &free)), Some("This tab has no folder"));

    let top = Repo::Top(SHOP.into());
    let target = Target::Folder(SHOP.into());
    let bound = Work::Bound { target: target.clone(), project: "p-1".into() };
    let details = MenuTarget::Session { entry: "e".into(), place: "p".into(), session_id: "s-1".into() };
    let holding = dock(vec![PanelKey::Git(path_key(Path::new(SHOP))), PanelKey::Work(target), PanelKey::details(&details)], true);
    assert_eq!(why(&add_git_item(folder, &top, &holding)), Some(HERE));
    assert_eq!(why(&add_work_item(folder, &bound, &holding)), Some(HERE));
    assert_eq!(why(&add_details_item(folder, Some(&details), &holding)), Some(HERE));
    let full = dock(vec![PanelKey::Terminal(7), PanelKey::Terminal(8), PanelKey::Terminal(9)], false);
    assert_eq!(why(&add_git_item(folder, &top, &full)), Some(FULL));
    assert_eq!(why(&add_work_item(folder, &bound, &full)), Some(FULL));
    assert_eq!(why(&add_details_item(folder, Some(&details), &full)), Some(FULL));
}

#[test]
fn tab_menu_add_git_stays_live_beside_a_git_panel_of_another_folder() {
    let other = dock(vec![PanelKey::Terminal(7), PanelKey::Git(path_key(Path::new(r"C:\other")))], true);
    assert_eq!(why(&add_git_item(Some(Path::new(SHOP)), &Repo::Top(SHOP.into()), &other)), None);
}

/// A claude shell tab in `cwd`.
fn claude_tab(view: &Entity<FleetView>, cwd: &Path, cx: &mut VisualTestContext) -> u64 {
    let (id, _) = shell(view, cx);
    view.update(cx, |v, _| {
        let tab = v.tab_mut(id).expect("the shell tab");
        tab.cwd = Some(cwd.to_path_buf());
        tab.claude = Some(format!("claude-{id}"));
    });
    id
}

/// Tab `id`'s menu opened, its Add Git live once the repository read lands, and chosen.
fn choose_add_git(view: &Entity<FleetView>, id: u64, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.open_tab_menu(id, point(px(10.), px(10.)), cx));
    let action = eventually(WITHIN, "Add Git live", polled(std::time::Duration::from_millis(20)), || {
        draw(view, cx);
        view.read_with(cx, |v, _| v.dialogs.menu.as_ref()?.items.iter().find(|i| i.key == "tab-add-git" && i.dimmed.is_none()).map(|i| i.action.clone()))
    });
    view.update(cx, |v, cx| {
        v.dialogs.menu = None;
        v.run_tab_action(action, cx);
    });
    draw(view, cx);
}

fn layout(view: &Entity<FleetView>, tab: u64, cx: &mut VisualTestContext) -> Layout<u64> {
    view.read_with(cx, |v, _| v.docks.of(tab).expect("a dock").layout.clone())
}

fn leaf(p: u64) -> Box<Layout<u64>> {
    Box::new(Layout::Leaf(p))
}

fn split(axis: Axis, first: Box<Layout<u64>>, second: Box<Layout<u64>>, ratio: f32) -> Layout<u64> {
    Layout::Split { axis, ratio, first, second }
}

fn ratio(l: &Layout<u64>) -> f32 {
    match l {
        Layout::Split { ratio, .. } => *ratio,
        Layout::Leaf(_) => 0.,
    }
}

fn git_of(view: &Entity<FleetView>, folder: &Path, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |v, cx| v.find_panel(&PanelKey::Git(path_key(folder)), cx)).expect("its Git panel")
}

#[gpui::test]
fn tab_menu_add_git_on_a_claude_tab_docks_it_right_then_below_on_the_right(cx: &mut gpui::TestAppContext) {
    let (a, b) = (Fixture::shared(), Fixture::copy());
    let (view, cx) = window(cx);
    let tab = claude_tab(&view, &a.dir, cx);
    choose_add_git(&view, tab, cx);
    let git_a = git_of(&view, &a.dir, cx);
    let one = layout(&view, tab, cx);
    assert_eq!(one, split(Axis::Columns, leaf(tab), leaf(git_a), ratio(&one)), "left|right, the Git panel on the right");
    assert_eq!(view.read_with(cx, |v, _| (v.header_ids(), v.active, v.docks.focused(tab))), (vec![tab], Some(tab), git_a));

    view.update(cx, |v, _| v.tab_mut(tab).expect("the tab").cwd = Some(b.dir.clone()));
    choose_add_git(&view, tab, cx);
    let git_b = git_of(&view, &b.dir, cx);
    let two = layout(&view, tab, cx);
    let Layout::Split { second, .. } = &two else { panic!("a split") };
    assert_eq!(two, split(Axis::Columns, leaf(tab), Box::new(split(Axis::Rows, leaf(git_a), leaf(git_b), ratio(second))), ratio(&two)), "the right half split, the new panel below");
    assert!(!view.read_with(cx, |v, cx| v.dock_facts(tab, cx).room), "three panels: no room for a fourth");
}

#[gpui::test]
fn tab_menu_add_git_moves_the_panel_from_another_tab_which_keeps_its_others(cx: &mut gpui::TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = window(cx);
    let (other, _) = shell(&view, cx);
    let (git, _) = git_panel(&view, &f, cx);
    let (third, _) = shell(&view, cx);
    view.update(cx, |v, cx| {
        assert!(v.add_panel(other, git, cx) && v.add_panel(other, third, cx));
    });
    let tab = claude_tab(&view, &f.dir, cx);
    choose_add_git(&view, tab, cx);
    let after = view.read_with(cx, |v, cx| (v.docks.panels(tab), v.docks.panels(other), v.header_ids(), v.active, v.find_panel(&PanelKey::Git(path_key(&f.dir)), cx)));
    assert_eq!(after, (vec![tab, git], vec![other, third], vec![other, tab], Some(tab), Some(git)), "moved, one Git panel, the other tab keeps its other panels");

    let elsewhere = Fixture::copy();
    let (alone, _) = git_panel(&view, &elsewhere, cx);
    let moved = claude_tab(&view, &f.dir, cx);
    view.update(cx, |v, cx| v.add_to_tab(moved, Added::Git(f.dir.clone()), cx));
    assert_eq!(view.read_with(cx, |v, _| (v.docks.panels(moved), v.docks.panels(tab))), (vec![moved, git], vec![tab]), "from a dock of two: that tab is a tab of one again");
    assert!(view.read_with(cx, |v, _| v.header_ids().contains(&alone)), "a Git tab of another repo is left alone");
}

#[gpui::test]
fn tab_menu_git_panels_of_two_repos_in_one_dock_paint_under_their_own_scopes(cx: &mut gpui::TestAppContext) {
    let (a, b) = (Fixture::shared(), Fixture::copy());
    let (view, cx) = window(cx);
    let (one, git_a) = git_panel(&view, &a, cx);
    let (two, git_b) = git_panel(&view, &b, cx);
    loaded(&view, &git_a, cx);
    assert!(keys(&view, cx).contains(&"git-toolbar".to_string()), "a Git tab of one paints under the window's own keys");
    view.update(cx, |v, cx| v.add_to_tab(one, Added::Git(b.dir.clone()), cx));
    assert_eq!(view.read_with(cx, |v, _| v.docks.panels(one)), vec![one, two]);
    loaded(&view, &git_b, cx);
    draw(&view, cx);
    let painted = keys(&view, cx);
    for key in [format!("p{one}/git-toolbar"), format!("p{two}/git-toolbar")] {
        assert!(painted.contains(&key), "{key} paints: {painted:?}");
    }
    assert!(!painted.iter().any(|k| k.starts_with("git-")), "no Git key outside a panel's scope: {painted:?}");

    view.update(cx, |v, cx| v.close_panel(two, cx));
    draw(&view, cx);
    let painted = keys(&view, cx);
    assert!(painted.contains(&"git-toolbar".to_string()) && !painted.iter().any(|k| k.starts_with(&format!("p{one}/"))), "back to the empty scope as a tab of one: {painted:?}");
}

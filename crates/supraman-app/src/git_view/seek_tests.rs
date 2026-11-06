use gpui::{Entity, TestAppContext, VisualTestContext, px};
use supraman_gitman::fixture::{Generated, git_in};

use super::details::PanelEvent;
use super::tests::open;
use super::{GitView, Selected, graph};

fn rows(view: &Entity<GitView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.rows.len())
}

fn toast(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.toast.text().map(str::to_string))
}

fn tag(g: &Generated, name: &str) -> String {
    supraman_gitman::refs(&g.repo()).unwrap().tags.into_iter().find(|t| t.name == name).and_then(|t| t.commit).unwrap()
}

#[gpui::test]
fn picking_a_ref_out_of_the_walk_loads_no_page_and_says_it_is_not_in_the_graph(cx: &mut TestAppContext) {
    let g = Generated::new(4000);
    let t1 = tag(&g, "t1");
    let who = ["GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL", "GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL"].map(|k| (k, "lone@example.test".to_string()));
    let lone = git_in(&g.git, &g.dir, &["commit-tree", "-p", &t1, "-m", "Lone", &format!("{t1}^{{tree}}")], &who, None);
    git_in(&g.git, &g.dir, &["branch", "lone", &lone], &[], None);
    let (view, cx) = open(&g.dir, cx);
    let panel = view.read_with(cx, |v, _| v.ref_panel().clone());
    panel.update(cx, |p, cx| p.toggle_hidden(&["refs/heads/lone".to_string()], cx));
    cx.run_until_parked();
    let loaded = rows(&view, cx);
    assert!(view.read_with(cx, |v, _| v.more), "more pages are left");
    panel.update(cx, |p, cx| p.picked("local/lone", cx));
    cx.run_until_parked();
    assert_eq!(rows(&view, cx), loaded, "no page loads for a ref the graph does not walk");
    assert_eq!(toast(&view, cx), Some(format!("{} is not in the graph.", &lone[..7])));
}

#[gpui::test]
fn a_seek_stops_after_its_pages_and_says_so(cx: &mut TestAppContext) {
    let g = Generated::new(4000);
    let t1 = tag(&g, "t1");
    let (view, cx) = open(&g.dir, cx);
    let loaded = rows(&view, cx);
    view.update(cx, |v, cx| v.seek_within(t1.clone().into(), 1, cx));
    cx.run_until_parked();
    assert_eq!(rows(&view, cx), loaded + supraman_gitman::NEXT_PAGE, "one page, then the seek stops");
    assert_ne!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(t1.clone().into())));
    assert!(toast(&view, cx).is_some_and(|t| t.starts_with(&format!("{} is more than", &t1[..7]))), "{:?}", toast(&view, cx));
    let lanes = view.read_with(cx, |v, _| (v.graph_max, px(graph::width_for(graph::lanes(&v.rows)))));
    assert_eq!(lanes.0, Some(lanes.1), "the lanes measured page by page match a measure of every row");
}

#[gpui::test]
fn a_parent_click_whose_page_is_not_loaded_reveals_it(cx: &mut TestAppContext) {
    let g = Generated::new(4000);
    let (view, cx) = open(&g.dir, cx);
    let (last, parent) = view.read_with(cx, |v, _| {
        let last = v.rows.last().unwrap();
        (last.sha.clone(), last.parents[0].clone())
    });
    assert!(view.read_with(cx, |v, _| !v.rows.iter().any(|r| r.sha == parent)), "the parent lies past the first page");
    view.update(cx, |v, cx| v.select_sha(&last, cx));
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.panel().clone());
    panel.update(cx, |_, cx| cx.emit(PanelEvent::Parent(parent.clone())));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Commit(parent)));
}

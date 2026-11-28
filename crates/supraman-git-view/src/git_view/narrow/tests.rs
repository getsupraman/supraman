use gpui::{Bounds, Pixels, TestAppContext, px};
use supraman_gitman::fixture::Fixture;
use supraman_ui::hit::Hit;

use super::*;
use crate::git_view::test_support::{self, Opts, resize, settle};
use crate::git_view::{GRAPH_PANE, PANEL_PANE, REFS_PANE, TABLE};

const TOOLBAR_KEYS: [&str; 7] = ["git-undo", "git-redo", "git-stash", "git-pop", "git-pull", "git-pull-menu", "git-push"];

/// Every toolbar key, and with `graph` every graph key, painted now: within the tab's width and wholly inside its clip (one the bottom cuts, across).
fn assert_all_shown(hits: &supraman_ui::hit::HitTargets, width: f32, graph: bool) {
    let hits = hits.borrow();
    let tab = hits.get("git-toolbar").expect("the toolbar").bounds;
    assert!((tab.size.width - px(width)).abs() < px(1.), "the tab {tab:?} is {width} wide");
    let inside = |key: &str, hit: &Hit| {
        assert!(hit.fully_visible() && hit.bounds.left() >= tab.left() - px(1.) && hit.bounds.right() <= tab.right() + px(1.), "{key} shown whole at {width}: {hit:?} in {tab:?}");
    };
    for key in TOOLBAR_KEYS {
        inside(key, hits.get(key).unwrap_or_else(|| panic!("{key} is painted at {width}")));
    }
    if !graph {
        return;
    }
    let graph: Vec<_> = hits.keys().filter(|k| k.starts_with(&format!("{TABLE}-"))).cloned().collect();
    assert!(graph.iter().any(|k| k.starts_with("git-row-")) && graph.iter().any(|k| k.starts_with("git-col-")), "the graph's rows and header: {graph:?}");
    for key in graph {
        let hit = hits[&key];
        if hit.bounds.bottom() <= hit.visible.bottom() {
            inside(&key, &hit);
        } else {
            assert!(hit.bounds.left() >= hit.visible.left() - px(1.) && hit.bounds.right() <= hit.visible.right().min(tab.right()) + px(1.), "{key} across at {width}: {hit:?}");
        }
    }
}

fn at(hits: &supraman_ui::hit::HitTargets, key: &str) -> Bounds<Pixels> {
    hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds
}

#[gpui::test]
fn narrow_at_640_the_toolbar_and_graph_show_whole_with_the_panel_under_the_graph(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = test_support::open(&f.dir, Opts::default().size(640., 700.), cx);
    settle(&view, cx);
    let hits = test_support::repaint(&view, cx);
    assert_all_shown(&hits, 640., true);
    let (graph, panel) = (at(&hits, GRAPH_PANE), at(&hits, PANEL_PANE));
    assert!(panel.top() >= graph.bottom() && panel.size.width >= graph.size.width - px(1.), "the commit panel {panel:?} under the graph {graph:?}");
    assert!(!hits.borrow().contains_key(REFS_PANE) && hits.borrow().contains_key("refpanel-expand"), "the ref panel shows its strip");
    assert!(!view.read_with(cx, |v, cx| v.ref_panel.read(cx).shut()), "the user's open ref panel is still theirs");
}

#[gpui::test]
fn narrow_at_the_least_size_the_toolbar_shows_whole_over_the_graph_and_the_panel(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (w, h) = (f32::from(min_size().width), f32::from(min_size().height));
    let (view, cx) = test_support::open(&f.dir, Opts::default().size(w, h), cx);
    settle(&view, cx);
    let hits = test_support::repaint(&view, cx);
    assert_all_shown(&hits, w, false);
    let (graph, panel) = (at(&hits, GRAPH_PANE), at(&hits, PANEL_PANE));
    assert!(panel.top() >= graph.bottom() && graph.size.height >= STACKED_MIN && panel.bottom() <= px(h + 1.), "both halves in the tab: {graph:?} {panel:?}");
}

#[gpui::test]
fn narrow_then_wide_again_gives_back_the_kept_layout(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = test_support::open(&f.dir, Opts::default().size(1400., 800.), cx);
    settle(&view, cx);
    let wide = test_support::repaint(&view, cx);
    let (graph, panel, refs) = (at(&wide, GRAPH_PANE), at(&wide, PANEL_PANE), at(&wide, REFS_PANE));
    let kept = view.read_with(cx, |v, cx| v.sizes(cx));
    for width in [700., 500., 360.] {
        resize(width, 800., cx);
        settle(&view, cx);
        assert_eq!(view.read_with(cx, |v, cx| v.sizes(cx)), kept, "the kept layout at {width}");
    }
    resize(1400., 800., cx);
    settle(&view, cx);
    let back = test_support::repaint(&view, cx);
    assert_eq!((at(&back, GRAPH_PANE), at(&back, PANEL_PANE), at(&back, REFS_PANE)), (graph, panel, refs), "the same layout as before");
    assert_eq!(view.read_with(cx, |v, cx| v.sizes(cx)), kept);
}

#[gpui::test]
fn narrow_the_ref_panel_shows_its_strip_first_and_the_panel_stays_beside_the_graph(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = test_support::open(&f.dir, Opts::default().size(740., 700.), cx);
    settle(&view, cx);
    let hits = test_support::repaint(&view, cx);
    let (graph, panel) = (at(&hits, GRAPH_PANE), at(&hits, PANEL_PANE));
    assert!(!hits.borrow().contains_key(REFS_PANE), "the strip");
    assert!(panel.left() >= graph.right() && graph.size.width >= px(crate::git_view::GRAPH_MIN), "beside: {graph:?} {panel:?}");
}

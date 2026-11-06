use gpui::{Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, Generated, git_in};
use supraman_gitman::{FIRST_PAGE, NEXT_PAGE};
use supraman_ui::hit::HitTargets;
use supraman_ui::themes::{NAME_FLOOR, SECONDARY_FLOOR};

use super::*;
use super::load::initials;
use crate::saved_tabs::{self, Reopen, SavedTab, SavedTabs, World};
use crate::view::git_tab::open_or_focus;
use crate::view::{Tab, TabKind};

pub(crate) fn open<'a>(folder: &std::path::Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    open_in(folder, None, cx)
}

/// A Git tab of `folder` whose layout file lives in `home`.
fn open_in<'a>(folder: &std::path::Path, home: Option<&std::path::Path>, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        if !cx.has_global::<supraman_term::terminal_settings::TerminalConfig>() {
            supraman_ui::init_globals(cx, "test");
            bind_keys(cx);
        }
    });
    let (folder, home) = (folder.to_path_buf(), home.map(|h| h.to_path_buf()));
    let (view, cx) = cx.add_window_view(|_, cx| {
        let view = GitView::open(folder, home, HitTargets::default(), &GitSizes::default(), cx);
        Holder(view)
    });
    let view = view.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    redraw(&view, cx);
    (view, cx)
}

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1200.)).h(px(800.)).child(self.0.clone())
    }
}

pub(crate) fn redraw(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|_, _| ());
}

fn row_of<'a>(view: &'a GitView, sha: &str) -> (usize, &'a GraphRow) {
    view.rows.iter().enumerate().find(|(_, r)| r.sha.as_ref() == sha).unwrap_or_else(|| panic!("no row of {sha}"))
}

fn names(groups: &[supraman_gitman::RefGroup]) -> Vec<&str> {
    groups.iter().map(|g| g.name.as_str()).collect()
}

fn sizes() -> GitSizes {
    GitSizes { widths: vec![200, 90, 300, 130, 70], panel: Some(360), message: Some(150), refs: Some(210), refs_shut: true, hidden: vec!["refs/heads/feature".into(), "stash/0123".into()], collapsed: Vec::new(), solo: Vec::new(), sections: Vec::new() }
}

#[gpui::test]
fn git_tab_opens_once_per_folder_and_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (mut tabs, mut next) = (Vec::<Tab>::new(), 1u64);
    let made = std::cell::Cell::new(0);
    for _ in 0..2 {
        let (id, _) = cx.update(|cx| {
            open_or_focus(&mut tabs, &mut next, &f.dir, "Git · repo".into(), cx, |cx| {
                made.set(made.get() + 1);
                GitView::open(f.dir.clone(), None, HitTargets::default(), &sizes(), cx)
            })
        });
        assert_eq!(id, 1, "a second open focuses the first tab");
    }
    assert_eq!((tabs.len(), made.get()), (1, 1), "one tab and one view for the folder");
    let other = f.temp.path().join("wt");
    let (id, new) = cx.update(|cx| open_or_focus(&mut tabs, &mut next, &other, "Git · wt".into(), cx, |cx| GitView::open(other.clone(), None, HitTargets::default(), &GitSizes::default(), cx)));
    assert_eq!((id, new, tabs.len()), (2, true, 2), "another folder, another tab");

    let TabKind::Git(view) = &tabs[0].kind else { panic!("a Git tab") };
    let saved = cx.update(|cx| SavedTab::git(tabs[0].title.clone(), view.read(cx).folder.clone()));
    let home = tempfile::tempdir().unwrap();
    saved_tabs::save(home.path(), &SavedTabs { tabs: vec![saved.clone()], active: Some(0) }).unwrap();
    let (back, notice) = saved_tabs::load(home.path());
    assert_eq!((back.tabs.as_slice(), notice), (std::slice::from_ref(&saved), None));
    let world = World { sessions: &[], now: 0, workspaces: &[], resume_on_click: false, is_dir: &|p| p.is_dir(), is_file: &|_| true, has_transcript: &|_, _| false };
    let Ok(Reopen::Git { title, folder, sizes: back }) = saved_tabs::plan(&back.tabs[0], &world) else { panic!("a Git tab reopens") };
    assert_eq!((title.as_str(), folder.as_path(), back), ("Git · repo", f.dir.as_path(), GitSizes::default()), "the sizes are the layout file's, not the tab's");
    let gone = SavedTab::git("Git · x".into(), f.temp.path().join("gone"));
    assert!(saved_tabs::plan(&gone, &world).unwrap_err().contains("is gone"));

    let old = serde_json::json!({ "kind": "git", "title": "Git · repo", "folder": f.dir, "widths": [200, 90, 300, 130, 70], "panel": 360, "message": 150, "refs": 210, "refs_shut": true, "hidden": ["refs/heads/feature", "stash/0123"] });
    let old: SavedTab = serde_json::from_value(old).unwrap();
    let Ok(Reopen::Git { sizes: back, .. }) = saved_tabs::plan(&old, &world) else { panic!("a Git tab reopens") };
    assert_eq!(back, sizes(), "a tab saved before the layout file still hands over its sizes");
}

#[gpui::test]
fn git_tab_holds_saved_widths_at_the_columns_mins(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let narrow = GitSizes { widths: vec![1, 1, 1, 1, 1], ..sizes() };
    let view = cx.update(|cx| GitView::open(f.dir.clone(), None, HitTargets::default(), &narrow, cx));
    cx.run_until_parked();
    let clamped = cx.update(|cx| view.read(cx).sizes(cx).widths);
    assert_eq!(clamped, vec![32, graph::MIN_WIDTH as u32, 50, 50, 50], "saved widths are held at the columns' mins on load");
}

#[gpui::test]
fn git_tab_reopens_at_its_repos_layout_and_a_new_repo_at_the_last_one(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let home = tempfile::tempdir().unwrap();
    let (view, vcx) = open_in(&f.dir, Some(home.path()), cx);
    let fitted = view.read_with(vcx, |v, _| v.widths[1]);
    assert!(fitted > px(graph::MIN_WIDTH), "the first open fits the graph to its lanes: {fitted:?}");
    let tags = view.read_with(vcx, |v, _| v.hits.borrow()["refpanel-row-tags"].bounds.center());
    vcx.simulate_click(tags, Modifiers::none());
    view.update(vcx, |v, cx| {
        v.ref_panel.update(cx, |p, cx| p.toggle_hidden(&["refs/heads/feature".to_string()], cx));
        v.panel.update(cx, |p, cx| p.set_message_height(Some(px(140.)), cx));
        v.panel_width = px(420.);
        v.widths[1] = px(graph::MIN_WIDTH);
        v.widths[2] = px(260.);
        v.keep_layout(cx);
    });
    vcx.run_until_parked();
    let kept = vcx.update(|_, cx| view.read(cx).sizes(cx));
    assert!(kept.collapsed == ["tags"] && kept.hidden == ["refs/heads/feature"], "{kept:?}");
    drop(view);

    let (again, vcx) = open_in(&f.dir, Some(home.path()), cx);
    let back = vcx.update(|_, cx| again.read(cx).sizes(cx));
    assert_eq!(back, kept, "the columns, the panels, the hidden refs and the collapsed folders come back");
    assert_eq!(back.widths[1], graph::MIN_WIDTH as u32, "a narrowed graph column stays narrow");
    let rows = again.read_with(vcx, |v, cx| v.ref_panel.read(cx).visible().iter().map(|p| p.to_string()).collect::<Vec<_>>());
    assert!(rows.contains(&"tags".to_string()) && !rows.iter().any(|r| r.starts_with("tags/")), "{rows:?}");

    let (file, notice) = layout::load(Some(home.path()));
    assert!(notice.is_none() && file.get().repos.get(&layout::key(&f.dir)) == Some(&kept), "{:?}", file.get());

    let other = Generated::shared(30);
    let (fresh, vcx) = open_in(&other.dir, Some(home.path()), cx);
    let new = vcx.update(|_, cx| fresh.read(cx).sizes(cx));
    assert_eq!((new.panel, new.message, new.widths[2]), (kept.panel, kept.message, kept.widths[2]), "a new repo takes the last widths");
    assert!(new.hidden.is_empty() && new.collapsed.is_empty(), "not the last repo's refs: {new:?}");
}

#[gpui::test]
fn git_tab_shows_refs_messages_dates_and_shas_of_the_fixture(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    view.read_with(cx, |v, _| {
        assert!(matches!(v.load, Load::Ready), "the fixture loads");
        let shas: Vec<&str> = v.rows.iter().skip(1).map(|r| r.sha.as_ref()).collect();
        for label in ["ahead", "merge", "main_work", "rename", "second", "root"] {
            assert!(shas.contains(&f.sha(label)), "{label} is a row");
        }
        let (_, ahead) = row_of(v, f.sha("ahead"));
        let cell = ref_cell(&ahead.row, false);
        let first = cell.first.expect("main on its commit");
        assert!(first.name == "main" && first.head() && !cell.ghost, "{first:?}");
        assert_eq!(ahead.summary.as_ref(), "Ahead of origin");
        assert_eq!(cells::sha_text(ahead), &f.sha("ahead")[..7]);
        assert_eq!(ahead.time_ms, 1_700_000_000_000 + 6 * 60_000, "the sixth commit's time");
        assert_eq!(cells::date_text(ahead), supraman_core::time::stamp(ahead.time_ms));

        let (_, rename) = row_of(v, f.sha("rename"));
        assert_eq!(rename.summary.as_ref(), "Move old.txt to new.txt");
        assert!(rename.body.starts_with("The old name no longer fits. Co-authored-by: Bea Two"), "{}", rename.body);
        assert_eq!(names(&rename.row.refs), ["feature"]);
        let (_, merge) = row_of(v, f.sha("merge"));
        assert!(merge.merge && merge.row.refs.iter().any(|g| g.refs.iter().any(|r| r.name == "origin/main")));
        let (_, root) = row_of(v, f.sha("root"));
        let ghost = ref_cell(&root.row, false);
        assert!(ghost.ghost && ghost.first.is_some() && root.row.refs.is_empty(), "a ref-less row offers its descendant's branch: {ghost:?}");
    });
}

#[gpui::test]
fn git_tab_shows_each_stash_as_a_row_of_its_label_at_its_time(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    view.read_with(cx, |v, _| {
        let label = |index: &str| {
            let (_, row) = row_of(v, f.sha(index));
            let stash = row.stash().unwrap_or_else(|| panic!("{index} is a stash's row"));
            assert_eq!((row.time_ms, row.body.as_ref()), (stash.time * 1000, ""));
            row.summary.to_string()
        };
        assert_eq!(label("stash_message_index"), supraman_gitman::fixture::MAIN_STASH);
        assert_eq!(label("stash_feature_index"), supraman_gitman::fixture::FEATURE_STASH);
        assert_eq!(label("stash_wip_index"), format!("{} Merge feature", &f.sha("merge")[..7]));
        assert!(v.rows.iter().all(|r| r.sha.as_ref() != f.sha("stash_message")), "the stash commit itself has no row");
        assert_eq!(v.rows.iter().filter(|r| r.stash().is_some()).count(), 3);
    });
}

#[gpui::test]
fn git_tab_stacks_a_rows_refs_and_lists_them_on_hover(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    let (ix, cell) = view.read_with(cx, |v, _| {
        let (ix, second) = row_of(v, f.sha("second"));
        (ix, ref_cell(&second.row, false))
    });
    assert_eq!((cell.first.as_ref().map(|g| g.name.as_str()), cell.more), (Some("side"), 1), "the worktree's branch, then +1 for v1");
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    assert!(hits.borrow().contains_key(&format!("git-more-{ix}")) && !hits.borrow().contains_key(&format!("git-reflist-{ix}-1")));
    let at = hits.borrow()[&format!("git-refs-{ix}")].bounds.center();
    cx.simulate_mouse_move(at, None, Modifiers::none());
    redraw(&view, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.listed(), Some(ix), "resting on the cell opens its list");
        let open = ref_cell(&v.rows[ix].row, true);
        assert_eq!((names(&open.list), open.more), (vec!["side", "v1"], 0), "every group listed and +N hidden");
    });
    let (pill, hovered, below) = {
        let hits = hits.borrow();
        let at = |i: usize| hits.get(&format!("git-reflist-{ix}-{i}")).map(|h| h.bounds).expect("the list paints a pill per group");
        (hits[&format!("git-pill-{ix}")].bounds, at(0), at(1))
    };
    assert!((hovered.left() - pill.left()).abs() < px(0.5) && (hovered.top() - pill.top()).abs() < px(0.5), "the layer lies on the row's pill: {pill:?} {hovered:?}");
    assert!((below.top() - hovered.bottom()).abs() < px(0.5) && (below.left() - hovered.left()).abs() < px(0.5), "v1 hangs from the hovered pill: {hovered:?} {below:?}");
    assert!((below.size.width - hovered.size.width).abs() < px(0.5) && hovered.size.width >= pill.size.width - px(0.5), "one width, the pill's or more: {pill:?} {hovered:?} {below:?}");
    let (_, ahead) = view.read_with(cx, |v, _| { let (i, r) = row_of(v, f.sha("ahead")); (i, r.row.clone()) });
    assert!(ref_cell(&ahead, true).list.is_empty(), "a single group opens no list");
}

#[gpui::test]
fn git_tab_puts_the_wip_row_on_top_of_a_dirty_tree_and_none_on_a_clean_one(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let (view, cx) = open(&f.dir, cx);
    view.read_with(cx, |v, _| {
        let top = &v.rows[0];
        assert!(top.wip() && v.rows[1].sha.as_ref() == f.sha("ahead"), "WIP, then HEAD's commit");
        assert!(top.row.edges().all(|e| e.dashed), "its edge to HEAD is dashed");
        assert_eq!(v.wip.map(|w| w.counts()).as_deref(), Some("+2 ✎2 −1"), "s.txt and u.txt added, a.txt and the rename changed, b.txt deleted");
        assert_eq!(v.rows.iter().filter(|r| r.wip()).count(), 1);
    });
    f.git(&["reset", "-q", "--hard"]);
    f.git(&["clean", "-qfd"]);
    view.update(cx, |v, cx| v.reload(cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.rows.iter().all(|r| !r.wip()) && v.wip.is_none(), "a clean tree has no WIP row");
        assert_eq!(v.rows[0].sha.as_ref(), f.sha("ahead"));
    });
}

#[gpui::test]
fn git_tab_paints_a_long_history_without_a_stall_and_pages_at_the_end(cx: &mut TestAppContext) {
    let g = Generated::shared(20_000);
    let loading = cx.update(|cx| {
        let view = GitView::open(g.dir.clone(), None, HitTargets::default(), &GitSizes::default(), cx);
        matches!(view.read(cx).load, Load::Loading) && view.read(cx).rows.is_empty()
    });
    assert!(loading, "the first page is read off the UI thread");
    let (view, cx) = open(&g.dir, cx);
    let commits = |v: &GitView| v.rows.iter().filter(|r| !r.wip()).count();
    let first = view.read_with(cx, |v, _| (commits(v), v.paging));
    assert_eq!(first, (FIRST_PAGE, false));
    let last = view.read_with(cx, |v, _| v.rows.len() - 1);
    view.read_with(cx, |v, _| v.table.reveal(FIRST_PAGE / 2));
    redraw(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| commits(v)), FIRST_PAGE, "half way down loads nothing");
    view.read_with(cx, |v, _| v.table.reveal(last));
    redraw(&view, cx);
    redraw(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| commits(v)), FIRST_PAGE + NEXT_PAGE, "the end loads the next page");
    let shas: std::collections::HashSet<_> = view.read_with(cx, |v, _| v.rows.iter().filter(|r| !r.wip()).map(|r| r.sha.clone()).collect());
    assert_eq!(shas.len(), FIRST_PAGE + NEXT_PAGE, "no commit twice");
    let paging = view.update(cx, |v, cx| {
        v.load_more(cx);
        (v.paging, commits(v))
    });
    assert_eq!(paging, (true, FIRST_PAGE + NEXT_PAGE), "the next page is read off the UI thread");
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| commits(v)), FIRST_PAGE + 2 * NEXT_PAGE);
}

#[gpui::test]
fn git_tab_gives_two_authors_their_initials_and_distinct_lane_rings(cx: &mut TestAppContext) {
    let f = Fixture::new();
    let second = f.sha("second").to_string();
    let date = "@1700010000 +0000".to_string();
    let env = [
        ("GIT_AUTHOR_NAME", "Bea Two".to_string()),
        ("GIT_AUTHOR_EMAIL", "bea@example.test".to_string()),
        ("GIT_AUTHOR_DATE", date.clone()),
        ("GIT_COMMITTER_DATE", date),
    ];
    let tree = format!("{second}^{{tree}}");
    let bea = git_in(&f.git, &f.dir, &["commit-tree", &tree, "-p", &second, "-m", "By Bea"], &env, None);
    f.git(&["branch", "bea", &bea]);
    let (view, cx) = open(&f.dir, cx);
    let palette = cx.update(|_, cx| Palette::of(cx.theme()));
    view.read_with(cx, |v, _| {
        let (_, b) = row_of(v, &bea);
        let (_, a) = row_of(v, f.sha("ahead"));
        assert_eq!((b.initials.as_ref(), a.initials.as_ref()), ("BT", "AF"));
        assert_ne!(a.row.lane, b.row.lane);
        assert_ne!(palette.lane(a.row.colour), palette.lane(b.row.colour), "their rings differ");
    });
    assert_eq!(initials("Jan van Vliet"), "JV");
    assert_eq!(initials("ada"), "A");
    assert_eq!(initials(""), "?");
}

fn on(p: &Palette, fg: gpui::Hsla, layer: gpui::Hsla) -> f32 {
    supraman_ui::themes::contrast(fg, super::colours::blend(layer, p.background))
}

fn light() -> theme::Theme {
    let style = serde_json::json!({
        "background": "#F3F3F3",
        "panel.background": "#F3F3F3",
        "editor.background": "#FFFFFF",
        "text": "#1F1F1F",
        "text.muted": "#5A5A5A",
    });
    let zed = supraman_ui::themes::ZedTheme { name: "Light test".into(), appearance: Some("light".into()), style: style.as_object().unwrap().clone() };
    let mut theme = supraman_ui::themes::from_zed(&supraman_ui::themes::one_dark(), &zed).theme;
    theme.styles.accents = theme::AccentColors::light();
    theme
}

#[test]
fn git_tab_band_runs_from_the_node_to_the_column_edge_short_of_the_row_edges() {
    let column = gpui::Bounds::new(gpui::point(px(100.), px(56.)), gpui::size(px(120.), px(graph::ROW)));
    for lane in 0..3 {
        let band = graph::band(lane, column);
        assert_eq!(band.left(), column.left() + px(graph::lane_x(lane)), "lane {lane} starts at its node's centre");
        assert_eq!(band.right(), column.right());
        assert_eq!((band.top(), band.bottom()), (column.top() + px(graph::GAP / 2.), column.bottom() - px(graph::GAP / 2.)));
    }
    let narrow = graph::band(9, column);
    assert!(narrow.size.width == px(0.) && narrow.left() == column.right(), "a lane past the edge paints nothing: {narrow:?}");
    let first = graph::lane_x(0);
    assert!(first - 10. >= 8. && graph::width_for(1) - first - 10. >= 8., "the first lane's node keeps 8 px either side");
}

#[test]
fn git_tab_colours_come_from_the_theme_and_keep_text_readable() {
    for theme in [supraman_ui::themes::dark_modern().theme, light()] {
        let p = Palette::of(&theme);
        let lanes: std::collections::HashSet<String> = (0..supraman_gitman::LANE_COLOURS).map(|i| supraman_ui::themes::hex(p.lane(i))).collect();
        assert_eq!(lanes.len(), supraman_gitman::LANE_COLOURS, "{}: {lanes:?}", theme.name);
        assert_eq!(p.lane(0), supraman_ui::themes::lanes::lanes(&theme)[0], "lanes are the theme's");
        assert!(supraman_ui::themes::contrast(p.muted, p.background) >= SECONDARY_FLOOR, "{} date and sha on the background", theme.name);
        assert!(supraman_ui::themes::contrast(p.muted, p.background) < supraman_ui::themes::contrast(p.text, p.background), "{}: date and sha are quieter than the message", theme.name);
        for i in 0..supraman_gitman::LANE_COLOURS {
            assert!(on(&p, p.text, p.band(i)) >= NAME_FLOOR, "{} text on band {i}", theme.name);
            assert!(on(&p, p.text, p.pill(i)) >= SECONDARY_FLOOR, "{} text on pill {i}", theme.name);
            assert!(supraman_ui::themes::contrast(p.initials(i), p.avatar(i)) >= SECONDARY_FLOOR, "{} initials on avatar {i}", theme.name);
        }
    }
}

#[gpui::test]
fn git_tab_reports_a_click_and_up_down_as_the_selection(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, cx);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _sub = cx.update({
        let (view, seen) = (view.clone(), seen.clone());
        move |_, cx| cx.subscribe(&view, move |v, GitEvent::Selected, cx| seen.borrow_mut().push(v.read(cx).selected()))
    });
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let row = hits.borrow()["git-row-1"].bounds.center();
    cx.simulate_click(row, Modifiers::none());
    cx.simulate_keystrokes("up down");
    let ahead = Selected::Commit(f.sha("ahead").to_string().into());
    assert_eq!(*seen.borrow(), vec![Some(ahead.clone()), Some(Selected::Wip), Some(ahead)]);
}

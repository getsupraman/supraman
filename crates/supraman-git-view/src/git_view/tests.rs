mod paint;
mod stream;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, Generated, git_in};
use supraman_gitman::{FIRST_PAGE, NEXT_PAGE};
use supraman_ui::hit::HitTargets;
use supraman_ui::themes::{NAME_FLOOR, SECONDARY_FLOOR};

use super::*;
use super::load::{GraphRow, initials};
use super::test_support::{Opts, open, open_unchecked, redraw, sizes};

fn graph_row<'a>(view: &GitView, sha: &str, cx: &'a App) -> (usize, &'a GraphRow) {
    view.pages.read(cx).rows.iter().enumerate().find(|(_, r)| r.sha.as_ref() == sha).unwrap_or_else(|| panic!("no row of {sha}"))
}

fn names(groups: &[supraman_gitman::RefGroup]) -> Vec<&str> {
    groups.iter().map(|g| g.name.as_str()).collect()
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
    let (view, vcx) = open(&f.dir, Opts::default().home(home.path()), cx);
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

    let (again, vcx) = open(&f.dir, Opts::default().home(home.path()), cx);
    let back = vcx.update(|_, cx| again.read(cx).sizes(cx));
    assert_eq!(back, kept, "the columns, the panels, the hidden refs and the collapsed folders come back");
    assert_eq!(back.widths[1], graph::MIN_WIDTH as u32, "a narrowed graph column stays narrow");
    let rows = again.read_with(vcx, |v, cx| v.ref_panel.read(cx).visible().iter().map(|p| p.to_string()).collect::<Vec<_>>());
    assert!(rows.contains(&"tags".to_string()) && !rows.iter().any(|r| r.starts_with("tags/")), "{rows:?}");

    let (file, notice) = layout::load(Some(home.path()));
    assert!(notice.is_none() && file.get().repos.get(&layout::key(&f.dir)) == Some(&kept), "{:?}", file.get());

    let other = Generated::shared(30);
    let (fresh, vcx) = open(&other.dir, Opts::default().home(home.path()), cx);
    let new = vcx.update(|_, cx| fresh.read(cx).sizes(cx));
    assert_eq!((new.panel, new.message, new.widths[2]), (kept.panel, kept.message, kept.widths[2]), "a new repo takes the last widths");
    assert!(new.hidden.is_empty() && new.collapsed.is_empty(), "not the last repo's refs: {new:?}");
}

#[gpui::test]
fn git_tab_shows_refs_messages_dates_and_shas_of_the_fixture(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    view.read_with(cx, |v, cx| {
        assert!(matches!(v.load, Load::Ready), "the fixture loads");
        let shas: Vec<&str> = v.pages.read(cx).rows.iter().skip(1).map(|r| r.sha.as_ref()).collect();
        for label in ["ahead", "merge", "main_work", "rename", "second", "root"] {
            assert!(shas.contains(&f.sha(label)), "{label} is a row");
        }
        let (_, ahead) = graph_row(v, f.sha("ahead"), cx);
        let cell = ref_cell(&ahead.row, false);
        let first = cell.first.expect("main on its commit");
        assert!(first.name == "main" && first.head() && !cell.ghost, "{first:?}");
        assert_eq!(ahead.summary.as_ref(), "Ahead of origin");
        assert_eq!(cells::sha_text(ahead), &f.sha("ahead")[..7]);
        assert_eq!(ahead.time_ms, 1_700_000_000_000 + 6 * 60_000, "the sixth commit's time");
        assert_eq!(cells::date_text(ahead), supraman_core::time::stamp(ahead.time_ms));

        let (_, rename) = graph_row(v, f.sha("rename"), cx);
        assert_eq!(rename.summary.as_ref(), "Move old.txt to new.txt");
        assert!(rename.body.starts_with("The old name no longer fits. Co-authored-by: Bea Two"), "{}", rename.body);
        assert_eq!(names(&rename.row.refs), ["feature"]);
        let (_, merge) = graph_row(v, f.sha("merge"), cx);
        assert!(merge.merge && merge.row.refs.iter().any(|g| g.refs.iter().any(|r| r.name == "origin/main")));
        let (_, root) = graph_row(v, f.sha("root"), cx);
        let ghost = ref_cell(&root.row, false);
        assert!(ghost.ghost && ghost.first.is_some() && root.row.refs.is_empty(), "a ref-less row offers its descendant's branch: {ghost:?}");
    });
    let ix = view.read_with(cx, |v, cx| graph_row(v, f.sha("root"), cx).0);
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let (pill, cell) = (hits.borrow()[&format!("git-ghost-{ix}")].bounds, hits.borrow()[&format!("git-refs-{ix}")].bounds);
    assert!(pill.left() >= cell.left() && pill.right() <= cell.right() + px(0.5), "the ghost pill paints in its row's ref cell: {pill:?} {cell:?}");
}

#[gpui::test]
fn git_tab_shows_each_stash_as_a_row_of_its_label_at_its_time(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    view.read_with(cx, |v, cx| {
        let label = |index: &str| {
            let (_, row) = graph_row(v, f.sha(index), cx);
            let stash = row.stash().unwrap_or_else(|| panic!("{index} is a stash's row"));
            assert_eq!((row.time_ms, row.body.as_ref()), (stash.time * 1000, ""));
            row.summary.to_string()
        };
        assert_eq!(label("stash_message_index"), supraman_gitman::fixture::MAIN_STASH);
        assert_eq!(label("stash_feature_index"), supraman_gitman::fixture::FEATURE_STASH);
        assert_eq!(label("stash_wip_index"), format!("{} Merge feature", &f.sha("merge")[..7]));
        assert!(v.pages.read(cx).rows.iter().all(|r| r.sha.as_ref() != f.sha("stash_message")), "the stash commit itself has no row");
        assert_eq!(v.pages.read(cx).rows.iter().filter(|r| r.stash().is_some()).count(), 3);
    });
}

#[gpui::test]
fn git_tab_stacks_a_rows_refs_and_lists_them_on_hover(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let (ix, cell) = view.read_with(cx, |v, cx| {
        let (ix, second) = graph_row(v, f.sha("second"), cx);
        (ix, ref_cell(&second.row, false))
    });
    assert_eq!((cell.first.as_ref().map(|g| g.name.as_str()), cell.more), (Some("side"), 1), "the worktree's branch, then +1 for v1");
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    assert!(hits.borrow().contains_key(&format!("git-more-{ix}")) && !hits.borrow().contains_key(&format!("git-reflist-{ix}-1")));
    let at = hits.borrow()[&format!("git-refs-{ix}")].bounds.center();
    cx.simulate_mouse_move(at, None, Modifiers::none());
    redraw(&view, cx);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.listed(), Some(ix), "resting on the cell opens its list");
        let open = ref_cell(&v.pages.read(cx).rows[ix].row, true);
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
    let (_, ahead) = view.read_with(cx, |v, cx| { let (i, r) = graph_row(v, f.sha("ahead"), cx); (i, r.row.clone()) });
    assert!(ref_cell(&ahead, true).list.is_empty(), "a single group opens no list");
}

#[gpui::test]
fn git_tab_puts_the_wip_row_on_top_of_a_dirty_tree_and_none_on_a_clean_one(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    view.read_with(cx, |v, cx| {
        let top = &v.pages.read(cx).rows[0];
        assert!(top.wip() && v.pages.read(cx).rows[1].sha.as_ref() == f.sha("ahead"), "WIP, then HEAD's commit");
        assert!(top.row.edges().all(|e| e.dashed), "its edge to HEAD is dashed");
        assert_eq!(v.wip.map(|w| w.counts()).as_deref(), Some("+2 ✎2 −1"), "s.txt and u.txt added, a.txt and the rename changed, b.txt deleted");
        assert_eq!(v.pages.read(cx).rows.iter().filter(|r| r.wip()).count(), 1);
    });
    f.git(&["reset", "-q", "--hard"]);
    f.git(&["clean", "-qfd"]);
    view.update(cx, |v, cx| v.reload(cx));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert!(v.pages.read(cx).rows.iter().all(|r| !r.wip()) && v.wip.is_none(), "a clean tree has no WIP row");
        assert_eq!(v.pages.read(cx).rows[0].sha.as_ref(), f.sha("ahead"));
    });
}

#[gpui::test]
fn git_tab_paints_a_long_history_without_a_stall_and_pages_at_the_end(cx: &mut TestAppContext) {
    let g = Generated::shared(20_000);
    let loading = cx.update(|cx| {
        let view = GitView::open(g.dir.clone(), None, HitTargets::default(), &GitSizes::default(), cx);
        matches!(view.read(cx).load, Load::Loading) && view.read(cx).pages.read(cx).rows.is_empty()
    });
    assert!(loading, "the first page is read off the UI thread");
    let (view, cx) = open(&g.dir, Opts::default(), cx);
    let commits = |v: &GitView, cx: &App| v.pages.read(cx).rows.iter().filter(|r| !r.wip()).count();
    let first = view.read_with(cx, |v, cx| (commits(v, cx), v.pages.read(cx).paging));
    assert_eq!(first, (FIRST_PAGE, false));
    let last = view.read_with(cx, |v, cx| v.pages.read(cx).rows.len() - 1);
    view.read_with(cx, |v, _| v.table.reveal(FIRST_PAGE / 2));
    redraw(&view, cx);
    assert_eq!(view.read_with(cx, |v, cx| commits(v, cx)), FIRST_PAGE, "half way down loads nothing");
    view.read_with(cx, |v, _| v.table.reveal(last));
    redraw(&view, cx);
    redraw(&view, cx);
    assert_eq!(view.read_with(cx, |v, cx| commits(v, cx)), FIRST_PAGE + NEXT_PAGE, "the end loads the next page");
    let shas: std::collections::HashSet<_> = view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().filter(|r| !r.wip()).map(|r| r.sha.clone()).collect());
    assert_eq!(shas.len(), FIRST_PAGE + NEXT_PAGE, "no commit twice");
    let paging = view.update(cx, |v, cx| {
        v.load_more(cx);
        (v.pages.read(cx).paging, commits(v, cx))
    });
    assert_eq!(paging, (true, FIRST_PAGE + NEXT_PAGE), "the next page is read off the UI thread");
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, cx| commits(v, cx)), FIRST_PAGE + 2 * NEXT_PAGE);
}

#[gpui::test]
fn git_tab_gives_two_authors_their_initials_and_distinct_lane_rings(cx: &mut TestAppContext) {
    let f = Fixture::copy();
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
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let palette = cx.update(|_, cx| Palette::of(cx.theme()));
    view.read_with(cx, |v, cx| {
        let (_, b) = graph_row(v, &bea, cx);
        let (_, a) = graph_row(v, f.sha("ahead"), cx);
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
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _sub = cx.update({
        let (view, seen) = (view.clone(), seen.clone());
        move |_, cx| cx.subscribe(&view, move |v, GitEvent::Selected, cx| seen.borrow_mut().push(v.read(cx).selected(cx)))
    });
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let row = hits.borrow()["git-row-1"].bounds.center();
    cx.simulate_click(row, Modifiers::none());
    cx.simulate_keystrokes("up down");
    let ahead = Selected::Commit(f.sha("ahead").to_string().into());
    assert_eq!(*seen.borrow(), vec![Some(ahead.clone()), Some(Selected::Wip), Some(ahead)]);
}

#[gpui::test]
fn git_tab_walks_as_a_ref_hidden_before_its_first_load_ended(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    let first = load::first(None, &f.dir, &ref_panel::Walk::default());
    view.update(cx, |v, cx| {
        (v.repo, v.load) = (None, Load::Loading);
        v.ref_panel().update(cx, |p, cx| p.toggle_hidden(&["refs/heads/main".to_string()], cx));
    });
    view.update(cx, |v, cx| v.apply_first(first, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, cx| assert_eq!(v.follow.walked, *v.ref_panel.read(cx).walk(), "the graph walks as the panel says"));
}

#[gpui::test]
fn git_tab_says_a_gitman_key_that_does_not_parse_and_loads(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("settings.toml"), "[gitman]\nfetch_every_minutes = \"often\"\n").unwrap();
    let (view, cx) = open_unchecked(&f.dir, Opts::default().home(home.path()), cx);
    let notice = view.read_with(cx, |v, _| (matches!(v.load, Load::Ready), v.settings_notice.clone()));
    assert!(notice.0, "the graph loads");
    assert!(notice.1.as_deref().is_some_and(|n| n.starts_with("[gitman] in settings.toml is ignored")), "{notice:?}");
    assert!(super::test_support::repaint(&view, cx).borrow().contains_key("git-settings-notice-dismiss"), "the banner is painted");
}

#[gpui::test]
fn git_tab_says_why_its_settings_give_no_git_and_retries(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let home = tempfile::tempdir().unwrap();
    let settings = home.path().join("settings.toml");
    let said = |view: &Entity<GitView>, cx: &mut VisualTestContext| {
        view.read_with(cx, |v, _| match &v.load {
            Load::Problem(Problem::Git(why)) => why.clone(),
            Load::Problem(Problem::Failed(why)) => format!("failed: {why}"),
            Load::Loading => "loading".into(),
            Load::Ready => "ready".into(),
        })
    };
    std::fs::write(&settings, "[gitman]
git = 5
").unwrap();
    let (view, cx) = open_unchecked(&f.dir, Opts::default().home(home.path()), cx);
    assert!(said(&view, cx).starts_with("[gitman] in settings.toml is ignored"), "{}", said(&view, cx));
    std::fs::write(&settings, "not toml [").unwrap();
    super::test_support::click(&view, "git-retry", cx);
    cx.run_until_parked();
    assert!(said(&view, cx).starts_with("[gitman] git is not read"), "{}", said(&view, cx));
    std::fs::remove_file(&settings).unwrap();
    super::test_support::click(&view, "git-retry", cx);
    cx.run_until_parked();
    assert_eq!(said(&view, cx), "ready", "Retry loads the graph");
}

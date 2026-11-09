//! The ref panel's WORKTREES rows and the worktree panel's header through the real tab, on a fixture copy whose
//! linked worktrees are in its temp dir (C196): marks, folder words, the WIP summary, Refresh WIPs, and a read the
//! repository's locks held off run again once the tab's write ends.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_gitman::BackgroundSettings;
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::write::LockSet;
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;

use super::background::{self, refresh_wips};
use super::ref_panel::{Entry, RefPanel, RefWips};
use super::tests::redraw;
use super::{GitSizes, GitView, Selected, bind_keys};
use crate::occupancy;

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1400.)).h(px(900.)).child(self.0.clone())
    }
}

const QUIET: BackgroundSettings = BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), worktree_wip: None };

struct Trees {
    f: Fixture,
    /// On branch `extra`, its folder's name.
    extra: PathBuf,
    /// Its folder removed: prunable.
    gone: PathBuf,
}

/// The fixture's `wt` (on `side`) locked with a reason and tracking `main`, `extra` on its own branch, and `gone`
/// whose folder was removed.
fn trees() -> Trees {
    let f = Fixture::copy();
    let (extra, gone) = (f.temp.path().join("extra"), f.temp.path().join("gone"));
    f.git(&["worktree", "add", "-q", "-b", "extra", &extra.to_string_lossy(), f.sha("root")]);
    f.git(&["worktree", "add", "-q", "--detach", &gone.to_string_lossy(), f.sha("root")]);
    std::fs::remove_dir_all(&gone).unwrap();
    f.git(&["worktree", "lock", "--reason", "on a stick", &f.worktree.to_string_lossy()]);
    git_in(&f.git, &f.worktree, &["branch", "-q", "--set-upstream-to=main", "side"], &[], None);
    Trees { f, extra, gone }
}

fn open<'a>(f: &Fixture, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        supraman_editor::init(supraman_ui::fonts::MONO, 13., cx);
        bind_keys(cx);
        background::start_for_test(QUIET, cx);
    });
    let folder = f.dir.clone();
    let live: Arc<dyn supraman_gitman::write::Occupancy> = Arc::new(occupancy::Live::default());
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open_with(folder, None, HitTargets::default(), &GitSizes::default(), live, cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    settle(&view, cx);
    (view, cx)
}

fn settle(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        redraw(view, cx);
    }
}

fn reread(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let id = view.entity_id();
    cx.update(|_, cx| refresh_wips(id, cx));
    settle(view, cx);
}

fn reads(view: &Entity<GitView>, cx: &mut VisualTestContext) -> usize {
    let id = view.entity_id();
    cx.update(|_, cx| background::wips(id, cx).map_or(0, |w| w.read(cx).reads))
}

fn panel(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Entity<RefPanel> {
    view.read_with(cx, |v, _| v.ref_panel().clone())
}

/// The panel path of the worktree at `dir`.
fn row(view: &Entity<GitView>, dir: &Path, cx: &mut VisualTestContext) -> String {
    let panel = panel(view, cx);
    panel.read_with(cx, |p, _| {
        p.entries().iter().map(|(path, _)| path.clone()).find(|path| p.worktree(path).is_some_and(|w| path_key(&w.path) == path_key(dir))).expect("a row of the worktree")
    })
}

fn entry(view: &Entity<GitView>, path: &str, cx: &mut VisualTestContext) -> Entry {
    panel(view, cx).read_with(cx, |p, _| p.entry(path).cloned()).expect("an entry")
}

/// The summary's words as the row shows them, `reading` for its spinner.
fn summary(view: &Entity<GitView>, path: &str, cx: &mut VisualTestContext) -> Option<String> {
    panel(view, cx).read_with(cx, |p, _| p.summary(path)).map(|s| match s {
        super::ref_panel::Summary::Reading => "reading".to_string(),
        super::ref_panel::Summary::Said { upstream, counts } => [upstream, counts].into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "),
    })
}

fn painted(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) -> bool {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    hits.borrow().contains_key(key)
}

fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let position = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    let (button, modifiers) = (MouseButton::Left, Modifiers::none());
    cx.simulate_event(MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position, button, modifiers, click_count: 1 });
}

#[gpui::test]
fn a_worktree_row_shows_its_icon_its_lock_or_prunable_mark_its_folder_where_it_differs_and_its_wip(cx: &mut TestAppContext) {
    let t = trees();
    std::fs::write(t.f.worktree.join("new.txt"), b"new\n").unwrap();
    std::fs::write(t.f.worktree.join("a.txt"), b"changed\n").unwrap();
    let (view, cx) = open(&t.f, cx);
    reread(&view, cx);
    let (main, wt, extra, gone) = (row(&view, &t.f.dir, cx), row(&view, &t.f.worktree, cx), row(&view, &t.extra, cx), row(&view, &t.gone, cx));

    assert_eq!(RefPanel::lead_icon(&entry(&view, &main, cx)), Some(Icon::Home));
    assert_eq!(RefPanel::lead_icon(&entry(&view, &wt, cx)), Some(Icon::Worktree));
    assert!(matches!(entry(&view, &wt, cx), Entry::Worktree { label, folder: Some(folder), .. } if label == "side" && folder == "wt"), "the folder beside a branch of another name");
    assert!(matches!(entry(&view, &extra, cx), Entry::Worktree { label, folder: None, .. } if label == "extra"), "no folder where it is the branch's name");

    let marks = |cx: &mut VisualTestContext| [&main, &wt, &extra, &gone].map(|p| painted(&view, &format!("refpanel-mark-{p}"), cx));
    assert_eq!(marks(cx), [false, true, false, true], "a mark on the locked and the prunable worktree only");
    let (locked, pruned) = panel(&view, cx).read_with(cx, |p, _| (p.worktree(&wt).cloned().unwrap(), p.worktree(&gone).cloned().unwrap()));
    assert!(locked.locked && !locked.prunable && locked.lock_reason == "on a stick");
    assert!(pruned.prunable && !pruned.prune_reason.is_empty(), "git's reason, the mark's tooltip");

    let ahead = git_in(&t.f.git, &t.f.worktree, &["rev-list", "--count", "main..side"], &[], None);
    let behind = git_in(&t.f.git, &t.f.worktree, &["rev-list", "--count", "side..main"], &[], None);
    let upstream = [(ahead.trim(), "↑"), (behind.trim(), "↓")].iter().filter(|(n, _)| *n != "0").map(|(n, a)| format!("{n}{a}")).collect::<Vec<_>>().join(" ");
    assert!(!upstream.is_empty(), "side and main differ");
    assert_eq!(summary(&view, &wt, cx), Some(format!("{upstream} +1 ✎1")), "the upstream's arrows, then the graph row's counts");
    assert!(painted(&view, &format!("refpanel-wip-{wt}"), cx));
    assert_eq!(summary(&view, &main, cx), None, "the tab's own worktree: its WIP is the graph's WIP row");
    assert_eq!(summary(&view, &extra, cx), None, "clean, no upstream: nothing");

    let p = panel(&view, cx);
    let landed = p.read_with(cx, |p, _| p.summary(&wt));
    p.update(cx, |p, cx| p.set_wips(RefWips { reading: true, landed: false, own: Some(path_key(&t.f.dir)), said: Default::default() }, cx));
    assert_eq!(summary(&view, &wt, cx).as_deref(), Some("reading"), "a spinner while the first read runs");
    assert!(painted(&view, &format!("refpanel-wip-reading-{wt}"), cx));
    assert!(painted(&view, "refpanel-worktrees-refresh-turning", cx), "Refresh WIPs turns while a read runs");
    assert_eq!(summary(&view, &main, cx), None, "never on the tab's own");
    p.update(cx, |p, cx| p.set_wips(RefWips { reading: true, landed: true, own: None, said: Default::default() }, cx));
    assert_eq!(summary(&view, &wt, cx), None, "no spinner once a read has landed");
    assert!(landed.is_some());
}

#[gpui::test]
fn refresh_wips_reads_every_other_worktree_again(cx: &mut TestAppContext) {
    let t = trees();
    let (view, cx) = open(&t.f, cx);
    reread(&view, cx);
    let (wt, extra) = (row(&view, &t.f.worktree, cx), row(&view, &t.extra, cx));
    assert!(painted(&view, "refpanel-worktrees-refresh", cx), "the repository has other worktrees");
    assert_eq!(summary(&view, &extra, cx), None);
    let before = summary(&view, &wt, cx);

    std::fs::write(t.extra.join("new.txt"), b"new\n").unwrap();
    std::fs::write(t.f.worktree.join("new.txt"), b"new\n").unwrap();
    settle(&view, cx);
    assert_eq!(summary(&view, &extra, cx), None, "not before the click");
    assert_eq!(summary(&view, &wt, cx), before);

    let read = reads(&view, cx);
    let turned = Rc::new(RefCell::new(Vec::new()));
    let seen = turned.clone();
    let p = panel(&view, cx);
    let _watch = cx.update(|_, cx| cx.observe(&p, move |p, cx| seen.borrow_mut().push(p.read(cx).wips_reading())));
    click(&view, "refpanel-worktrees-refresh", cx);
    settle(&view, cx);
    assert_eq!(reads(&view, cx), read + 1);
    assert!(turned.borrow().contains(&true), "Refresh WIPs turned while the read ran");
    assert!(!p.read_with(cx, |p, _| p.wips_reading()), "and stopped once it landed");
    assert_eq!(summary(&view, &extra, cx).as_deref(), Some("+1"), "after the click");
    assert!(summary(&view, &wt, cx).is_some_and(|s| s.ends_with("+1")), "every other worktree");
}

#[gpui::test]
fn refresh_wips_shows_only_while_the_repository_has_another_worktree(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    f.git(&["worktree", "remove", "--force", &f.worktree.to_string_lossy()]);
    let (view, cx) = open(&f, cx);
    assert!(painted(&view, "refpanel-row-worktrees", cx), "the section shows its one worktree");
    assert!(!painted(&view, "refpanel-worktrees-refresh", cx));
}

#[gpui::test]
fn the_worktree_panel_header_says_its_upstreams_ahead_and_behind(cx: &mut TestAppContext) {
    let t = trees();
    std::fs::write(t.f.worktree.join("new.txt"), b"new\n").unwrap();
    let (view, cx) = open(&t.f, cx);
    reread(&view, cx);
    let wt = row(&view, &t.f.worktree, cx);
    let upstream = summary(&view, &wt, cx).unwrap().trim_end_matches("+1").trim().to_string();
    let ix = view.read_with(cx, |v, _| v.rows.iter().position(|r| r.worktree().is_some_and(|p| path_key(p) == path_key(&t.f.worktree)))).expect("its graph row");
    view.update(cx, |v, cx| {
        v.table.select(Some(ix));
        v.selection_moved(cx);
    });
    settle(&view, cx);
    assert!(matches!(view.read_with(cx, |v, _| v.selected()), Some(Selected::Worktree(_))));
    let said = view.update(cx, |v, cx| v.worktree_read(cx).1).expect("read");
    assert_eq!(said.upstream, upstream, "the ref panel's words");
    assert!(painted(&view, "worktree-upstream", cx));
    assert!(painted(&view, "worktree-counts", cx));
}

#[gpui::test]
fn a_read_the_tabs_write_held_off_runs_again_once_the_write_ends(cx: &mut TestAppContext) {
    let t = trees();
    let (view, cx) = open(&t.f, cx);
    reread(&view, cx);
    let extra = row(&view, &t.extra, cx);
    let read = reads(&view, cx);
    view.update(cx, |v, cx| v.refresh_after_write(cx));
    settle(&view, cx);
    assert_eq!(reads(&view, cx), read, "nothing skipped: a write's refresh reads nothing");

    let writer = view.read_with(cx, |v, cx| v.remote.read(cx).writer().cloned()).expect("a writer");
    let held = writer.scheduler().try_acquire(LockSet::checkout()).expect("the locks free");
    std::fs::write(t.extra.join("new.txt"), b"new\n").unwrap();
    reread(&view, cx);
    assert_eq!(reads(&view, cx), read, "the write's locks hold the read off");
    drop(held);
    settle(&view, cx);
    assert_eq!(summary(&view, &extra, cx), None, "nothing wakes it by itself");

    view.update(cx, |v, cx| v.refresh_after_write(cx));
    settle(&view, cx);
    assert_eq!(reads(&view, cx), read + 1, "the write's refresh runs the skipped read");
    assert_eq!(summary(&view, &extra, cx).as_deref(), Some("+1"));
}

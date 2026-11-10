//! The background fetch and maintenance through real tabs on fixture copies and their temp bare `origin` (C196), the
//! test executor's clock driving the intervals.

use std::cell::Cell;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::write::{self, LockSet};
use supraman_gitman::{BackgroundSettings, Repo};
use supraman_ui::hit::HitTargets;

use super::maintain::{AFTER, EVERY, RETRY};
use super::*;
use crate::git_view::{GitSizes, bind_keys};
use crate::occupancy;

const MINUTE: Duration = Duration::from_secs(60);

thread_local! {
    static NOW: Cell<i64> = const { Cell::new(1_000_000_000_000) };
}

fn now() -> i64 {
    NOW.get()
}

fn held() -> Option<Box<dyn std::any::Any + Send>> {
    Some(Box::new(()))
}

/// The background on for this test, its heavy-job lock free (the gate's own cargo holds the real one) and its clock
/// the test's.
fn start(home: Option<&Path>, settings: BackgroundSettings, cx: &mut TestAppContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        bind_keys(cx);
        let mut bg = Background::new(home.map(Path::to_path_buf));
        bg.on = true;
        bg.settings = Some(settings);
        bg.heavy = Arc::new(held);
        bg.clock = now;
        cx.set_global(bg);
    });
}

fn open(folder: &Path, home: Option<&Path>, live: occupancy::Live, cx: &mut VisualTestContext) -> Entity<GitView> {
    let (folder, home) = (folder.to_path_buf(), home.map(Path::to_path_buf));
    let view = cx.update(|_, cx| GitView::open_with(folder, home, HitTargets::default(), &GitSizes::default(), Arc::new(live), cx));
    cx.run_until_parked();
    crate::git_view::test_support::assert_loaded(&view, cx);
    view
}

fn window(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.add_empty_window()
}

/// A commit on `origin`'s main made from a clone of it; its sha.
fn advance_origin(f: &Fixture, n: usize) -> String {
    let origin = f.temp.path().join("origin.git");
    let other = f.temp.path().join("other");
    if !other.exists() {
        git_in(&f.git, f.temp.path(), &["clone", "-q", "-b", "main", &origin.to_string_lossy(), "other"], &[], None);
    }
    let env = [("GIT_AUTHOR_NAME", "t".to_string()), ("GIT_AUTHOR_EMAIL", "t@t".to_string()), ("GIT_COMMITTER_NAME", "t".to_string()), ("GIT_COMMITTER_EMAIL", "t@t".to_string())];
    git_in(&f.git, &other, &["commit", "-q", "--allow-empty", "-m", &format!("origin {n}")], &env, None);
    git_in(&f.git, &other, &["push", "-q", "origin", "HEAD:main"], &[], None);
    git_in(&f.git, &other, &["rev-parse", "HEAD"], &[], None)
}

/// The tab closed: its entity released at the next flush.
fn close(view: Entity<GitView>, cx: &mut VisualTestContext) {
    drop(view);
    cx.update(|_, _| ());
    cx.run_until_parked();
}

fn tracking(f: &Fixture) -> String {
    f.git(&["rev-parse", "refs/remotes/origin/main"])
}

fn tick(d: Duration, cx: &mut VisualTestContext) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn fetches(f: &Fixture, cx: &mut VisualTestContext) -> usize {
    let key = path_key(&write::Writer::open(&f.repo()).unwrap().dirs().common_dir);
    cx.update(|_, cx| cx.global::<Background>().repos.get(&key).map_or(0, |s| s.fetches))
}

#[gpui::test]
fn a_tab_fetches_on_open_on_activation_and_each_interval_never_during_a_push_and_not_at_interval_0(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    start(None, BackgroundSettings { fetch_every_minutes: Some(1), maintenance: Some(false), ..Default::default() }, cx);
    let cx = window(cx);
    let first = advance_origin(&f, 1);
    let view = open(&f.dir, None, occupancy::Live::default(), cx);
    assert_eq!(tracking(&f), first, "fetched as the tab opened");

    let second = advance_origin(&f, 2);
    tick(MINUTE / 2, cx);
    assert_eq!(tracking(&f), first, "not before the interval");
    tick(MINUTE / 2 + Duration::from_secs(1), cx);
    assert_eq!(tracking(&f), second, "fetched after a minute");

    let third = advance_origin(&f, 3);
    view.update(cx, |v, cx| v.set_shown(false, cx));
    view.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_eq!(tracking(&f), third, "fetched as the tab became the active one");

    let fourth = advance_origin(&f, 4);
    let dirs = write::Writer::open(&f.repo()).unwrap().dirs().clone();
    let push = write::scheduler(&dirs).acquire(LockSet::push());
    let before = fetches(&f, cx);
    tick(MINUTE + Duration::from_secs(1), cx);
    assert_eq!((tracking(&f), fetches(&f, cx)), (third, before), "no fetch while a push holds the repository");
    drop(push);
    tick(MINUTE + Duration::from_secs(1), cx);
    assert_eq!(tracking(&f), fourth, "the next interval fetches");

    let pull = write::scheduler(&dirs).acquire(LockSet::pull());
    view.update(cx, |v, cx| v.set_shown(false, cx));
    view.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_eq!(fetches(&f, cx), before + 1, "no fetch while a pull runs");
    drop(pull);

    let fifth = advance_origin(&f, 5);
    cx.update(|_, cx| apply(BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), ..Default::default() }, cx));
    cx.run_until_parked();
    tick(MINUTE * 10, cx);
    view.update(cx, |v, cx| v.set_shown(false, cx));
    view.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_ne!(tracking(&f), fifth, "interval 0 is off: no interval, no activation fetch");

    cx.update(|_, cx| apply(BackgroundSettings { fetch_every_minutes: Some(2), maintenance: Some(false), ..Default::default() }, cx));
    cx.run_until_parked();
    assert_eq!(tracking(&f), fifth, "a new interval applies to the open tab at once");
}

#[gpui::test]
fn tabs_of_one_repository_share_one_schedule_ended_with_the_last_and_a_failure_names_its_remote(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    start(None, BackgroundSettings { fetch_every_minutes: Some(1), maintenance: Some(false), ..Default::default() }, cx);
    let cx = window(cx);
    let main = open(&f.dir, None, occupancy::Live::default(), cx);
    let side = open(&f.worktree, None, occupancy::Live::default(), cx);
    let repos = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.global::<Background>().repos.values().map(|s| s.tabs.len()).collect::<Vec<_>>());
    assert_eq!(repos(cx), [2], "the worktree's tab joins its repository's schedule");
    assert_eq!(fetches(&f, cx), 1, "one fetch for both tabs");

    f.git(&["remote", "add", "gone", &f.temp.path().join("nowhere.git").to_string_lossy()]);
    main.update(cx, |v, cx| v.reload(cx));
    cx.run_until_parked();
    tick(MINUTE + Duration::from_secs(1), cx);
    let tip = main.update(cx, |v, cx| v.pull_tooltip(supraman_gitman::write::remote::PullChoice::FastForwardIfPossible, cx));
    assert!(tip.contains("Last background fetch from gone failed:") && !tip.contains("from origin"), "{tip}");
    tick(MINUTE + Duration::from_secs(1), cx);
    let tip = main.update(cx, |v, cx| v.pull_tooltip(supraman_gitman::write::remote::PullChoice::FastForwardIfPossible, cx));
    assert!(tip.contains("from gone failed (2 in a row):"), "{tip}");

    close(side, cx);
    assert_eq!(repos(cx), [1]);
    close(main, cx);
    assert!(repos(cx).is_empty(), "the last tab closed ends the schedule");
    let before = advance_origin(&f, 9);
    tick(MINUTE * 3, cx);
    assert_ne!(tracking(&f), before, "nothing fetches once the tabs are closed");
}

fn graph(dirs: &supraman_gitman::Dirs) -> bool {
    let info = dirs.common_dir.join("objects").join("info");
    info.join("commit-graph").is_file() || info.join("commit-graphs").is_dir()
}

fn clear(dirs: &supraman_gitman::Dirs) {
    let info = dirs.common_dir.join("objects").join("info");
    let _ = std::fs::remove_file(info.join("commit-graph"));
    let _ = std::fs::remove_dir_all(info.join("commit-graphs"));
}

fn dirs_of(dir: &Path, f: &Fixture) -> supraman_gitman::Dirs {
    write::Writer::open(&Repo::new(f.git.clone(), dir)).unwrap().dirs().clone()
}

#[gpui::test]
fn maintenance_runs_once_per_hour_after_a_tab_opens_and_a_restart_does_not_run_it_again(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let dirs = dirs_of(&f.dir, &f);
    clear(&dirs);
    start(Some(home.path()), BackgroundSettings { fetch_every_minutes: Some(0), maintenance: None, ..Default::default() }, cx);
    let cx = window(cx);
    let view = open(&f.dir, Some(home.path()), occupancy::Live::default(), cx);
    tick(AFTER - Duration::from_secs(1), cx);
    assert!(!graph(&dirs), "not before 15 s");
    tick(Duration::from_secs(2), cx);
    assert!(graph(&dirs), "ran 15 s after the tab opened");
    assert!(super::maintain::path(home.path()).is_file(), "its run is kept on disk");

    clear(&dirs);
    tick(EVERY / 2, cx);
    NOW.set(NOW.get() + (EVERY / 2).as_millis() as i64);
    assert!(!graph(&dirs), "not again within the hour");
    NOW.set(NOW.get() + (EVERY / 2).as_millis() as i64 + 1000);
    tick(EVERY / 2 + Duration::from_secs(1), cx);
    assert!(graph(&dirs), "again after an hour");

    close(view, cx);
    clear(&dirs);
    cx.update(|_, cx| cx.remove_global::<Background>());
    NOW.set(NOW.get() + 1000);
    cx.update(|_, cx| {
        let mut bg = Background::new(Some(home.path().to_path_buf()));
        bg.on = true;
        bg.settings = Some(BackgroundSettings { fetch_every_minutes: Some(0), maintenance: None, ..Default::default() });
        bg.heavy = Arc::new(held);
        bg.clock = now;
        cx.set_global(bg);
    });
    let _again = open(&f.dir, Some(home.path()), occupancy::Live::default(), cx);
    tick(AFTER + RETRY, cx);
    assert!(!graph(&dirs), "a restart within the hour does not run it again");
}

#[gpui::test]
fn maintenance_waits_while_a_session_works_there_an_index_is_locked_or_a_heavy_job_runs_and_not_when_off(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let dirs = dirs_of(&f.dir, &f);
    clear(&dirs);
    start(None, BackgroundSettings { fetch_every_minutes: Some(0), maintenance: None, ..Default::default() }, cx);
    let cx = window(cx);
    let live = occupancy::Live::default();
    live.set_sessions(vec![occupancy::LiveSession { name: "fm-alpha".into(), cwd: f.worktree.join("src"), activity: supraman_ui::marks::Activity::Working, attach: None }]);
    let _view = open(&f.dir, None, live.clone(), cx);
    tick(AFTER + Duration::from_secs(1), cx);
    assert!(!graph(&dirs), "a session works in the repository's other worktree");

    live.set_sessions(Vec::new());
    let lock = dirs.git_dir.join("index.lock");
    std::fs::write(&lock, b"").unwrap();
    tick(RETRY, cx);
    assert!(!graph(&dirs), "a git holds the index");

    std::fs::remove_file(&lock).unwrap();
    cx.update(|_, cx| cx.global_mut::<Background>().heavy = Arc::new(|| None));
    tick(RETRY, cx);
    assert!(!graph(&dirs), "a build or selftest holds the heavy-job lock");

    cx.update(|_, cx| apply(BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), ..Default::default() }, cx));
    cx.update(|_, cx| cx.global_mut::<Background>().heavy = Arc::new(held));
    tick(RETRY * 3, cx);
    assert!(!graph(&dirs), "turned off");

    cx.update(|_, cx| apply(BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(true), ..Default::default() }, cx));
    cx.run_until_parked();
    assert!(graph(&dirs), "turned on, it runs at the next look");
}

fn reads(view: &Entity<GitView>, cx: &mut VisualTestContext) -> usize {
    let id = view.entity_id();
    cx.update(|_, cx| super::wips(id, cx).map_or(0, |w| w.read(cx).reads))
}

fn others(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(String, u32)> {
    let mut others: Vec<(String, u32)> = view.update(cx, |v, cx| v.other_wips(cx)).iter().map(|w| (w.worktree.branch.clone().unwrap_or_default(), w.counts().total())).collect();
    others.sort();
    others
}

#[gpui::test]
fn the_other_worktrees_are_read_on_open_activation_the_fetch_tick_and_a_refresh_never_on_a_timer_of_their_own(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    for n in 0..4 {
        f.git(&["worktree", "add", "-q", "-b", &format!("extra{n}"), &f.temp.path().join(format!("extra{n}")).to_string_lossy(), f.sha("second")]);
    }
    start(None, BackgroundSettings { fetch_every_minutes: Some(1), maintenance: Some(false), ..Default::default() }, cx);
    let cx = window(cx);
    let main = open(&f.dir, None, occupancy::Live::default(), cx);
    assert_eq!(reads(&main, cx), 1, "read as the tab opened");
    let shown = others(&main, cx);
    assert_eq!(shown, [("extra0".into(), 0), ("extra1".into(), 0), ("extra2".into(), 0), ("extra3".into(), 0), ("side".into(), 0)], "every worktree but the tab's own");

    tick(MINUTE / 2, cx);
    assert_eq!(reads(&main, cx), 1, "nothing between the ticks");
    tick(MINUTE / 2 + Duration::from_secs(1), cx);
    assert_eq!(reads(&main, cx), 2, "read at the fetch tick");

    main.update(cx, |v, cx| v.set_shown(false, cx));
    main.update(cx, |v, cx| v.set_shown(true, cx));
    cx.run_until_parked();
    assert_eq!(reads(&main, cx), 3, "read as the tab became the active one");

    std::fs::write(f.worktree.join("new.txt"), b"new\n").unwrap();
    cx.run_until_parked();
    assert_eq!(reads(&main, cx), 3, "no watcher on the other worktrees");
    cx.update(|_, cx| super::refresh_wips(main.entity_id(), cx));
    cx.run_until_parked();
    assert_eq!(reads(&main, cx), 4, "read on an explicit refresh");
    assert!(others(&main, cx).contains(&("side".into(), 1)), "{:?}", others(&main, cx));

    let dirs = write::Writer::open(&f.repo()).unwrap().dirs().clone();
    let push = write::scheduler(&dirs).acquire(LockSet::push());
    cx.update(|_, cx| super::refresh_wips(main.entity_id(), cx));
    cx.run_until_parked();
    assert_eq!(reads(&main, cx), 4, "no read while a push holds the repository");
    drop(push);
    cx.update(|_, cx| super::refresh_wips(main.entity_id(), cx));
    cx.run_until_parked();
    assert_eq!(reads(&main, cx), 5, "the next refresh reads");

    cx.update(|_, cx| apply(BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), ..Default::default() }, cx));
    cx.run_until_parked();
    tick(IDLE + MINUTE, cx);
    assert_eq!(reads(&main, cx), 5, "with the fetch off, no tick and no timer of its own");

    let side = open(&f.worktree, None, occupancy::Live::default(), cx);
    assert_eq!(reads(&main, cx), 6, "a tab joining reads once for the repository's tabs");
    assert_eq!(reads(&side, cx), 6, "one read shared by the repository's tabs");
    assert!(others(&side, cx).iter().any(|(branch, n)| branch == "main" && *n > 0), "the worktree's tab sees the main checkout: {:?}", others(&side, cx));
    assert!(!others(&side, cx).iter().any(|(branch, _)| branch == "side"));
    assert!(others(&main, cx).contains(&("side".into(), 1)));
}

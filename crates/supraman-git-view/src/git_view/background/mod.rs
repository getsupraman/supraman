//! A repository's background work while a Git tab shows it (docs/research/gitkraken-12.6.1.md §4 part B, §7): one
//! schedule per common git dir, shared by every tab and worktree of the repository and dropped with its last tab.
//! It fetches every remote silently when a tab opens, each time one becomes the active tab and every
//! `[gitman] fetch_every_minutes`, runs `git maintenance` ([`maintain`]) and reads the other worktrees ([`wips`]).

mod maintain;
mod wips;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, AsyncApp, Context, Entity, EntityId, Global, Subscription, Task, WeakEntity};
use supraman_core::Settings;
use supraman_core::paths::path_key;
use supraman_gitman::write::remote::{self, Fetched};
use supraman_gitman::write::{LockSet, WriteError, Writer};
use supraman_gitman::BackgroundSettings;
use supraman_ui::nudge::{self, Nudge, Nudged};

use super::GitView;

pub(crate) use wips::{Wips, after_write as wips_after_write, refresh as refresh_wips, wips};

/// How long a loop sleeps with nothing due: its schedule's tabs and a settings change wake it sooner.
const IDLE: Duration = Duration::from_secs(24 * 3600);

/// A remote whose background fetches failed: how many in a row, and the last one's reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) count: u32,
    pub(crate) reason: String,
}

struct Tab {
    view: WeakEntity<GitView>,
    _released: Subscription,
}

struct Schedule {
    tabs: Vec<(EntityId, Tab)>,
    fetch: Nudge,
    maintain: Nudge,
    wip: Nudge,
    wips: Entity<Wips>,
    failures: BTreeMap<String, Failure>,
    #[cfg(test)]
    fetches: usize,
    _loops: [Task<()>; 3],
}

/// A guard that keeps a build, a test or a selftest from overlapping the job that holds it; `None` while one runs.
pub(crate) type HeavyLock = Arc<dyn Fn() -> Option<Box<dyn std::any::Any + Send>> + Send + Sync>;

pub(crate) struct Background {
    home: Option<PathBuf>,
    /// `None` until `settings.toml` is read.
    settings: Option<BackgroundSettings>,
    repos: HashMap<String, Schedule>,
    heavy: HeavyLock,
    clock: fn() -> i64,
    ran: maintain::Ran,
    #[cfg(test)]
    on: bool,
}

impl Global for Background {}

/// A test's tabs work in the background only when its `Background` says so.
fn enabled(cx: &App) -> bool {
    #[cfg(test)]
    return cx.try_global::<Background>().is_some_and(|b| b.on);
    #[cfg(not(test))]
    {
        let _ = cx;
        true
    }
}

fn heavy_job_lock() -> Option<Box<dyn std::any::Any + Send>> {
    match supraman_core::fs::try_heavy_job_lock() {
        Ok(held) => held.map(|f| Box::new(f) as Box<dyn std::any::Any + Send>),
        Err(e) => {
            log::warn!("git maintenance: cannot take the heavy-job lock: {e}");
            None
        }
    }
}

impl Background {
    fn new(home: Option<PathBuf>) -> Background {
        let settings = home.is_none().then(BackgroundSettings::default);
        Background { home, settings, repos: HashMap::new(), heavy: Arc::new(heavy_job_lock), clock: supraman_core::time::now_ms, ran: maintain::Ran::default(), #[cfg(test)] on: false }
    }

    /// Created with `home`'s `[gitman]` read off the UI thread, once.
    fn ensure(home: Option<PathBuf>, cx: &mut App) {
        if cx.has_global::<Background>() {
            return;
        }
        cx.set_global(Background::new(home.clone()));
        let Some(home) = home else { return };
        let read = cx.background_spawn(async move { Settings::load(&home).map(|s| supraman_gitman::background_settings(&s)).unwrap_or_default() });
        cx.spawn(async move |cx| {
            let settings = read.await;
            cx.update(|cx| apply(settings, cx));
        })
        .detach();
    }

    /// Unread settings show the rows: `[gitman] worktree_wip` is on by default.
    fn worktree_wips(&self) -> bool {
        self.settings.is_none_or(|s| s.worktree_wips())
    }

    fn fetch_every(&self) -> Option<Duration> {
        let minutes = self.settings?.fetch_every();
        (minutes > 0).then(|| Duration::from_secs(u64::from(minutes) * 60))
    }

    /// The live tabs of `key`'s schedule.
    fn views(&self, key: &str) -> Vec<Entity<GitView>> {
        self.repos.get(key).map(|s| s.tabs.iter().filter_map(|(_, t)| t.view.upgrade()).collect()).unwrap_or_default()
    }
}

/// New settings for every schedule: each loop looks again now.
pub fn apply(settings: BackgroundSettings, cx: &mut App) {
    if !cx.has_global::<Background>() {
        return;
    }
    let bg = cx.global_mut::<Background>();
    let rows_turned = bg.worktree_wips() != settings.worktree_wips();
    bg.settings = Some(settings);
    for schedule in bg.repos.values() {
        schedule.fetch.wake();
        schedule.maintain.wake();
        if rows_turned {
            schedule.wip.wake();
        }
    }
    if !rows_turned {
        return;
    }
    let bg = cx.global::<Background>();
    let views: Vec<_> = bg.repos.keys().flat_map(|key| bg.views(key)).collect();
    for view in views {
        view.update(cx, |v, cx| v.worktrees_read(cx));
    }
}

/// The background on for a test of the tab, with `settings`, no home and its heavy-job lock free.
#[cfg(test)]
pub(crate) fn start_for_test(settings: BackgroundSettings, cx: &mut App) {
    let mut bg = Background::new(None);
    bg.on = true;
    bg.settings = Some(settings);
    bg.heavy = Arc::new(|| Some(Box::new(()) as Box<dyn std::any::Any + Send>));
    cx.set_global(bg);
}

/// Whether the graph shows the other worktrees' WIP rows: `[gitman] worktree_wip`, on without a background.
pub(crate) fn worktree_wips(cx: &App) -> bool {
    cx.try_global::<Background>().is_none_or(Background::worktree_wips)
}

/// The repository `writer` writes to, followed by the tab `view` until it closes; its first tab fetches now.
pub(super) fn join(view: &GitView, writer: &Writer, cx: &mut Context<GitView>) {
    if !enabled(cx) {
        return;
    }
    let id = cx.entity_id();
    Background::ensure(view.home.clone(), cx);
    let key = path_key(&writer.dirs().common_dir);
    if cx.global::<Background>().repos.get(&key).is_some_and(|s| s.tabs.iter().any(|(t, _)| *t == id)) {
        return;
    }
    let released = {
        let (key, entity) = (key.clone(), cx.entity());
        let app: &mut App = cx;
        app.observe_release(&entity, move |_, cx| left(&key, id, cx))
    };
    let tab = Tab { view: cx.weak_entity(), _released: released };
    if let Some(schedule) = cx.global_mut::<Background>().repos.get_mut(&key) {
        schedule.tabs.push((id, tab));
        schedule.wip.wake();
        return;
    }
    let (fetch, fetching) = nudge::nudge();
    let (maintain, maintaining) = nudge::nudge();
    let (wip, reading) = nudge::nudge();
    fetch.wake();
    wip.wake();
    let loops = [
        cx.spawn({
            let key = key.clone();
            async move |_, cx| fetch_loop(key, fetching, cx).await
        }),
        cx.spawn({
            let key = key.clone();
            async move |_, cx| maintain::run_loop(key, maintaining, cx).await
        }),
        cx.spawn({
            let key = key.clone();
            async move |_, cx| wips::run_loop(key, reading, cx).await
        }),
    ];
    let schedule = Schedule {
        tabs: vec![(id, tab)],
        fetch,
        maintain,
        wip,
        wips: cx.new(|_| Wips::default()),
        failures: BTreeMap::new(),
        #[cfg(test)]
        fetches: 0,
        _loops: loops,
    };
    cx.global_mut::<Background>().repos.insert(key, schedule);
}

fn left(key: &str, id: EntityId, cx: &mut App) {
    if !cx.has_global::<Background>() {
        return;
    }
    let repos = &mut cx.global_mut::<Background>().repos;
    if let Some(schedule) = repos.get_mut(key) {
        schedule.tabs.retain(|(t, _)| *t != id);
        if schedule.tabs.is_empty() {
            repos.remove(key);
        }
    }
}

/// The tab `id` became the active tab: its repository fetches and reads its worktrees now.
pub(super) fn shown(id: EntityId, cx: &mut App) {
    let Some(bg) = cx.try_global::<Background>() else { return };
    for schedule in bg.repos.values().filter(|s| s.tabs.iter().any(|(t, _)| *t == id)) {
        schedule.fetch.wake();
        schedule.wip.wake();
    }
}

/// What the Pull button says of the tab `id`'s repository: each remote whose last background fetch failed.
pub(super) fn failures(id: EntityId, cx: &App) -> Vec<String> {
    let Some(bg) = cx.try_global::<Background>() else { return Vec::new() };
    let Some(schedule) = bg.repos.values().find(|s| s.tabs.iter().any(|(t, _)| *t == id)) else { return Vec::new() };
    schedule
        .failures
        .iter()
        .map(|(remote, f)| match f.count {
            1 => format!("Last background fetch from {remote} failed: {}", f.reason),
            n => format!("Last background fetch from {remote} failed ({n} in a row): {}", f.reason),
        })
        .collect()
}

/// The fetch a tab of `key` can run now: none while a tab's gate or remote call is busy or an operation is under way.
struct FetchJob {
    writer: Writer,
    remotes: Vec<String>,
    prune: bool,
}

enum Step<T> {
    End,
    Skip,
    Run(T),
}

fn fetch_job(key: &str, cx: &App) -> Step<FetchJob> {
    let Some(bg) = cx.try_global::<Background>() else { return Step::End };
    let views = bg.views(key);
    if views.is_empty() {
        return Step::End;
    }
    if bg.fetch_every().is_none() {
        return Step::Skip;
    }
    let mut job = None;
    for view in &views {
        let remote = view.read(cx).remote.read(cx);
        if remote.busy(cx).is_some() || remote.under_way() {
            return Step::Skip;
        }
        if let Some(writer) = remote.writer().filter(|_| job.is_none() && remote.has_remotes()) {
            job = Some(FetchJob { writer: writer.clone(), remotes: remote.remote_names(), prune: remote.prune() });
        }
    }
    job.map_or(Step::Skip, Step::Run)
}

async fn fetch_loop(key: String, mut nudged: Nudged, cx: &mut AsyncApp) {
    let executor = cx.background_executor().clone();
    loop {
        let every = cx.update(|cx| cx.try_global::<Background>().and_then(Background::fetch_every));
        let ticked = !nudged.wait(&executor, every.unwrap_or(IDLE)).await && every.is_some();
        match cx.update(|cx| fetch_job(&key, cx)) {
            Step::End => return,
            Step::Skip => {}
            Step::Run(job) => {
                let fetched = executor.spawn(async move { job.writer.try_run_watched(LockSet::fetch(), |s| remote::background_fetch(s, &job.remotes, job.prune)) }).await;
                cx.update(|cx| fetched_into(&key, fetched, cx));
            }
        }
        if ticked {
            cx.update(|cx| cx.try_global::<Background>().and_then(|bg| bg.repos.get(&key)).map(|s| s.wip.wake()));
        }
    }
}

/// What git said of a failed fetch: its `fatal:` line, else its last.
fn reason(why: &str) -> String {
    let fatal = why.lines().find_map(|l| l.split_once("fatal: ").map(|(_, r)| r));
    fatal.or_else(|| why.lines().rev().find(|l| !l.trim().is_empty())).unwrap_or(why).trim().to_string()
}

/// The fetch's failures kept per remote, a success forgetting its remote's; the tabs read the repo again when a
/// ref moved.
fn fetched_into(key: &str, fetched: Option<Result<Fetched, WriteError>>, cx: &mut App) {
    let fetched = match fetched {
        None => return,
        Some(Err(e)) => return log::warn!("git tab: background fetch: {e}"),
        Some(Ok(fetched)) => fetched,
    };
    if !cx.has_global::<Background>() {
        return;
    }
    let Some(schedule) = cx.global_mut::<Background>().repos.get_mut(key) else { return };
    #[cfg(test)]
    {
        schedule.fetches += 1;
    }
    for (remote, done) in fetched.remotes {
        match done {
            Ok(()) => drop(schedule.failures.remove(&remote)),
            Err(why) => {
                let reason = reason(&why);
                let failure = schedule.failures.entry(remote).or_insert(Failure { count: 0, reason: String::new() });
                failure.count += 1;
                failure.reason = reason;
            }
        }
    }
    for view in cx.global::<Background>().views(key) {
        view.update(cx, |v, cx| {
            if fetched.moved {
                v.refresh_after_write(cx);
            }
            cx.notify();
        });
    }
}

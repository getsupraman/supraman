use std::{
    collections::{HashMap as StdHashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use futures::StreamExt as _;
use gpui::{AsyncApp, Context, WeakEntity, Window};
use supraman_claude::{Session, Transcripts, main_repo, now_ms};
use supraman_ui::nudge::{self, Nudged};
use supraman_ui::timeline;

use super::shells::powershell;
use super::{FleetView, QUIT_RELEASE_WITHIN, TabKind, held_line, left};
use crate::fleet;
use crate::hand::{Extras, ExtrasFrom, Hands, Launch};

const TRANSCRIPTS_EVERY: Duration = Duration::from_secs(10);
const NO_PLUGIN: &str = "No plugins/supraman found for this build: [+] sessions start plain claude, without the Supraman skills, hooks and sm.";
/// Claude draws a frame per wakeup; a hand tab looks at its token file at most this often.
const HAND_CHECK_GAP: Duration = Duration::from_millis(250);
const HAND_EXIT_FALLBACK: Duration = Duration::from_secs(30);

/// claude's first prompt, in a file the launch script reads, and its `--permission-mode`.
#[derive(Debug, Clone)]
pub struct FirstPrompt {
    pub file: PathBuf,
    pub mode: &'static str,
}

impl FirstPrompt {
    fn apply(first: Option<&Self>, mut extras: Extras) -> Extras {
        if let Some(first) = first {
            extras.args.extend(["--permission-mode".to_string(), first.mode.to_string()]);
            extras.prompt = Some(first.file.clone());
        }
        extras
    }
}

impl FleetView {
    pub(super) fn open_hands(home: &std::path::Path, cx: &mut Context<Self>) -> Arc<Mutex<Hands>> {
        let (hands, notice) = Hands::open(home);
        let hands = Arc::new(Mutex::new(hands));
        let stale = hands.clone();
        cx.spawn(async move |this, cx| {
            let mut notices = cx
                .background_executor()
                .spawn(async move { stale.lock().unwrap_or_else(|e| e.into_inner()).release_last_run() })
                .await;
            notices.extend(notice);
            this.update(cx, |this, cx| {
                this.hand_notices.extend(notices);
                this.lifecycle.hands_ready = true;
                this.reopen_tabs(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        let live = hands.clone();
        cx.on_app_quit(move |_, cx| {
            let live = live.clone();
            let release = cx.background_executor().spawn(async move { crate::hand::release_all_within(live, QUIT_RELEASE_WITHIN) });
            async move {
                release.await;
            }
        })
        .detach();
        hands
    }

    pub(super) fn watch_transcripts(claude_dir: Option<PathBuf>, cache: Option<PathBuf>, mut nudged: Nudged, cx: &mut Context<Self>) {
        let Some(claude_dir) = claude_dir else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut scanner: Option<Transcripts> = None;
            let mut repos: Repos = StdHashMap::new();
            let mut from_cache = false;
            loop {
                if scanner.is_none() {
                    let Some((opened, cached_first)) = open_transcripts(&this, claude_dir.clone(), cache.clone(), cx).await else {
                        return;
                    };
                    scanner = Some(opened);
                    from_cache = cached_first;
                }
                // the first scan waits for the first poll's apply, which wakes it with the workspaces and sessions
                nudged.wait(&executor, TRANSCRIPTS_EVERY).await;
                let Ok(asked) = this.update(cx, |this, _| this.transcripts_ask()) else {
                    return;
                };
                let Some(ask) = asked else { continue };
                let since = now_ms() - fleet::CLOSED_WINDOW_MS;
                let scanning = Instant::now();
                if std::mem::take(&mut from_cache)
                    && let Some(s) = scanner.take()
                {
                    let Some((back, known)) = list_cached(&this, s, &ask, since, scanning, cx).await else {
                        return;
                    };
                    scanner = Some(back);
                    repos = known;
                }
                let Some(s) = scanner.take() else { continue };
                let Some((back, known)) = scan_transcripts(&this, s, repos, ask, since, scanning, cx).await else {
                    return;
                };
                scanner = Some(back);
                repos = known;
            }
        })
        .detach();
    }

    fn transcripts_ask(&self) -> Option<TranscriptsAsk> {
        self.polls.sessions_read.then(|| TranscriptsAsk {
            roots: self.workspaces.iter().flat_map(|w| w.folders.iter().filter(|r| r.enabled).map(|r| r.path.clone())).collect(),
            keep: self.stars.ids().iter().cloned().collect(),
            live: self.sessions.iter().map(|s| s.session_id.clone()).collect(),
            asks: self.polls.refresh_asks,
        })
    }

    pub fn new_claude(&mut self, cwd: PathBuf, title: String, _window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let claude = self.claude_command.clone();
        self.hand_claude_in(None, cwd, title, claude, None, None, None, cx)
    }

    /// `into` is a prompt tab that becomes this session's tab.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn hand_claude_in(
        &mut self,
        into: Option<u64>,
        cwd: PathBuf,
        title: String,
        claude: String,
        own: Option<String>,
        resumes: Option<String>,
        first: Option<FirstPrompt>,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = match into.and_then(|id| self.tab_mut(id)) {
            Some(tab) => {
                tab.kind = TabKind::Shell;
                tab.title = title;
                tab.cwd = Some(cwd.clone());
                let id = tab.id;
                self.active = Some(id);
                cx.notify();
                id
            }
            None => self.push_tab(title, TabKind::Shell, Some(cwd.clone()), cx),
        };
        self.reveal_new(left::PendingReveal::TabSession(id));
        let (watch, changed) = nudge::nudge();
        if let Some(tab) = self.tab_mut(id) {
            tab.hand = true;
            tab.watchers.push(watch);
            tab.launched_session = resumes.clone();
            tab.session_name = own.clone();
        }
        let (from, hands, at) = (self.extras_from(), self.lifecycle.hands.clone(), cwd.clone());
        let mint = hands.clone();
        let ready = cx.background_executor().spawn(async move {
            let launch = mint.map_or(Launch::Plain, |m| crate::hand::launch_locked(&m, &at, own.as_deref(), resumes.as_deref()));
            let extras = from.map(|from| from.extras(&at, own.as_deref(), &launch)).unwrap_or_default();
            (FirstPrompt::apply(first.as_ref(), extras), launch)
        });
        cx.spawn(async move |this, cx| {
            let (extras, launch) = ready.await;
            this.update(cx, |this, cx| {
                if let Launch::Minted { name, session, notice, new_session, .. } = &launch {
                    if let Some(tab) = this.tab_mut(id) {
                        tab.workman = Some(format!("Supraman token · {} · {session}", crate::hand::ROLE));
                        tab.hand_key = Some(name.clone());
                        if new_session.is_some() {
                            tab.launched_session = new_session.clone();
                        }
                    }
                    this.hand_notices.extend(notice.clone());
                }
                if this.tab(id).is_some() {
                    let shell = this.claude_shell(&launch, &claude, &extras);
                    this.start_terminal_env(id, shell, Some(cwd), &extras, cx);
                }
            })
            .ok();
            let Some(hands) = hands else { return };
            let name = match launch {
                Launch::Minted { name, .. } => name,
                Launch::Unminted(notice) | Launch::Helped { notice: Some(notice), .. } => {
                    this.update(cx, |this, cx| {
                        this.hand_notices.push(notice);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                Launch::Plain | Launch::Helped { .. } => return,
            };
            Self::release_on_exit(this, id, hands, name, changed, cx).await;
        })
        .detach();
        id
    }

    /// The hand's token goes once its claude has exited or its tab is closed.
    async fn release_on_exit(this: gpui::WeakEntity<Self>, id: u64, hands: Arc<Mutex<Hands>>, name: String, mut changed: Nudged, cx: &mut gpui::AsyncApp) {
        let executor = cx.background_executor().clone();
        // claude's exit deletes the token file, then the shell prints its prompt: a wakeup
        loop {
            changed.wait(&executor, HAND_EXIT_FALLBACK).await;
            let open = this.read_with(cx, |this, _| this.tab(id).is_some_and(|t| !t.exited)).unwrap_or(false);
            if open {
                let (hands, name) = (hands.clone(), name.clone());
                if !executor.spawn(async move { hands.lock().unwrap_or_else(|e| e.into_inner()).ended(&name) }).await {
                    executor.timer(HAND_CHECK_GAP).await;
                    continue;
                }
            }
            let notice = executor.spawn(async move { hands.lock().unwrap_or_else(|e| e.into_inner()).release(&name) }).await;
            if let Some(notice) = notice {
                this.update(cx, |this, cx| {
                    this.hand_notices.push(notice);
                    cx.notify();
                })
                .ok();
            }
            return;
        }
    }

    /// A missing plugin starts plain claude and says so once.
    fn extras_from(&mut self) -> Option<ExtrasFrom> {
        let Some(plugin_dir) = self.plugin_dir.clone() else {
            if !self.hand_notices.iter().any(|n| n == NO_PLUGIN) {
                self.hand_notices.push(NO_PLUGIN.into());
            }
            return None;
        };
        Some(ExtrasFrom { home: self.home.clone(), plugin_dir, workspaces: self.workspaces.clone() })
    }

    fn claude_shell(&self, launch: &Launch, claude: &str, extras: &Extras) -> util::shell::Shell {
        let launch_args = match &self.supervising.daemon_hold {
            Some(why) => {
                let script = format!("{}; {}", held_line(why), launch.script_with(claude, extras));
                ["-NoExit".into(), "-EncodedCommand".into(), crate::hand::encoded_command(&script)]
            }
            None => launch.shell_args_with(claude, extras),
        };
        powershell(self.shell_args.iter().cloned().chain(launch_args).collect())
    }
}

struct TranscriptsAsk {
    roots: Vec<PathBuf>,
    keep: HashSet<String>,
    live: HashSet<String>,
    asks: u64,
}

type Repos = StdHashMap<PathBuf, Option<PathBuf>>;

async fn open_transcripts(this: &WeakEntity<FleetView>, claude_dir: PathBuf, cache: Option<PathBuf>, cx: &mut AsyncApp) -> Option<(Transcripts, bool)> {
    let opened = cx
        .background_executor()
        .spawn(async move {
            let s = Transcripts::in_claude_dir(&claude_dir);
            match cache {
                Some(cache) => s.with_cache(cache),
                None => s,
            }
        })
        .await;
    let has_cache = opened.has_cache();
    let cached_first = this
        .update(cx, |this, cx| {
            let cached_first = this.transcripts.opened(has_cache);
            this.check_loaded(cx);
            cx.notify();
            cached_first
        })
        .ok()?;
    Some((opened, cached_first))
}

async fn list_cached(this: &WeakEntity<FleetView>, s: Transcripts, ask: &TranscriptsAsk, since: i64, scanning: Instant, cx: &mut AsyncApp) -> Option<(Transcripts, Repos)> {
    let (roots, keep) = (ask.roots.clone(), ask.keep.clone());
    let (back, known, (closed, continued)) = cx
        .background_executor()
        .spawn(async move {
            let mut repos = StdHashMap::new();
            let found = s.cached(&roots, since, &keep);
            for t in &found {
                if let Some(cwd) = &t.cwd {
                    repos.entry(cwd.clone()).or_insert_with(|| main_repo(cwd));
                }
            }
            (s, repos, (found.iter().map(|t| t.to_session()).collect::<Vec<Session>>(), supraman_claude::chains(&found)))
        })
        .await;
    let repos = known.clone();
    this.update(cx, |this, cx| {
        timeline(&format!("transcripts from the cache done ({} listed)", closed.len()), scanning.elapsed());
        this.listing.main_repos.extend(known.clone());
        this.listing.closed_repos = known;
        this.set_closed(closed, continued, cx);
        this.transcripts.listed_cached();
        this.check_loaded(cx);
        cx.notify();
    })
    .ok()?;
    Some((back, repos))
}

async fn scan_transcripts(
    this: &WeakEntity<FleetView>,
    mut s: Transcripts,
    mut repos: Repos,
    ask: TranscriptsAsk,
    since: i64,
    scanning: Instant,
    cx: &mut AsyncApp,
) -> Option<(Transcripts, Repos)> {
    let (progress, mut progressed) = futures::channel::mpsc::unbounded::<(usize, usize)>();
    let shown = this.clone();
    let showing = cx.spawn(async move |cx| {
        while let Some(at) = progressed.next().await {
            if shown
                .update(cx, |this, cx| {
                    this.transcripts.reading(at);
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
        }
    });
    let TranscriptsAsk { roots, keep, live, asks } = ask;
    let (back, known, (closed, continued), read) = cx
        .background_executor()
        .spawn(async move {
            let read = AtomicUsize::new(0);
            let found = s.scan_with(&roots, since, &keep, &live, &|done, of| {
                read.store(of, Ordering::Relaxed);
                if done == of || done % (of / 100).max(1) == 0 {
                    progress.unbounded_send((done, of)).ok();
                }
            });
            drop(progress);
            for t in &found {
                if let Some(cwd) = &t.cwd {
                    repos.entry(cwd.clone()).or_insert_with(|| main_repo(cwd));
                }
            }
            let closed: Vec<Session> = found.iter().map(|t| t.to_session()).collect();
            (s, repos, (closed, supraman_claude::chains(&found)), read.into_inner())
        })
        .await;
    showing.await;
    let repos = known.clone();
    this.update(cx, |this, cx| {
        this.listing.main_repos.extend(known.clone());
        this.listing.closed_repos = known;
        if this.transcripts.scanned() {
            timeline(&format!("transcripts done ({read} files read, {} listed)", closed.len()), scanning.elapsed());
        }
        this.set_closed(closed, continued, cx);
        this.polls.transcripts_answered = asks;
        this.check_loaded(cx);
        cx.notify();
    })
    .ok()?;
    Some((back, repos))
}

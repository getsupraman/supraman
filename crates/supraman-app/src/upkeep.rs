use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use supraman_claude::Session;
use supraman_core::{Settings, find_plugin_dir, hook_exe, install_hook_exe};
use supraman_supervisor::{Absent, Bearer, HAND_ROLE, HttpMinter, Keeper, LaunchConfig, Millis, Unlisted, bearer, end_controllers, revoke_deleted_runs, runs_signed_out, sign_in_generation};

pub use supraman_core::sm_allowed_tools;

/// Origins signed out of since the last tick, each with its home and sign-in generation: the runs minted there before it
/// are marked at the home's next.
static SIGNED_OUT: Mutex<Vec<(PathBuf, String, u64)>> = Mutex::new(Vec::new());

pub fn signed_out(home: &Path, origin: &str) {
    SIGNED_OUT.lock().unwrap_or_else(|e| e.into_inner()).push((home.to_path_buf(), origin.to_string(), sign_in_generation()));
}

/// Signs out of `auth`'s configured server, whose grant revoke takes every session token minted there; the hands
/// and scheduled runs minted there are marked signed out for the next sign-in. What failed.
pub fn sign_out(auth: &supraman_workman::Auth, hands: Option<Arc<Mutex<crate::hand::Hands>>>) -> Vec<String> {
    let origin = match auth.configured_origin() {
        Ok(origin) => origin,
        Err(e) => return vec![e.to_string()],
    };
    let mut notices: Vec<String> = auth.sign_out(&origin).err().into_iter().collect();
    signed_out(auth.home(), &origin);
    if let Some(hands) = hands {
        notices.extend(hands.lock().unwrap_or_else(|e| e.into_inner()).signed_out(&origin));
    }
    notices
}

pub struct Setup {
    pub plugin_dir: Option<PathBuf>,
    pub notices: Vec<String>,
}

pub fn setup(home: &Path) -> Setup {
    let mut notices: Vec<String> = Vec::new();
    let sm = match supraman_core::install::sm_exe() {
        Ok(sm) => sm,
        Err(e) => {
            notices.push(format!("Cannot locate supraman.exe ({e}); sessions cannot be started from this build."));
            return Setup { plugin_dir: None, notices };
        }
    };
    let plugin_dir = match find_plugin_dir(&sm) {
        Ok(dir) => dir,
        Err(e) => {
            notices.push(format!("{e}: sessions cannot be started from this build."));
            return Setup { plugin_dir: None, notices };
        }
    };
    if sm.is_file() {
        if let Err(e) = install_hook_exe(&plugin_dir, &sm) {
            notices.push(format!("Cannot install {} into {}: {e}", sm.display(), plugin_dir.display()));
        }
    } else {
        notices.push(format!(
            "{} is missing (build it with cargo build -p sm): started sessions get no hook signals and no role injection.",
            sm.display()
        ));
    }
    if let Err(e) = write_role_template(home, &plugin_dir, HAND_ROLE) {
        notices.push(format!("Cannot write the {HAND_ROLE} role into {}: {e:#}", home.display()));
    }
    Setup {
        plugin_dir: Some(plugin_dir),
        notices,
    }
}

/// `{sm}` (the plugin's `sm.exe`) and `{home}` filled in, forward slashes: a role's commands may run under Git Bash.
pub fn write_role_template(home: &Path, plugin_dir: &Path, role: &str) -> anyhow::Result<()> {
    let file = format!("{role}.md");
    let forward = |p: &Path| p.display().to_string().replace('\\', "/");
    let template = std::fs::read_to_string(plugin_dir.join("roles").join(&file))?
        .replace("{sm}", &forward(&plugin_dir.join("bin").join(hook_exe())))
        .replace("{home}", &forward(home));
    let dir = home.join("roles");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(file);
    if std::fs::read_to_string(&target).ok().as_deref() != Some(template.as_str()) {
        std::fs::write(&target, template)?;
    }
    Ok(())
}

pub fn set_max_working(home: &Path, max_working: usize) -> anyhow::Result<()> {
    Settings::update(home, |settings| settings.sessions.max_working = max_working)?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub max_working: usize,
    pub terminal: supraman_core::TerminalSettings,
    pub errors: Vec<String>,
    /// Why no `claude --bg` from here can reach the background service.
    pub daemon_hold: Option<String>,
}

pub const DAEMON_CHECK_EVERY: Millis = 15_000;

/// [`supraman_claude::daemon_mismatch`] read at most every [`DAEMON_CHECK_EVERY`]: what a [+] tab's launch and a resume say while held.
pub struct DaemonWatch {
    check: fn() -> Option<String>,
    checked_at: Option<Millis>,
    why: Option<String>,
}

impl DaemonWatch {
    pub fn new(check: fn() -> Option<String>) -> Self {
        Self { check, checked_at: None, why: None }
    }

    pub fn current(&mut self, now: Millis) -> Option<String> {
        if self.checked_at.is_none_or(|t| now - t >= DAEMON_CHECK_EVERY) {
            self.why = (self.check)();
            self.checked_at = Some(now);
        }
        self.why.clone()
    }
}

/// The poll's background work on this home's own sessions: settings, retention, sign-outs, deleted runs and hands' configs.
pub struct Upkeep {
    home: PathBuf,
    pub daemon: DaemonWatch,
    daemon_logged: Option<String>,
    pruned_at: Option<Millis>,
    /// Whether the records a home kept for its workspace controllers were ended ([`end_controllers`]), once per start.
    controllers_ended: bool,
    /// The scheduled runs' claude sessions missing from the snapshots.
    runs_absent: Absent,
    /// The [+] tabs' claude sessions missing from the snapshots.
    hands_absent: Absent,
    unlisted: Unlisted,
    hands: Option<Arc<Mutex<crate::hand::Hands>>>,
    auth: Arc<supraman_workman::Auth>,
}

pub const PRUNE_EVERY: Millis = 60 * 60 * 1000;

pub fn plugin_dirs(plugin_dir: Option<&Path>, settings: &Settings) -> Vec<PathBuf> {
    plugin_dir.map(Path::to_path_buf).into_iter().chain(settings.sessions.extra_plugin_dirs.iter().cloned()).collect()
}

/// The launch pieces of this home's [+] sessions.
pub fn launch_config(home: Option<PathBuf>, plugin_dir: Option<&Path>, settings: &Settings) -> LaunchConfig {
    LaunchConfig {
        mcp_dir: home.as_deref().map(|h| h.join("mcp")).unwrap_or_default(),
        plugin_dirs: plugin_dirs(plugin_dir, settings),
        supraman_home: home,
        bg_isolation_none: true,
        allowed_tools: plugin_dir.map(sm_allowed_tools).unwrap_or_default(),
        claude_dir: supraman_core::paths::claude_dir(),
    }
}

impl Upkeep {
    /// A sign-out since the last tick marks the scheduled runs minted there, only while no sign-in followed: `sm` mints
    /// them, so their generation is unknown here.
    fn end_signed_out(&mut self) -> Vec<String> {
        let keeper = Keeper::open(&self.home);
        let mut notices = Vec::new();
        let queued: Vec<(PathBuf, String, u64)> = {
            let mut all = SIGNED_OUT.lock().unwrap_or_else(|e| e.into_inner());
            let (here, elsewhere) = std::mem::take(&mut *all).into_iter().partition(|(home, _, _)| *home == self.home);
            *all = elsewhere;
            here
        };
        for (_, origin, generation) in queued {
            if sign_in_generation() == generation {
                notices.extend(runs_signed_out(&keeper, &origin));
            }
        }
        notices
    }

    /// Revokes deleted scheduled runs, drops the runs and hands left unlisted until their tokens lapsed, and deletes token-less configs
    /// whose session is gone, none without the hands; `observed` was read without error.
    fn end_deleted(&mut self, observed: &[Session], now: Millis) -> Vec<String> {
        let keeper = Keeper::open(&self.home).with_claude_dir(supraman_core::paths::claude_dir());
        let auth = self.auth.clone();
        let mut failed = revoke_deleted_runs(&keeper, &HttpMinter::default(), || Bearer::of(bearer(&auth)), &mut self.runs_absent, observed, now);
        if let Some(hands) = &self.hands {
            failed.extend(crate::hand::end_lapsed(hands, &mut self.hands_absent, observed, now));
            let held = hands.lock().unwrap_or_else(|e| e.into_inner()).held();
            failed.extend(keeper.sweep_unminted(&mut self.unlisted, observed, &held, now));
        }
        failed
    }

    fn end_controllers_once(&mut self) -> Vec<String> {
        if std::mem::replace(&mut self.controllers_ended, true) {
            return Vec::new();
        }
        let auth = self.auth.clone();
        end_controllers(&Keeper::open(&self.home), &HttpMinter::default(), || Bearer::of(bearer(&auth)))
    }

    #[cfg(test)]
    fn with_auth(self, auth: Arc<supraman_workman::Auth>) -> Self {
        Self { auth, ..self }
    }

    /// The [+] tabs whose token-less configs [`Self::end_deleted`] leaves alone.
    pub fn with_hands(self, hands: Option<Arc<Mutex<crate::hand::Hands>>>) -> Self {
        Self { hands, ..self }
    }

    pub fn new(home: PathBuf) -> Self {
        let auth = crate::signin::auth(&home);
        Self {
            home,
            daemon: DaemonWatch::new(supraman_claude::daemon_mismatch),
            daemon_logged: None,
            pruned_at: None,
            controllers_ended: false,
            runs_absent: Absent::default(),
            hands_absent: Absent::default(),
            unlisted: Unlisted::default(),
            hands: None,
            auth,
        }
    }

    pub fn tick(&mut self, observed: &[Session], now: Millis) -> Report {
        let mut report = Report::default();
        self.read_settings(&mut report, now);
        for ended in self.end_controllers_once().into_iter().chain(self.end_signed_out()).chain(self.end_deleted(observed, now)) {
            log::warn!("{ended}");
        }
        report.daemon_hold = self.daemon.current(now);
        if self.daemon_logged != report.daemon_hold {
            match &report.daemon_hold {
                Some(why) => log::warn!("[+] launches and resumes held: {why}"),
                None => log::info!("[+] launches and resumes no longer held: the background service is reachable"),
            }
            self.daemon_logged = report.daemon_hold.clone();
        }
        report
    }

    fn read_settings(&mut self, report: &mut Report, now: Millis) {
        let settings = Settings::load(&self.home).unwrap_or_else(|e| {
            report.errors.push(format!("Cannot read the settings, using the defaults: {e}"));
            Settings::default()
        });
        report.max_working = settings.sessions.max_working;
        report.terminal = settings.terminal;
        if self.pruned_at.is_none_or(|at| now - at >= PRUNE_EVERY) {
            self.pruned_at = Some(now);
            for failed in supraman_core::prune::prune(&self.home, supraman_core::time::now_ms(), supraman_core::prune::KEEP_MS) {
                log::warn!("{failed}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn plugin_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/supraman")
    }

    #[test]
    fn a_sign_out_whose_revoke_is_refused_adds_no_notice() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = supraman_workman::fake::FakeWorkman::start(vec![]);
        let oauth = fake.enable_oauth();
        let auth = crate::test_auth::signed_in(&home, fake.base());
        oauth.fail_revoke(true);
        assert_eq!(sign_out(&auth, None), Vec::<String>::new());
        assert_eq!(oauth.revoked(), 0, "the fake refused the revoke");
        assert_eq!(auth.status(fake.base()), supraman_workman::AuthStatus::NotSignedIn);
    }

    #[test]
    fn finds_the_plugin_above_the_exe() {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/supraman.exe");
        let found = find_plugin_dir(&exe).unwrap();
        assert!(found.join("roles").join("hand.md").is_file(), "{}", found.display());
        assert!(find_plugin_dir(Path::new(r"C:\nowhere\supraman.exe")).is_err());
    }

    #[test]
    fn launches_get_extra_plugin_dirs_after_supramans_and_the_sm_rules() {
        let mut settings = Settings::default();
        settings.sessions.extra_plugin_dirs = vec![r"C:\skills\plugins\rlarch".into()];
        let own = Path::new(r"C:\fm\plugins\supraman");
        assert_eq!(plugin_dirs(Some(own), &settings), [own.to_path_buf(), r"C:\skills\plugins\rlarch".into()]);
        assert_eq!(plugin_dirs(None, &Settings::default()), Vec::<PathBuf>::new());
        assert_eq!(
            sm_allowed_tools(own),
            ["Bash(sm:*)", "Bash(C:/fm/plugins/supraman/bin/sm.exe:*)", "mcp__agents__agent_launch", "mcp__agents__agent_list", "mcp__agents__agent_stop"]
        );
    }

    #[test]
    fn the_hand_role_is_written_with_its_paths_and_injected_whole() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), HAND_ROLE).unwrap();
        let injected = supraman_core::role_injection(home.path(), HAND_ROLE, Some("Alpha")).unwrap();
        assert!(injected.contains("/supraman:supraman-scheduler"), "{injected}");
        assert!(!injected.contains('{'), "every placeholder filled: {injected}");
    }

    static CHECKS: AtomicUsize = AtomicUsize::new(0);

    fn counted_mismatch() -> Option<String> {
        CHECKS.fetch_add(1, Ordering::SeqCst);
        Some("runs as administrator".into())
    }

    #[test]
    fn an_unreachable_service_is_reported_and_checked_at_most_every_interval() {
        let home = tempfile::tempdir().unwrap();
        set_max_working(home.path(), 1).unwrap();
        let mut s = Upkeep::new(home.path().to_path_buf());
        s.daemon = DaemonWatch::new(counted_mismatch);
        let t0 = 1_800_000_000_000;
        let report = s.tick(&[], t0);
        assert_eq!((report.max_working, report.daemon_hold.as_deref()), (1, Some("runs as administrator")));
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1);
        s.tick(&[], t0 + 2_000);
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1, "not every tick");
        s.tick(&[], t0 + DAEMON_CHECK_EVERY);
        assert_eq!(CHECKS.load(Ordering::SeqCst), 2);
    }

    /// The names `kept.json` under `home` marks signed out.
    fn signed_out_marks(home: &Path) -> Vec<String> {
        let text = std::fs::read_to_string(home.join("supervisor").join("kept.json")).unwrap_or_default();
        let table: BTreeMap<String, serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
        table.into_iter().filter(|(_, k)| k["signedOut"] == true).map(|(n, _)| n).collect()
    }

    fn run_record(home: &Path, name: &str, origin: &str) {
        let session = supraman_supervisor::MintedSession { session_id: "r0".into(), role: "repo".into(), claude_session_id: None, origin: Some(origin.into()) };
        supraman_supervisor::MintedSessions::edit_run(&supraman_supervisor::MintedSessions::run_path(home, name), |r| r.insert(name, session)).unwrap();
    }

    #[test]
    fn a_sign_out_marks_the_runs_minted_there_unless_a_sign_in_followed_before_the_tick() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = supraman_workman::fake::FakeWorkman::start(vec![]);
        fake.enable_oauth();
        let auth = crate::test_auth::signed_in(&home, fake.base());
        let run = "fm-demo-sched-nightly-20261113-0900";
        run_record(&home, run, fake.base());
        let mut s = Upkeep::new(home.clone()).with_auth(auth);
        s.daemon = DaemonWatch::new(|| None);

        signed_out(&home, fake.base());
        supraman_supervisor::signing_in();
        s.tick(&[], supraman_claude::now_ms());
        assert_eq!(signed_out_marks(&home), Vec::<String>::new(), "a sign-in after it: sm's mint may be the new grant's");

        signed_out(&home, fake.base());
        s.tick(&[], supraman_claude::now_ms());
        assert_eq!(signed_out_marks(&home), [run], "marked for a fresh token after the next sign-in");
    }

    #[test]
    fn the_first_tick_ends_a_left_controller_record_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = supraman_workman::fake::FakeWorkman::start(vec![]);
        fake.enable_oauth();
        let sessions = fake.enable_sessions();
        let auth = crate::test_auth::signed_in(&home, fake.base());
        let (url, token) = bearer(&auth).unwrap().unwrap();
        let request = supraman_supervisor::MintRequest {
            name: "fm-demo-workspace".into(),
            role: "controller".into(),
            launched_by: "supraman/demo".into(),
            origin: "supraman/demo".into(),
            cwd: r"C:\demo".into(),
            external_role: "workspace".into(),
            replace: false,
            repo: None,
            claude_session_id: None,
            takeover_from: None,
        };
        let minted = supraman_supervisor::Minter::mint(&HttpMinter::default(), &supraman_core::url::origin(&url), &token, &request).unwrap();
        let path = home.join("supervisor").join("demo.sessions.json");
        let mut records = supraman_supervisor::MintedSessions::empty(path.clone());
        let session = supraman_supervisor::MintedSession { session_id: minted.session_id, role: "workspace".into(), claude_session_id: None, origin: Some(fake.base().into()) };
        records.insert(&request.name, session);
        records.save().unwrap();
        assert!(sessions.token_of("fm-demo-workspace").is_some());

        let mut s = Upkeep::new(home.clone()).with_auth(auth);
        s.daemon = DaemonWatch::new(|| None);
        s.tick(&[], supraman_claude::now_ms());
        assert_eq!(sessions.token_of("fm-demo-workspace"), None, "revoked at the first tick");
        assert!(!path.exists(), "its record goes");
        let deletes = || fake.seen().iter().filter(|r| r.method == "DELETE").count();
        let once = deletes();
        records.save().unwrap();
        s.tick(&[], supraman_claude::now_ms());
        assert_eq!(deletes(), once, "once per start: a later tick leaves the home's files alone");
    }
}

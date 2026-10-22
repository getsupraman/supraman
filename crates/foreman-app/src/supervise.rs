//! The app's supervisor: one [`foreman_supervisor::Supervisor`] per platform that has a desired-state
//! file, ticked after every `claude agents --json --all` poll on that snapshot, all platforms with one
//! shared budget ([`tick_all`]). Runs on the refresh loop's background thread; the UI only writes
//! files (desired state, settings) and queues Retries.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, anyhow};
use foreman_claude::{Claude, Session};
use foreman_core::{HOOK_EXE, Platform, Settings, install_hook_exe};
use foreman_supervisor::{
    DesiredState, LaunchConfig, Millis, NeedsYou, PLATFORM_ROLE, Supervisor, platform_session_name, role_prompt,
    tick_all,
};

use crate::tokens::AppTokens;

/// Off: no tick acts. The selftest's cleanup turns it off, then takes [`TICK`] to wait out a tick
/// in flight, so nothing it removes is resumed behind its back.
pub static ENABLED: AtomicBool = AtomicBool::new(true);
pub static TICK: Mutex<()> = Mutex::new(());

pub fn disable_and_wait() {
    ENABLED.store(false, Ordering::SeqCst);
    drop(TICK.lock().unwrap_or_else(|e| e.into_inner()));
}

/// What the app found at start: where the plugin is and whether its hook exe is current.
pub struct Setup {
    pub plugin_dir: Option<PathBuf>,
    pub notices: Vec<String>,
}

/// `<repo>/plugins/foreman`, found by walking up from the exe (`<repo>/target/<profile>/foreman.exe`).
pub fn find_plugin_dir(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .map(|dir| dir.join("plugins").join("foreman"))
        .find(|dir| dir.join("hooks").join("hooks.json").is_file())
}

/// `fm plugin install` for the `fm.exe` built next to `foreman.exe`, and the platform role template at
/// `<home>/roles/platform.md` (the SessionStart hook fills in `{platform}` from `FOREMAN_PLATFORM`).
pub fn setup(home: &Path) -> Setup {
    let mut notices = Vec::new();
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            return Setup {
                plugin_dir: None,
                notices: vec![format!("Cannot locate foreman.exe ({e}); Start is unavailable.")],
            };
        }
    };
    let Some(plugin_dir) = find_plugin_dir(&exe) else {
        notices.push(format!(
            "No plugins/foreman above {}: sessions cannot be started from this build.",
            exe.display()
        ));
        return Setup { plugin_dir: None, notices };
    };
    let fm = exe.with_file_name(HOOK_EXE);
    if fm.is_file() {
        if let Err(e) = install_hook_exe(&plugin_dir, &fm) {
            notices.push(format!("Cannot install {} into {}: {e}", fm.display(), plugin_dir.display()));
        }
    } else {
        notices.push(format!(
            "{} is missing (build it with cargo build -p fm): started sessions get no hook signals and no role re-injection.",
            fm.display()
        ));
    }
    if let Err(e) = write_role_template(home, &plugin_dir) {
        notices.push(format!("Cannot write the platform role into {}: {e:#}", home.display()));
    }
    Setup {
        plugin_dir: Some(plugin_dir),
        notices,
    }
}

fn write_role_template(home: &Path, plugin_dir: &Path) -> anyhow::Result<()> {
    let file = format!("{PLATFORM_ROLE}.md");
    let template = std::fs::read_to_string(plugin_dir.join("roles").join(&file))?;
    let dir = home.join("roles");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(file);
    if std::fs::read_to_string(&target).ok().as_deref() != Some(template.as_str()) {
        std::fs::write(&target, template)?;
    }
    Ok(())
}

/// Start: the platform's desired state with its `fm-<platform>-platform` session (cwd = the platform
/// root, prompt = the platform role), `started = true`, saved. Other wanted sessions are kept.
pub fn start_platform(home: &Path, plugin_dir: Option<&Path>, platform: &Platform) -> anyhow::Result<DesiredState> {
    let plugin_dir = plugin_dir.ok_or_else(|| anyhow!("no plugins/foreman found for this build"))?;
    let prompt = role_prompt(plugin_dir, PLATFORM_ROLE, &platform.name)?;
    let name = platform_session_name(&platform.name);
    let mut desired = DesiredState::load(home, &platform.name)?
        .unwrap_or_else(|| DesiredState::platform_controller(&platform.name, &platform.root, &prompt));
    match desired.sessions.iter_mut().find(|w| w.name == name) {
        Some(w) => {
            w.cwd = platform.root.clone();
            w.prompt = prompt;
        }
        None => {
            let controller = DesiredState::platform_controller(&platform.name, &platform.root, &prompt);
            desired.sessions.insert(0, controller.sessions.into_iter().next().context("no controller session")?);
        }
    }
    desired.started = true;
    desired.save(home)?;
    Ok(desired)
}

/// Stop: `started = false`, saved. The next tick stops the live wanted sessions (never removes them).
pub fn stop_platform(home: &Path, platform: &str) -> anyhow::Result<()> {
    if let Some(mut desired) = DesiredState::load(home, platform)? {
        desired.started = false;
        desired.save(home)?;
    }
    Ok(())
}

pub fn set_max_working(home: &Path, max_working: usize) -> anyhow::Result<()> {
    let mut settings = Settings::load(home)?;
    settings.max_working = max_working;
    settings.save(home)?;
    Ok(())
}

/// One tick's outcome, for the UI.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub needs: Vec<NeedsYou>,
    /// Per platform with a desired-state file: started, and the wanted session names.
    pub desired: BTreeMap<String, (bool, Vec<String>)>,
    pub max_working: usize,
    pub errors: Vec<String>,
    /// Per wanted name, the supervisor's last launch/resume attempt.
    pub last_attempt: HashMap<String, Millis>,
}

pub struct Supervision {
    home: PathBuf,
    config: LaunchConfig,
    supervisors: BTreeMap<String, Supervisor<Claude, AppTokens>>,
}

impl Supervision {
    pub fn new(home: PathBuf, plugin_dir: Option<PathBuf>) -> Self {
        let config = LaunchConfig {
            mcp_dir: home.join("mcp"),
            plugin_dirs: plugin_dir.into_iter().collect(),
            foreman_home: Some(home.clone()),
            bg_isolation_none: true,
        };
        Self {
            home,
            config,
            supervisors: BTreeMap::new(),
        }
    }

    /// Re-reads settings and every registered platform's desired state, applies the queued Retries
    /// (`(platform, name)`), then ticks every supervisor on `observed`.
    pub fn tick(&mut self, platforms: &[Platform], observed: &[Session], retries: Vec<(String, String)>, now: Millis) -> Report {
        let mut report = Report::default();
        let settings = Settings::load(&self.home).unwrap_or_else(|e| {
            report.errors.push(format!("Cannot read the settings, using the defaults: {e}"));
            Settings::default()
        });
        report.max_working = settings.max_working;
        self.supervisors.retain(|name, _| platforms.iter().any(|p| &p.name == name));
        for p in platforms {
            match DesiredState::load(&self.home, &p.name) {
                Ok(Some(desired)) => match self.supervisors.get_mut(&p.name) {
                    Some(s) => s.desired = desired,
                    None => {
                        let tokens = AppTokens { home: self.home.clone() };
                        let s = Supervisor::new(desired, self.config.clone(), Claude::default(), tokens);
                        self.supervisors.insert(p.name.clone(), s);
                    }
                },
                Ok(None) => {
                    self.supervisors.remove(&p.name);
                }
                Err(e) => report.errors.push(format!("Cannot read the desired state of {}: {e}", p.name)),
            }
        }
        for (platform, name) in retries {
            if let Some(s) = self.supervisors.get_mut(&platform) {
                s.retry(&name);
            }
        }
        {
            let _tick = TICK.lock().unwrap_or_else(|e| e.into_inner());
            if ENABLED.load(Ordering::SeqCst) {
                report.needs = tick_all(self.supervisors.values_mut(), observed, settings.max_working, now);
            }
        }
        for (platform, s) in &self.supervisors {
            let names: Vec<String> = s.desired.sessions.iter().map(|w| w.name.clone()).collect();
            for name in &names {
                if let Some(t) = s.memory.get(name).and_then(|a| a.last_attempt) {
                    report.last_attempt.insert(name.clone(), t);
                }
            }
            report.desired.insert(platform.clone(), (s.desired.started, names));
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/foreman")
    }

    #[test]
    fn finds_the_plugin_above_the_exe() {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/foreman.exe");
        let found = find_plugin_dir(&exe).unwrap();
        assert!(found.join("roles").join("platform.md").is_file(), "{}", found.display());
        assert_eq!(find_plugin_dir(Path::new(r"C:\nowhere\foreman.exe")), None);
    }

    #[test]
    fn start_keeps_extra_sessions_and_stop_keeps_the_file() {
        let home = tempfile::tempdir().unwrap();
        let platform = Platform::new("demo", r"C:\demo", Vec::new());
        let d = start_platform(home.path(), Some(&plugin_dir()), &platform).unwrap();
        assert!(d.started);
        assert_eq!(d.sessions[0].name, "fm-demo-platform");
        assert_eq!(d.sessions[0].cwd, PathBuf::from(r"C:\demo"));
        assert!(d.sessions[0].prompt.contains("platform controller for demo"), "{}", d.sessions[0].prompt);

        let mut extra = d.clone();
        extra.started = false;
        let mut park = extra.sessions[0].clone();
        park.name = "fm-demo-park".into();
        extra.sessions.push(park);
        extra.save(home.path()).unwrap();
        let moved = Platform::new("demo", r"C:\moved", Vec::new());
        let d = start_platform(home.path(), Some(&plugin_dir()), &moved).unwrap();
        assert_eq!(d.sessions.len(), 2);
        assert_eq!(d.sessions[0].cwd, PathBuf::from(r"C:\moved"));

        stop_platform(home.path(), "demo").unwrap();
        let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
        assert!(!d.started);
        assert_eq!(d.sessions.len(), 2);
        stop_platform(home.path(), "never-started").unwrap();
        assert!(DesiredState::load(home.path(), "never-started").unwrap().is_none());
        assert!(start_platform(home.path(), None, &platform).is_err());
    }

    #[test]
    fn tick_supervises_only_platforms_with_a_desired_file() {
        let home = tempfile::tempdir().unwrap();
        let platforms = [Platform::new("alpha", r"C:\a", Vec::new()), Platform::new("beta", r"C:\b", Vec::new())];
        DesiredState::platform_controller("beta", r"C:\b", "p").save(home.path()).unwrap();
        set_max_working(home.path(), 1).unwrap();
        let mut s = Supervision::new(home.path().to_path_buf(), None);
        // beta is not started: nothing to launch, and no `claude` is run
        let report = s.tick(&platforms, &[], Vec::new(), 1_800_000_000_000);
        assert_eq!(report.max_working, 1);
        assert_eq!(report.desired.keys().collect::<Vec<_>>(), ["beta"]);
        assert_eq!(report.desired["beta"], (false, vec!["fm-beta-platform".to_string()]));
        assert!(report.needs.is_empty() && report.errors.is_empty(), "{report:?}");
    }
}

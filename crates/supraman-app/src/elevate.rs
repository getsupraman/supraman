//! An elevated `claude --bg` starts the background service elevated, out of an unelevated Supraman's reach.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use supraman_core::Settings;

pub const AFTER_WITHIN: Duration = Duration::from_secs(60);

/// What [`relaunch`] passes: `--elevated --home <home> [--after <pid>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relaunched {
    pub home: PathBuf,
    pub after: Option<u32>,
}

pub fn parse_relaunch(args: &[&str]) -> Option<Relaunched> {
    match args {
        ["--elevated", "--home", home] => Some(Relaunched { home: home.into(), after: None }),
        ["--elevated", "--home", home, "--after", pid] => Some(Relaunched { home: home.into(), after: Some(pid.parse().ok()?) }),
        _ => None,
    }
}

/// UAC's fresh environment may lack `SUPRAMAN_HOME`; a trailing backslash would escape the closing quote.
pub fn arguments(home: &Path, after: Option<u32>) -> String {
    let home = home.display().to_string();
    let mut args = format!("--elevated --home \"{}\"", home.trim_end_matches('\\'));
    if let Some(pid) = after {
        args.push_str(&format!(" --after {pid}"));
    }
    args
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Exit,
    Run { declined: Option<String> },
}

pub fn start(app: bool, toggle: bool, elevated: bool, relaunched: bool, relaunch: impl FnOnce() -> anyhow::Result<()>) -> Start {
    if !app || !toggle || elevated || relaunched {
        return Start::Run { declined: None };
    }
    match relaunch() {
        Ok(()) => Start::Exit,
        Err(e) => Start::Run { declined: Some(format!("{e:#}")) },
    }
}

pub fn toggle(home: &Path) -> bool {
    Settings::load(home).is_ok_and(|s| s.sessions.run_as_admin)
}

pub fn relaunch(home: &Path, after: Option<u32>) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.to_path_buf());
    let args = arguments(home, after);
    supraman_core::platform::open::runas(&exe, args.as_ref(), &cwd).map_err(anyhow::Error::msg)
}

pub fn wait_for_exit(pid: u32, within: Duration) {
    let deadline = Instant::now() + within;
    while supraman_core::process::alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn the_copy_gets_its_home_quoted_and_reads_it_back() {
        let args = arguments(Path::new(r"C:\Users\o neil\Supraman"), None);
        assert_eq!(args, r#"--elevated --home "C:\Users\o neil\Supraman""#);
        assert_eq!(arguments(Path::new("C:\\fm\\"), Some(42)), r#"--elevated --home "C:\fm" --after 42"#);
        // as the C runtime splits them
        assert_eq!(
            parse_relaunch(&["--elevated", "--home", r"C:\fm", "--after", "42"]),
            Some(Relaunched { home: r"C:\fm".into(), after: Some(42) })
        );
        assert_eq!(parse_relaunch(&["--elevated", "--home", r"C:\fm"]).unwrap().after, None);
        assert_eq!(parse_relaunch(&["--home", r"C:\fm"]), None);
        assert_eq!(parse_relaunch(&["--elevated", "--home", r"C:\fm", "--after", "x"]), None);
    }

    fn decide(app: bool, toggle: bool, elevated: bool, relaunched: bool, result: anyhow::Result<()>) -> (Start, usize) {
        let calls = Cell::new(0);
        let start = start(app, toggle, elevated, relaunched, || {
            calls.set(calls.get() + 1);
            result
        });
        (start, calls.get())
    }

    #[test]
    fn only_an_unelevated_app_start_with_the_toggle_on_asks_once() {
        assert_eq!(decide(true, true, false, false, Ok(())), (Start::Exit, 1));
        let run = Start::Run { declined: None };
        assert_eq!(decide(true, false, false, false, Ok(())), (run.clone(), 0), "toggle off");
        assert_eq!(decide(true, true, true, false, Ok(())), (run.clone(), 0), "already elevated");
        assert_eq!(decide(false, true, false, false, Ok(())), (run.clone(), 0), "a selftest never elevates");
        assert_eq!(decide(true, true, false, true, Ok(())), (run, 0), "the copy never asks again: no loop");
    }

    #[test]
    fn a_declined_prompt_runs_unelevated_and_says_why() {
        let (start, calls) = decide(true, true, false, false, Err(anyhow::anyhow!("ShellExecuteW runas returned 5")));
        assert_eq!(calls, 1);
        assert_eq!(start, Start::Run { declined: Some("ShellExecuteW runas returned 5".into()) });
    }

    #[test]
    fn the_toggle_is_off_by_default_and_kept_in_the_home() {
        let home = tempfile::tempdir().unwrap();
        assert!(!toggle(home.path()));
        Settings::update(home.path(), |s| s.sessions.run_as_admin = true).unwrap();
        assert!(toggle(home.path()));
    }
}

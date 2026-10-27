//! Settings → Run Foreman as administrator (m5/25). An elevated `claude --bg` starts the background
//! service elevated, and an unelevated Foreman (any start from Explorer) cannot reach it
//! ([`foreman_claude::daemon_mismatch`]). With the setting on, an unelevated start asks UAC once for an
//! elevated copy of itself on the same home and exits; declined, it carries on unelevated.

use std::path::Path;

use foreman_core::Settings;

/// Whether this start should hand over to an elevated copy: the setting is on and it is not elevated.
pub fn wanted(home: &Path) -> bool {
    Settings::load(home).is_ok_and(|s| s.run_as_admin) && !foreman_claude::is_elevated()
}

/// What the elevated copy gets: `--home <home>`, since UAC starts it with a fresh environment that
/// may lack `FOREMAN_HOME`. Windows' command-line quoting: a trailing `\` would escape the quote.
pub fn arguments(home: &Path) -> String {
    let home = home.display().to_string();
    format!("--home \"{}\"", home.trim_end_matches('\\'))
}

/// `ShellExecuteW` with the `runas` verb: the UAC prompt. `Ok` once the elevated copy started.
#[cfg(windows)]
pub fn relaunch(home: &Path) -> anyhow::Result<()> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    let exe = std::env::current_exe()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.to_path_buf());
    let started = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            &HSTRING::from(exe.as_os_str()),
            &HSTRING::from(arguments(home)),
            &HSTRING::from(cwd.as_os_str()),
            SW_SHOWNORMAL,
        )
    };
    // > 32 is success; anything else, including a declined prompt, is an error code.
    let code = started.0 as isize;
    anyhow::ensure!(code > 32, "ShellExecuteW runas returned {code}");
    Ok(())
}

#[cfg(not(windows))]
pub fn relaunch(_home: &Path) -> anyhow::Result<()> {
    anyhow::bail!("only Windows elevates")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_elevated_copy_gets_the_home_quoted_without_a_trailing_backslash() {
        assert_eq!(arguments(Path::new(r"C:\Users\o neil\Foreman")), r#"--home "C:\Users\o neil\Foreman""#);
        assert_eq!(arguments(Path::new("C:\\fm\\")), r#"--home "C:\fm""#);
    }

    #[test]
    fn off_by_default_and_on_only_when_set() {
        let home = tempfile::tempdir().unwrap();
        assert!(!wanted(home.path()));
        Settings { run_as_admin: true, ..Settings::default() }.save(home.path()).unwrap();
        assert_eq!(wanted(home.path()), !foreman_claude::is_elevated());
    }
}

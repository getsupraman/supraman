use std::time::Duration;

use crate::{Claude, ClaudeError, Result, Session};

pub const LIST_TIMEOUT: Duration = Duration::from_secs(10);
/// The oldest claude CLI Foreman runs on (docs/RELEASE.md): every CLI fact it relies on (`--bg`, the
/// `agents --json --all` fields, `--plugin-dir`, the peer registry's `peerProtocol 1`) was measured on it.
pub const MIN_VERSION: (u32, u32, u32) = (2, 1, 308);
/// The newest claude CLI the release gate passed on. Newer ones run; a newer minor or major warns.
pub const TESTED_VERSION: (u32, u32, u32) = (2, 1, 310);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// The whole first line, e.g. `2.1.308 (Claude Code)`.
    pub raw: String,
}

impl ClaudeVersion {
    fn triple(&self) -> (u32, u32, u32) {
        (self.major, self.minor, self.patch)
    }

    /// A warning for a CLI whose major.minor is newer than [`TESTED_VERSION`]'s; a newer patch of the
    /// tested minor is not worth one. Everything still runs either way.
    pub fn untested(&self) -> Option<String> {
        let (major, minor, _) = TESTED_VERSION;
        ((self.major, self.minor) > (major, minor)).then(|| {
            format!(
                "claude CLI {} is newer than {major}.{minor}, the newest Supraman was tested on ({}): everything stays enabled; if something misbehaves, report it with this version.",
                self.raw,
                dotted(TESTED_VERSION)
            )
        })
    }
}

fn dotted((major, minor, patch): (u32, u32, u32)) -> String {
    format!("{major}.{minor}.{patch}")
}

/// One `claude agents --json --all` process: interactive and background, live and stopped.
pub fn list_sessions() -> Result<Vec<Session>> {
    Claude::default().list_sessions_within(LIST_TIMEOUT)
}

pub fn cli_version() -> Result<ClaudeVersion> {
    parse_version(&Claude::default().run_within(&["--version"], LIST_TIMEOUT)?)
}

pub fn check_supported() -> Result<ClaudeVersion> {
    let version = cli_version()?;
    check_version(&version)?;
    Ok(version)
}

/// `"2.1.308 (Claude Code)"` → 2.1.308.
pub fn parse_version(text: &str) -> Result<ClaudeVersion> {
    let bad = || ClaudeError::BadVersion(text.trim().to_string());
    let line = text.lines().next().unwrap_or_default().trim();
    let mut parts = line.split_whitespace().next().ok_or_else(bad)?.split('.');
    let mut next = || parts.next().and_then(|p| p.parse::<u32>().ok()).ok_or_else(bad);
    Ok(ClaudeVersion {
        major: next()?,
        minor: next()?,
        patch: next()?,
        raw: line.to_string(),
    })
}

/// Refuses only a CLI older than [`MIN_VERSION`]; a newer one than tested passes ([`ClaudeVersion::untested`]).
pub fn check_version(version: &ClaudeVersion) -> Result<()> {
    if version.triple() >= MIN_VERSION {
        Ok(())
    } else {
        Err(ClaudeError::Unsupported {
            found: version.raw.clone(),
            minimum: dotted(MIN_VERSION),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_measured_version_line() {
        let v = parse_version("2.1.308 (Claude Code)\n").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (2, 1, 308));
        assert_eq!(v.raw, "2.1.308 (Claude Code)");
        assert!(check_version(&v).is_ok());
    }

    #[test]
    fn below_the_minimum_is_refused_with_the_reason() {
        for text in ["2.1.307 (Claude Code)", "2.0.999", "1.9.9"] {
            let v = parse_version(text).unwrap();
            let err = check_version(&v).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "claude CLI {} is too old: Supraman needs 2.1.308 or newer (`--bg`, `agents --json --all`, `--plugin-dir` and peerProtocol 1 as measured there); update claude",
                    text
                )
            );
        }
    }

    #[test]
    fn the_tested_range_is_fine_without_a_warning() {
        for text in ["2.1.308 (Claude Code)", "2.1.309", "2.1.310 (Claude Code)"] {
            let v = parse_version(text).unwrap();
            assert!(check_version(&v).is_ok(), "{text}");
            assert_eq!(v.untested(), None, "{text}");
        }
    }

    #[test]
    fn a_newer_patch_runs_without_a_warning() {
        for text in ["2.1.311 (Claude Code)", "2.1.999"] {
            let v = parse_version(text).unwrap();
            assert!(check_version(&v).is_ok(), "never block a newer claude: {text}");
            assert_eq!(v.untested(), None, "{text}");
        }
    }

    #[test]
    fn a_newer_minor_or_major_runs_with_a_warning() {
        for text in ["2.2.0", "2.3.1 (Claude Code)", "3.0.0 (Claude Code)"] {
            let v = parse_version(text).unwrap();
            assert!(check_version(&v).is_ok(), "never block a newer claude: {text}");
            let warning = v.untested().expect(text);
            assert!(warning.contains(text) && warning.contains("newer than 2.1, the newest Supraman was tested on (2.1.310)"), "{warning}");
        }
    }

    #[test]
    fn garbage_is_a_bad_version() {
        for text in ["", "Claude Code", "2.1", "2.x.3 (Claude Code)"] {
            assert!(matches!(parse_version(text), Err(ClaudeError::BadVersion(_))), "{text:?}");
        }
    }
}

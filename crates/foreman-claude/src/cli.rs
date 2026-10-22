use std::io::{ErrorKind, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{ClaudeError, Result, Session, parse};

pub const LIST_TIMEOUT: Duration = Duration::from_secs(10);
pub const SUPPORTED_MAJOR: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// The whole first line, e.g. `2.1.308 (Claude Code)`.
    pub raw: String,
}

/// One `claude agents --json --all` process: interactive and background, live and stopped.
pub fn list_sessions() -> Result<Vec<Session>> {
    parse(&run(&["agents", "--json", "--all"], LIST_TIMEOUT)?)
}

pub fn cli_version() -> Result<ClaudeVersion> {
    parse_version(&run(&["--version"], LIST_TIMEOUT)?)
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

pub fn check_version(version: &ClaudeVersion) -> Result<()> {
    if version.major == SUPPORTED_MAJOR {
        Ok(())
    } else {
        Err(ClaudeError::Unsupported {
            found: version.raw.clone(),
            supported: SUPPORTED_MAJOR,
        })
    }
}

fn run(args: &[&str], timeout: Duration) -> Result<String> {
    let command = format!("claude {}", args.join(" "));
    let mut child = Command::new("claude")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| match source.kind() {
            ErrorKind::NotFound => ClaudeError::NotInstalled,
            _ => ClaudeError::Spawn {
                command: command.clone(),
                source,
            },
        })?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ClaudeError::Timeout {
                    command,
                    after: timeout,
                });
            }
            Err(source) => return Err(ClaudeError::Spawn { command, source }),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    if !status.success() {
        return Err(ClaudeError::Failed {
            command,
            status: status.to_string(),
            stderr: stderr.join().unwrap_or_default().trim().to_string(),
        });
    }
    Ok(stdout)
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
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
    fn other_majors_are_refused_clearly() {
        let v = parse_version("3.0.0 (Claude Code)").unwrap();
        let err = check_version(&v).unwrap_err();
        assert_eq!(
            err.to_string(),
            "claude CLI 3.0.0 (Claude Code) is not supported: Foreman needs major version 2"
        );
        assert!(check_version(&parse_version("1.9.9").unwrap()).is_err());
    }

    #[test]
    fn garbage_is_a_bad_version() {
        for text in ["", "Claude Code", "2.1", "2.x.3 (Claude Code)"] {
            assert!(matches!(parse_version(text), Err(ClaudeError::BadVersion(_))), "{text:?}");
        }
    }
}

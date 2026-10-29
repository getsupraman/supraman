use std::time::Duration;

pub type Result<T> = std::result::Result<T, ClaudeError>;

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("the `claude` CLI is not on PATH")]
    NotInstalled,
    #[error("cannot run `{command}`: {source}")]
    Spawn {
        command: String,
        source: std::io::Error,
    },
    #[error("`{command}` did not finish within {}s and was killed", .after.as_secs())]
    Timeout { command: String, after: Duration },
    #[error("`claude agents` output is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("`claude agents` output is not a JSON array")]
    NotAnArray,
    #[error("`claude agents` row {row}: missing required field `{field}`")]
    MissingField { field: &'static str, row: String },
    #[error("`claude agents` row {row}: field `{field}` is not {expected}")]
    InvalidField {
        field: &'static str,
        row: String,
        expected: &'static str,
    },
    #[error("cannot read a version from `claude --version` output {0:?}")]
    BadVersion(String),
    #[error(
        "claude CLI {found} is too old: Foreman needs {minimum} or newer (`--bg`, `agents --json --all`, `--plugin-dir` and peerProtocol 1 as measured there); update claude"
    )]
    Unsupported { found: String, minimum: String },
    #[error("workspace {} is not trusted: run `claude` there once and accept the trust prompt", .cwd.display())]
    TrustNeeded { cwd: std::path::PathBuf },
    #[error("a session named {name} already exists")]
    NameTaken { name: String },
    #[error("claude failed (exit {}): {stderr}", .code.map_or("none".to_string(), |c| c.to_string()))]
    CliFailed { code: Option<i32>, stderr: String },
    /// `--bg` could not connect to the background service ([`crate::UNREACHABLE`], m5/25).
    #[error("claude failed (exit {}): {stderr}", .code.map_or("none".to_string(), |c| c.to_string()))]
    DaemonUnreachable { code: Option<i32>, stderr: String },
}

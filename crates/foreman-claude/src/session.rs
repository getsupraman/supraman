use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::{ClaudeError, Result};

/// A `working`/`blocked` background row without a pid older than this is [`Health::Orphaned`].
pub const ORPHAN_AFTER: Duration = Duration::from_secs(60);

/// One row of `claude agents --json --all`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// 8 hex; `sessionId[..8]` when the row has no `id` (interactive rows never do).
    pub id: String,
    pub session_id: String,
    /// `""` when the row has none.
    pub name: String,
    pub cwd: PathBuf,
    pub kind: Kind,
    pub state: Option<SessionState>,
    /// Present only while the process is alive.
    pub status: Option<Status>,
    /// Present only while parked, e.g. `"permission prompt"`.
    pub waiting_for: Option<String>,
    pub pid: Option<u32>,
    /// Epoch milliseconds.
    pub started_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Background,
    Interactive,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Working,
    Blocked,
    Stopped,
    Done,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Busy,
    Waiting,
    Idle,
    Other(String),
}

/// See the derivation table in the crate docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Running,
    Blocked { waiting_for: Option<String> },
    Idle,
    Stopped,
    Orphaned { last_state: SessionState },
    Unknown,
}

impl Kind {
    fn from_wire(s: &str) -> Self {
        match s {
            "background" => Self::Background,
            "interactive" => Self::Interactive,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Background => "background",
            Self::Interactive => "interactive",
            Self::Other(s) => s,
        }
    }
}

impl SessionState {
    fn from_wire(s: &str) -> Self {
        match s {
            "working" => Self::Working,
            "blocked" => Self::Blocked,
            "stopped" => Self::Stopped,
            "done" => Self::Done,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Stopped => "stopped",
            Self::Done => "done",
            Self::Other(s) => s,
        }
    }
}

impl Status {
    fn from_wire(s: &str) -> Self {
        match s {
            "busy" => Self::Busy,
            "waiting" => Self::Waiting,
            "idle" => Self::Idle,
            other => Self::Other(other.to_string()),
        }
    }
}

impl fmt::Display for Health {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => f.write_str("running"),
            Self::Blocked { waiting_for: Some(w) } => write!(f, "blocked ({w})"),
            Self::Blocked { waiting_for: None } => f.write_str("blocked"),
            Self::Idle => f.write_str("idle"),
            Self::Stopped => f.write_str("stopped"),
            Self::Orphaned { last_state } => write!(f, "orphaned (was {})", last_state.as_str()),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Session {
    pub fn health(&self, now_ms: i64) -> Health {
        use SessionState as S;
        let state = self.state.as_ref();
        if matches!(state, Some(S::Stopped | S::Done)) {
            return Health::Stopped;
        }
        if let Some(last @ (S::Working | S::Blocked)) = state
            && self.kind == Kind::Background
            && self.pid.is_none()
            && self.started_at.is_none_or(|t| now_ms - t > ORPHAN_AFTER.as_millis() as i64)
        {
            return Health::Orphaned {
                last_state: last.clone(),
            };
        }
        if matches!(state, Some(S::Blocked))
            || matches!(self.status, Some(Status::Waiting))
            || self.waiting_for.is_some()
        {
            return Health::Blocked {
                waiting_for: self.waiting_for.clone(),
            };
        }
        match (&self.status, state) {
            (Some(Status::Busy), _) => Health::Running,
            (Some(Status::Idle), _) => Health::Idle,
            (None, Some(S::Working)) => Health::Running,
            _ => Health::Unknown,
        }
    }
}

/// Parses the stdout of `claude agents --json --all`. Pure: never runs the CLI.
pub fn parse(json: &str) -> Result<Vec<Session>> {
    let value: Value = serde_json::from_str(json)?;
    let rows = value.as_array().ok_or(ClaudeError::NotAnArray)?;
    rows.iter().enumerate().map(|(i, row)| parse_row(i, row)).collect()
}

fn parse_row(index: usize, row: &Value) -> Result<Session> {
    let Some(obj) = row.as_object() else {
        return Err(ClaudeError::InvalidField {
            field: "<row>",
            row: format!("#{index}"),
            expected: "an object",
        });
    };
    let label = obj
        .get("id")
        .or_else(|| obj.get("sessionId"))
        .and_then(Value::as_str)
        .map_or_else(|| format!("#{index}"), str::to_string);
    let r = Row { obj, label };

    let session_id = r.required_str("sessionId")?;
    let cwd = r.required_str("cwd")?;
    let kind = r.required_str("kind")?;
    let id = match r.optional_str("id")? {
        Some(id) => id,
        None => session_id.chars().take(8).collect(),
    };
    Ok(Session {
        id,
        name: r.optional_str("name")?.unwrap_or_default(),
        cwd: PathBuf::from(cwd),
        kind: Kind::from_wire(&kind),
        state: r.optional_str("state")?.as_deref().map(SessionState::from_wire),
        status: r.optional_str("status")?.as_deref().map(Status::from_wire),
        waiting_for: r.optional_str("waitingFor")?,
        pid: r
            .optional_int("pid")?
            .map(|p| u32::try_from(p).map_err(|_| r.invalid("pid", "a process id")))
            .transpose()?,
        started_at: r.optional_int("startedAt")?,
        session_id,
    })
}

struct Row<'a> {
    obj: &'a Map<String, Value>,
    label: String,
}

impl Row<'_> {
    fn invalid(&self, field: &'static str, expected: &'static str) -> ClaudeError {
        ClaudeError::InvalidField {
            field,
            row: self.label.clone(),
            expected,
        }
    }

    /// Absent and `null` are both "not there".
    fn optional_str(&self, field: &'static str) -> Result<Option<String>> {
        match self.obj.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(self.invalid(field, "a string")),
        }
    }

    fn required_str(&self, field: &'static str) -> Result<String> {
        self.optional_str(field)?.ok_or_else(|| ClaudeError::MissingField {
            field,
            row: self.label.clone(),
        })
    }

    fn optional_int(&self, field: &'static str) -> Result<Option<i64>> {
        match self.obj.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_i64().map(Some).ok_or_else(|| self.invalid(field, "an integer")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic rows modelled on the m0/2 lifecycle table; no real session names or paths.
    const NOW: i64 = 1_800_000_000_000;

    fn fixture() -> String {
        let fresh = NOW - 2_000;
        let old = NOW - 3_600_000;
        format!(
            r#"[
  {{"id":"aaaa0001","sessionId":"aaaa0001-0000-4000-8000-000000000001","name":"fm-demo-starting","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{fresh},"state":"working"}},
  {{"id":"aaaa0002","sessionId":"aaaa0002-0000-4000-8000-000000000002","name":"fm-demo-busy","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"working","status":"busy","pid":4242}},
  {{"id":"aaaa0003","sessionId":"aaaa0003-0000-4000-8000-000000000003","name":"fm-demo-parked","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":4343}},
  {{"id":"aaaa0004","sessionId":"aaaa0004-0000-4000-8000-000000000004","name":"fm-demo-stopped","cwd":"C:\\demo\\Beta","kind":"background","startedAt":{old},"state":"stopped"}},
  {{"sessionId":"bbbb0005-0000-4000-8000-000000000005","name":"","cwd":"C:\\demo\\Alpha","kind":"interactive","startedAt":{old},"status":"idle","pid":5555}},
  {{"sessionId":"bbbb0006-0000-4000-8000-000000000006","cwd":"C:\\demo","kind":"interactive","startedAt":{old},"status":"busy","pid":6666}},
  {{"id":"aaaa0007","sessionId":"aaaa0007-0000-4000-8000-000000000007","name":"fm-demo-future","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"paused"}},
  {{"id":"aaaa0008","sessionId":"aaaa0008-0000-4000-8000-000000000008","name":"fm-demo-done","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"done"}},
  {{"id":"aaaa0009","sessionId":"aaaa0009-0000-4000-8000-000000000009","name":"fm-demo-dead","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"blocked"}}
]"#
        )
    }

    fn sessions() -> Vec<Session> {
        parse(&fixture()).unwrap()
    }

    fn health_of(name_or_id: &str) -> Health {
        sessions()
            .into_iter()
            .find(|s| s.name == name_or_id || s.id == name_or_id)
            .unwrap()
            .health(NOW)
    }

    #[test]
    fn parses_every_field() {
        let s = &sessions()[2];
        assert_eq!(s.id, "aaaa0003");
        assert_eq!(s.session_id, "aaaa0003-0000-4000-8000-000000000003");
        assert_eq!(s.name, "fm-demo-parked");
        assert_eq!(s.cwd, PathBuf::from(r"C:\demo\Beta"));
        assert_eq!(s.kind, Kind::Background);
        assert_eq!(s.state, Some(SessionState::Blocked));
        assert_eq!(s.status, Some(Status::Waiting));
        assert_eq!(s.waiting_for.as_deref(), Some("permission prompt"));
        assert_eq!(s.pid, Some(4343));
        assert_eq!(s.started_at, Some(NOW - 3_600_000));
    }

    #[test]
    fn working_without_pid_is_running() {
        assert_eq!(health_of("fm-demo-starting"), Health::Running);
    }

    #[test]
    fn busy_is_running() {
        assert_eq!(health_of("fm-demo-busy"), Health::Running);
    }

    #[test]
    fn parked_is_blocked_with_reason() {
        assert_eq!(
            health_of("fm-demo-parked"),
            Health::Blocked {
                waiting_for: Some("permission prompt".into())
            }
        );
    }

    #[test]
    fn stopped_and_done_are_stopped() {
        assert_eq!(health_of("fm-demo-stopped"), Health::Stopped);
        let done = sessions().into_iter().find(|s| s.name == "fm-demo-done").unwrap();
        assert_eq!(done.state, Some(SessionState::Done));
        assert_eq!(done.health(NOW), Health::Stopped);
    }

    #[test]
    fn interactive_rows_have_no_id_or_state() {
        let all = sessions();
        assert_eq!(all[4].id, "bbbb0005");
        assert_eq!(all[4].kind, Kind::Interactive);
        assert_eq!(all[4].state, None);
        assert_eq!(all[4].health(NOW), Health::Idle);
        assert_eq!(all[5].name, "", "a missing name becomes empty");
        assert_eq!(all[5].health(NOW), Health::Running);
    }

    #[test]
    fn unknown_state_is_kept_and_unknown() {
        let s = sessions().into_iter().find(|s| s.name == "fm-demo-future").unwrap();
        assert_eq!(s.state, Some(SessionState::Other("paused".into())));
        assert_eq!(s.health(NOW), Health::Unknown);
    }

    #[test]
    fn old_live_row_without_pid_is_orphaned() {
        assert_eq!(
            health_of("fm-demo-dead"),
            Health::Orphaned {
                last_state: SessionState::Blocked
            }
        );
    }

    #[test]
    fn orphan_threshold_both_sides() {
        let limit = ORPHAN_AFTER.as_millis() as i64;
        for state in [SessionState::Working, SessionState::Blocked] {
            let mut s = sessions().remove(0);
            s.state = Some(state.clone());
            s.started_at = Some(NOW - limit);
            assert_ne!(
                s.health(NOW),
                Health::Orphaned { last_state: state.clone() },
                "exactly at the threshold is not orphaned"
            );
            s.started_at = Some(NOW - limit - 1);
            assert_eq!(s.health(NOW), Health::Orphaned { last_state: state.clone() });
            s.started_at = None;
            assert_eq!(s.health(NOW), Health::Orphaned { last_state: state });
        }
        let mut fresh_blocked = sessions().remove(0);
        fresh_blocked.state = Some(SessionState::Blocked);
        assert_eq!(fresh_blocked.health(NOW), Health::Blocked { waiting_for: None });
    }

    #[test]
    fn interactive_never_orphans() {
        let mut s = sessions().remove(5);
        s.pid = None;
        s.status = None;
        s.state = Some(SessionState::Working);
        assert_eq!(s.health(NOW), Health::Running);
    }

    #[test]
    fn unknown_kind_and_status_are_kept() {
        let json = r#"[{"sessionId":"cccc0001-x","cwd":"C:\\x","kind":"remote","status":"compacting","pid":1}]"#;
        let s = &parse(json).unwrap()[0];
        assert_eq!(s.kind, Kind::Other("remote".into()));
        assert_eq!(s.status, Some(Status::Other("compacting".into())));
        assert_eq!(s.health(NOW), Health::Unknown);
    }

    #[test]
    fn missing_required_field_names_field_and_row() {
        for (field, json) in [
            ("sessionId", r#"[{"id":"dddd0001","cwd":"C:\\x","kind":"background"}]"#),
            ("cwd", r#"[{"id":"dddd0001","sessionId":"dddd0001-x","kind":"background"}]"#),
            ("kind", r#"[{"id":"dddd0001","sessionId":"dddd0001-x","cwd":"C:\\x"}]"#),
        ] {
            match parse(json) {
                Err(ClaudeError::MissingField { field: f, row }) => {
                    assert_eq!(f, field);
                    assert_eq!(row, "dddd0001");
                }
                other => panic!("{field}: {other:?}"),
            }
        }
        let err = parse(r#"[{"cwd":"C:\\x","kind":"background"}]"#).unwrap_err();
        assert_eq!(
            err.to_string(),
            "`claude agents` row #0: missing required field `sessionId`"
        );
    }

    #[test]
    fn wrong_types_and_shapes_are_errors() {
        assert!(matches!(parse("{}"), Err(ClaudeError::NotAnArray)));
        assert!(matches!(parse("not json"), Err(ClaudeError::Json(_))));
        assert!(matches!(
            parse(r#"[{"sessionId":"e-1","cwd":"C:\\x","kind":"background","pid":"12"}]"#),
            Err(ClaudeError::InvalidField { field: "pid", .. })
        ));
        assert!(matches!(
            parse(r#"[{"sessionId":"e-1","cwd":"C:\\x","kind":"background","pid":-1}]"#),
            Err(ClaudeError::InvalidField { field: "pid", .. })
        ));
        assert!(matches!(parse("[1]"), Err(ClaudeError::InvalidField { field: "<row>", .. })));
        assert_eq!(parse("[]").unwrap(), vec![]);
    }

    #[test]
    fn health_labels() {
        assert_eq!(
            Health::Blocked { waiting_for: Some("permission prompt".into()) }.to_string(),
            "blocked (permission prompt)"
        );
        assert_eq!(
            Health::Orphaned { last_state: SessionState::Working }.to_string(),
            "orphaned (was working)"
        );
    }
}

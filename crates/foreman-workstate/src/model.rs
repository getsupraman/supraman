//! The read API v1 shapes (Skills `docs/workstate/10-read-api.md`). Lenient on purpose: v1 grows
//! additively, so unknown fields are ignored, every field defaults when missing, `| null` is an
//! `Option`, and an enum value this build does not know is `Unknown`. Instants stay ISO-8601 strings.

use serde::Deserialize;
use serde_json::Value;

/// `ItemStatus`, exactly; a value added later is `Unknown`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Open,
    Ready,
    InProgress,
    Blocked,
    Review,
    Done,
    Cancelled,
    Superseded,
    #[default]
    #[serde(other)]
    Unknown,
}

impl ItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Ready => "ready",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Review => "review",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
            Self::Unknown => "unknown",
        }
    }
}

/// `ItemResolution`, exactly; a value added later is `Unknown`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemResolution {
    Accepted,
    Reworked,
    Replanned,
    Escalated,
    Skipped,
    Wontfix,
    Duplicate,
    Obsolete,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Every item, wherever it appears. `qualified_ref` is `ref` when unique in the workspace, else `run/ref`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemRef {
    pub id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub qualified_ref: String,
    pub kind: String,
    pub title: String,
    pub status: ItemStatus,
}

/// Leaf counts below a container; `total` includes cancelled and superseded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Rollup {
    pub total: u32,
    pub done: u32,
    pub in_progress: u32,
    pub review: u32,
    pub blocked: u32,
    pub open: u32,
    pub cancelled: u32,
    pub superseded: u32,
}

impl Rollup {
    /// The MCP text's "n/m done" denominator: total − cancelled − superseded.
    pub fn counted(&self) -> u32 {
        self.total.saturating_sub(self.cancelled + self.superseded)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub name: String,
    pub role: String,
    pub external_role: Option<String>,
    pub origin: Option<String>,
    pub launched_by: Option<String>,
    pub claude_session_id: Option<String>,
    pub cwd: Option<String>,
    pub tree: Option<String>,
    pub client: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Claim {
    pub item: ItemRef,
    pub run: ItemRef,
    pub session: Option<Session>,
    /// Legacy label.
    pub agent: String,
    pub role: String,
    pub tree: Option<String>,
    pub acquired_at: String,
    pub heartbeat_at: String,
    pub expires_at: String,
    pub expired: bool,
}

impl Claim {
    /// The session's name when the claim has one, else the legacy agent label.
    pub fn holder(&self) -> &str {
        self.session.as_ref().map_or(self.agent.as_str(), |s| s.name.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JournalEvent {
    pub seq: i64,
    pub at: String,
    pub item: Option<ItemRef>,
    pub run: Option<ItemRef>,
    pub session: Option<Session>,
    pub actor: String,
    pub role: String,
    pub kind: String,
    pub summary: String,
    pub payload: Value,
}

// ---- GET /api/v1/overview

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Overview {
    pub as_of: String,
    pub seq: i64,
    pub projects: Vec<OverviewProject>,
    pub working_now: Vec<Claim>,
    pub inbox_count: u32,
    pub honesty: Honesty,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OverviewProject {
    pub item: ItemRef,
    pub rollup: Rollup,
    pub runs: Vec<OverviewRun>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OverviewRun {
    pub item: ItemRef,
    pub rollup: Rollup,
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub active_claims: u32,
    pub last_event_at: Option<String>,
    pub inbox_count: u32,
    pub last_verification: Option<LastVerification>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LastVerification {
    pub passed: bool,
    pub at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Honesty {
    pub done_unverified: u32,
    pub forced: u32,
    pub ungated: u32,
    pub followups_open: u32,
}

// ---- GET /api/v1/runs/{id}/board

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Board {
    pub as_of: String,
    pub seq: i64,
    pub run: ItemRef,
    pub project: ItemRef,
    pub intent: String,
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub rollup: Rollup,
    /// Pre-order by seq, stable: a parent precedes its children.
    pub rows: Vec<BoardRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BoardRow {
    pub item: ItemRef,
    pub parent_id: Option<String>,
    pub depth: u32,
    /// The item's position label, e.g. `"1.5"`.
    pub seq: String,
    pub has_children: bool,
    pub resolution: Option<ItemResolution>,
    pub priority: i32,
    pub severity: Option<String>,
    pub criteria: Criteria,
    /// Containers only.
    pub rollup: Option<Rollup>,
    pub claim: Option<Claim>,
    pub commit: Option<String>,
    pub verification: Option<RowVerification>,
    /// Open blockers only.
    pub blocked_by: Vec<ItemRef>,
    pub flags: Vec<String>,
    pub opened_at: String,
    pub started_at: Option<String>,
    pub closed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Criteria {
    pub checked: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RowVerification {
    pub passed: bool,
    pub run_by: String,
    pub at: String,
}

// ---- GET /api/v1/items/{id}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemPage {
    pub as_of: String,
    pub seq: i64,
    pub item: ItemRef,
    /// Project → … → parent.
    pub path: Vec<ItemRef>,
    pub intent: String,
    pub verify_cmd: Option<String>,
    pub risk_tags: Vec<String>,
    pub priority: i32,
    pub severity: Option<String>,
    pub source: Option<String>,
    pub resolution: Option<ItemResolution>,
    /// Unshaped in the doc.
    pub closed_by: Option<Value>,
    pub opened_at: String,
    pub started_at: Option<String>,
    pub closed_at: Option<String>,
    pub criteria: Vec<Criterion>,
    pub claim: Option<Claim>,
    pub reports: Vec<Report>,
    pub verifications: Vec<Verification>,
    pub artifacts: Vec<Artifact>,
    pub children: Vec<ChildRow>,
    pub findings: Vec<FindingRow>,
    pub notes: Vec<Note>,
    pub links: Links,
    pub assumptions: Vec<Assumption>,
    /// Newest 50 for this item, ascending.
    pub events: Vec<JournalEvent>,
    pub events_has_more: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Criterion {
    pub id: String,
    pub text: String,
    pub checked: bool,
    pub evidence: Option<String>,
    pub checked_at: Option<String>,
}

/// What the agent said (`work_report`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Report {
    pub attempt: u32,
    pub session: Option<Session>,
    pub agent: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub status: String,
    pub files: Vec<String>,
    pub verify: Option<String>,
    pub red_check: Option<String>,
    pub deviations: Option<String>,
    pub risks: Option<String>,
    pub not_done: Option<String>,
    pub at: String,
}

/// What was proven (`work_verify`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Verification {
    pub command: String,
    pub result_line: String,
    pub passed: bool,
    pub counts: Option<Value>,
    pub run_by: String,
    pub session: Option<Session>,
    pub red_check: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Artifact {
    pub kind: String,
    pub value: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ChildRow {
    pub item: ItemRef,
    pub criteria: Criteria,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct FindingRow {
    pub item: ItemRef,
    pub severity: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub kind: String,
    pub body: String,
    pub from: Option<ItemRef>,
    pub to: Option<ItemRef>,
    pub consumed_at: Option<String>,
    pub created_at: String,
    pub session: Option<Session>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Links {
    pub out: Vec<Link>,
    #[serde(rename = "in")]
    pub in_: Vec<Link>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Link {
    #[serde(rename = "type")]
    pub type_: String,
    pub item: ItemRef,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Assumption {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub text: String,
    pub status: String,
    pub revisit_when: Option<ItemRef>,
}

// ---- GET /api/v1/inbox

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Inbox {
    pub as_of: String,
    pub seq: i64,
    /// Newest first.
    pub rows: Vec<InboxRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct InboxRow {
    /// Stable: kind + ":" + item id.
    pub key: String,
    /// `blocked|blocker_finding|question|expired_claim|assumption_due|done_unverified|stale_in_progress|unconsumed_note`.
    pub kind: String,
    pub item: ItemRef,
    pub run: ItemRef,
    pub what: String,
    pub since: String,
    /// The resolving MCP call, qualified.
    pub call: String,
}

// ---- GET /api/v1/journal

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Journal {
    pub as_of: String,
    pub seq: i64,
    pub cursor: i64,
    pub has_more: bool,
    /// Ascending by seq.
    pub events: Vec<JournalEvent>,
}

// ---- GET /api/v1/backlog

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Backlog {
    pub as_of: String,
    pub seq: i64,
    pub run: Option<ItemRef>,
    pub groups: Vec<BacklogGroup>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct BacklogGroup {
    pub origin: Option<ItemRef>,
    pub items: Vec<ItemRef>,
}

// ---- GET /api/v1/sessions

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sessions {
    pub as_of: String,
    pub seq: i64,
    /// Most recently seen first.
    pub sessions: Vec<SessionRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SessionRow {
    pub session: Session,
    pub claims: Vec<Claim>,
}

// ---- GET /api/v1/resolve

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Resolved {
    pub as_of: String,
    pub seq: i64,
    /// Zero, one or many; the client decides.
    pub matches: Vec<ItemRef>,
}

// ---- GET /api/stream

/// One SSE frame's data: "re-read", not the change itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StreamEvent {
    pub seq: i64,
    pub run_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: String,
}

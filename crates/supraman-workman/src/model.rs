//! The read API v1 shapes (Skills `docs/workstate/10-read-api.md`), lenient on purpose: v1 grows additively.

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
    pub run: Option<ItemRef>,
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
    pub summary: Option<String>,
    pub payload: Value,
}


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
    /// The repos bound to the project; see [`crate::repo`].
    pub repos: Vec<ProjectRepo>,
    /// An archived project is listed; the client hides it.
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ProjectRepo {
    /// `remote` or `root`.
    pub kind: String,
    pub key: String,
}

/// `GET /api/v1/prompts?scope=`: the prompt actions that apply to one item, in the catalogue's order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prompts {
    pub as_of: String,
    pub seq: i64,
    pub scope: ItemRef,
    pub prompts: Vec<Prompt>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Prompt {
    pub key: String,
    pub label: String,
    pub count: Option<u32>,
    /// The whole prompt a fresh session starts with.
    pub text: String,
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


#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Board {
    pub as_of: String,
    pub seq: i64,
    pub run: ItemRef,
    pub project: Option<ItemRef>,
    pub intent: Option<String>,
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


#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ItemPage {
    pub as_of: String,
    pub seq: i64,
    pub item: ItemRef,
    /// Project → … → parent.
    pub path: Vec<ItemRef>,
    pub intent: Option<String>,
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
    pub agent: Option<String>,
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
    pub command: Option<String>,
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
    pub run: Option<ItemRef>,
    pub what: String,
    pub since: String,
    /// The resolving MCP call, qualified.
    pub call: String,
    /// `human` (question, blocked) or `controller`; absent from a server before 3.0.1.
    pub audience: Option<String>,
    /// One line per kind, at most 200 characters.
    pub detail: Option<String>,
    /// Who closed it, or the claim's agent.
    pub actor: Option<String>,
    pub tree: Option<String>,
    /// A blocker finding's target.
    pub target: Option<ItemRef>,
    pub count: Option<u32>,
    /// The item's web page.
    pub url: Option<String>,
    pub intent: Option<String>,
}


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


#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Resolved {
    pub as_of: String,
    pub seq: i64,
    /// Zero, one or many.
    pub matches: Vec<ItemRef>,
}


/// One SSE frame's data: "re-read", not the change itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StreamEvent {
    pub seq: i64,
    pub run_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEM: &str = r#"{"id":"33333333-0000-4000-8000-000000000001","ref":"6","qualifiedRef":"m4/6","kind":"task","title":"E2E","status":"done"}"#;
    const SESSION: &str = r#"{"id":"44444444-0000-4000-8000-000000000001","name":"aw-m4-6","role":"worker","externalRole":null,"origin":null,"launchedBy":null,"claudeSessionId":null,"cwd":null,"tree":null,"client":null,"firstSeenAt":"2026-10-26T10:00:00+00:00","lastSeenAt":"2026-10-26T12:00:00+00:00"}"#;

    #[test]
    fn item_page_takes_the_real_servers_nulls() {
        let json = format!(
            r#"{{"asOf":"2026-10-26T12:00:00+00:00","seq":7,"item":{ITEM},"path":[{ITEM}],"intent":null,"verifyCmd":null,
            "riskTags":[],"priority":0,"severity":null,"source":null,"resolution":null,"closedBy":null,
            "openedAt":"2026-10-26T10:00:00+00:00","startedAt":null,"closedAt":null,
            "criteria":[{{"id":"c1","text":"t","checked":true,"evidence":null,"checkedAt":null}}],"claim":null,
            "reports":[{{"attempt":1,"session":null,"agent":null,"startedAt":"2026-10-26T10:00:00+00:00","endedAt":null,"status":"done",
              "files":[],"verify":null,"redCheck":null,"deviations":null,"risks":null,"notDone":null,"at":"2026-10-26T11:00:00+00:00"}}],
            "verifications":[{{"command":null,"resultLine":"ok","passed":true,"counts":null,"runBy":"controller","session":null,"redCheck":null,"at":"2026-10-26T11:30:00+00:00"}}],
            "artifacts":[],"children":[],"findings":[{{"item":{ITEM},"severity":null,"source":null}}],
            "notes":[{{"id":"n","kind":"note","body":"b","from":null,"to":null,"consumedAt":null,"createdAt":"2026-10-26T11:00:00+00:00","session":null}}],
            "links":{{"out":[],"in":[]}},"assumptions":[{{"ref":"A1","text":"t","status":"open","revisitWhen":null}}],
            "events":[{{"seq":1,"at":"2026-10-26T11:00:00+00:00","item":null,"run":null,"session":{SESSION},"actor":"a","role":"worker",
              "kind":"transition","summary":null,"payload":{{"from":"review","to":null,"forced":null,"reason":null}}}}],
            "eventsHasMore":false,"imported":null}}"#
        );
        let page: ItemPage = serde_json::from_str(&json).unwrap();
        assert_eq!((page.intent, page.severity, page.source, page.claim), (None, None, None, None));
        assert_eq!((page.reports[0].agent.as_deref(), page.reports[0].risks.as_deref(), page.reports[0].not_done.as_deref()), (None, None, None));
        assert_eq!((page.notes[0].to.as_ref(), page.notes[0].consumed_at.as_deref()), (None, None));
        assert_eq!((page.verifications[0].command.as_deref(), page.events[0].summary.as_deref()), (None, None));
        assert_eq!(page.events[0].session.as_ref().unwrap().tree, None);
        assert!(page.events[0].payload["forced"].is_null());
    }

    #[test]
    fn board_overview_inbox_and_journal_take_nulls() {
        let claim = format!(
            r#"{{"item":{ITEM},"run":null,"session":null,"agent":"a","role":"worker","tree":null,"acquiredAt":"2026-10-26T10:00:00+00:00",
            "heartbeatAt":"2026-10-26T10:00:00+00:00","expiresAt":"2026-10-26T11:00:00+00:00","expired":false,"heldBySession":null}}"#
        );
        let board: Board = serde_json::from_str(&format!(
            r#"{{"asOf":"x","seq":1,"run":{ITEM},"project":null,"intent":null,"branch":null,"worktree":null,"rollup":{{}},
            "rows":[{{"item":{ITEM},"parentId":null,"depth":0,"seq":"1","hasChildren":false,"resolution":null,"priority":0,"severity":null,
              "criteria":{{"checked":0,"total":0}},"rollup":null,"claim":{claim},"commit":null,"verification":null,"blockedBy":[],"flags":[],
              "openedAt":"x","startedAt":null,"closedAt":null}}]}}"#
        ))
        .unwrap();
        assert_eq!((board.project, board.intent), (None, None));
        assert_eq!(board.rows[0].claim.as_ref().unwrap().run, None);
        let overview: Overview = serde_json::from_str(&format!(
            r#"{{"asOf":"x","seq":1,"projects":[{{"item":{ITEM},"rollup":{{}},"runs":[{{"item":{ITEM},"rollup":{{}},"branch":null,"worktree":null,
              "activeClaims":0,"lastEventAt":null,"inboxCount":0,"lastVerification":null}}],"repos":[],"archivedAt":null,"lastActivityAt":null,"openRuns":1}}],
            "workingNow":[{claim}],"inboxCount":0,"honesty":{{}}}}"#
        ))
        .unwrap();
        assert_eq!(overview.working_now[0].run, None);
        let inbox: Inbox =
            serde_json::from_str(&format!(r#"{{"asOf":"x","seq":1,"rows":[{{"key":"k","kind":"question","item":{ITEM},"run":null,"what":"w","since":"x","call":"c"}}]}}"#))
                .unwrap();
        assert_eq!(inbox.rows[0].run, None);
        assert_eq!((inbox.rows[0].audience.as_deref(), inbox.rows[0].target.as_ref(), inbox.rows[0].url.as_deref()), (None, None, None), "a server before 3.0.1");
        let inbox: Inbox = serde_json::from_str(&format!(
            r#"{{"asOf":"x","seq":1,"rows":[{{"key":"k","kind":"blocker_finding","item":{ITEM},"run":null,"what":"w","since":"x","call":"c",
              "audience":"controller","detail":null,"actor":"aw-1","tree":null,"target":{ITEM},"count":2,"url":"https://w/r/m4/6"}}]}}"#
        ))
        .unwrap();
        let row = &inbox.rows[0];
        assert_eq!((row.audience.as_deref(), row.detail.as_deref(), row.actor.as_deref(), row.count), (Some("controller"), None, Some("aw-1"), Some(2)));
        assert_eq!((row.target.as_ref().map(|t| t.qualified_ref.as_str()), row.url.as_deref()), (Some("m4/6"), Some("https://w/r/m4/6")));
        let journal: Journal = serde_json::from_str(
            r#"{"asOf":"x","seq":1,"cursor":0,"hasMore":false,"events":[{"seq":1,"at":"x","item":null,"run":null,"session":null,"actor":"a","role":"r","kind":"k","summary":null,"payload":null}]}"#,
        )
        .unwrap();
        assert_eq!(journal.events[0].summary, None);
    }
}

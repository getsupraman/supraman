//! What the Inbox paints of a Workman row: why the user sees it and what they can do; the resolving call is only ever copied.

use supraman_core::text::count;
use supraman_core::time;
use supraman_workman::{InboxRow, ItemRef};

pub const NOTE: &str = "unconsumed_note";
const STALE: [&str; 2] = ["stale_in_progress", "expired_claim"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub workspace: String,
    /// The row's key, `kind:item`.
    pub key: String,
    pub kind: String,
    pub badge: String,
    pub title: String,
    pub ref_: String,
    pub detail: Option<String>,
    pub since: Option<i64>,
    pub item_id: String,
    pub url: Option<String>,
    pub call: String,
    pub(crate) run: Option<ItemRef>,
}

impl Card {
    pub fn when(&self, now: i64) -> String {
        self.since.and_then(|at| time::short(at, now)).unwrap_or_default()
    }

    /// Every text the card paints, top to bottom; the render paints these and nothing else.
    pub fn texts(&self, now: i64) -> Vec<String> {
        let mut texts = vec![self.workspace.clone(), self.when(now), self.badge.clone(), self.title.clone(), self.ref_.clone()];
        texts.extend(self.detail.clone());
        texts.retain(|t| !t.is_empty());
        texts
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub(crate) key: String,
    pub caption: String,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    pub needs_you: Vec<Card>,
    pub housekeeping: Vec<Group>,
}

impl Sections {
    pub fn housekeeping_count(&self) -> usize {
        self.housekeeping.iter().map(|g| g.cards.len()).sum()
    }
}

pub(crate) fn badge(kind: &str) -> String {
    match kind {
        "blocker_finding" => "blocker".into(),
        "done_unverified" => "not verified".into(),
        "stale_in_progress" => "stalled".into(),
        "expired_claim" => "claim expired".into(),
        other => other.replace('_', " "),
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

fn detail(row: &InboxRow, held: bool, now: i64) -> Option<String> {
    if let Some(detail) = row.detail.as_deref().and_then(first_line) {
        return Some(detail);
    }
    let actor = row.actor.as_deref().filter(|a| !a.is_empty());
    match row.kind.as_str() {
        "question" => row.intent.as_deref().and_then(first_line).or_else(|| first_line(&row.what)),
        "blocked" => {
            let what = row.what.trim();
            let reason = what.strip_prefix("Blocked:").or_else(|| what.strip_prefix("blocked:")).unwrap_or(what).trim();
            (!reason.is_empty()).then(|| format!("waits on: {reason}"))
        }
        "blocker_finding" => row.target.as_ref().map(|t| format!("blocks {}", t.qualified_ref)),
        "done_unverified" => Some(match actor {
            Some(actor) => format!("closed by {actor}, not verified"),
            None => "closed, not verified".into(),
        }),
        kind if STALE.contains(&kind) => {
            let mut parts = vec![actor.unwrap_or("no holder").to_string()];
            parts.extend(time::rfc3339_ms(&row.since).map(|at| format!("idle {}", time::ago(now - at).trim_end_matches(" ago"))));
            parts.push(if held { "a Supraman session holds it" } else { "no Supraman session holds it" }.into());
            Some(parts.join(" · "))
        }
        _ => first_line(&row.what),
    }
}

/// `None` for a row the Inbox does not show.
pub fn card(workspace: &str, row: &InboxRow, held: bool, now: i64) -> Option<Card> {
    if row.kind == NOTE {
        return None;
    }
    Some(Card {
        workspace: workspace.to_string(),
        key: row.key.clone(),
        kind: row.kind.clone(),
        badge: badge(&row.kind),
        title: if row.item.title.is_empty() { row.item.qualified_ref.clone() } else { row.item.title.clone() },
        ref_: row.item.qualified_ref.clone(),
        detail: detail(row, held, now).filter(|d| row.call.is_empty() || !d.contains(row.call.as_str())),
        since: time::rfc3339_ms(&row.since),
        item_id: row.item.id.clone(),
        url: row.url.clone().filter(|u| !u.is_empty()),
        call: row.call.clone(),
        run: row.run.clone(),
    })
}

fn run_key(workspace: &str, row: &InboxRow) -> String {
    format!("{workspace}/{}", row.run.as_ref().map_or("", |r| r.id.as_str()))
}

/// A stale or expired row whose run some live Supraman session holds: its `actor` names one, or another such row's in the run does.
fn held_runs(rows: &[(&str, &InboxRow)], live: &[String]) -> Vec<String> {
    rows.iter()
        .filter(|(_, r)| STALE.contains(&r.kind.as_str()) && r.actor.as_ref().is_some_and(|a| live.contains(a)))
        .map(|(w, r)| run_key(w, r))
        .collect()
}

fn needs_you(row: &InboxRow, held: bool) -> bool {
    matches!(row.kind.as_str(), "question" | "blocked") || row.audience.as_deref() == Some("human") || (STALE.contains(&row.kind.as_str()) && !held)
}

fn caption(kind: &str, n: usize, run: Option<&ItemRef>) -> String {
    let what = match kind {
        "done_unverified" => format!("{n} done without verification"),
        "blocker_finding" => count(n, "blocker finding", "blocker findings"),
        "assumption_due" => count(n, "assumption due", "assumptions due"),
        "stale_in_progress" => format!("{n} stalled"),
        "expired_claim" => count(n, "expired claim", "expired claims"),
        other => format!("{n} {}", other.replace('_', " ")),
    };
    match run.map(|r| if r.title.is_empty() { r.qualified_ref.as_str() } else { r.title.as_str() }) {
        Some(run) if !run.is_empty() => format!("{what} · {run}"),
        _ => what,
    }
}

/// Needs you: questions, blocked items, and stalled or expired work no live Supraman session holds; the rest grouped by kind and run.
pub fn sections(rows: &[(&str, &InboxRow)], live: &[String], now: i64) -> Sections {
    let held = held_runs(rows, live);
    let mut sections = Sections::default();
    for (workspace, row) in rows {
        let run_held = held.contains(&run_key(workspace, row));
        let Some(card) = card(workspace, row, run_held, now) else { continue };
        if needs_you(row, run_held) {
            sections.needs_you.push(card);
            continue;
        }
        let key = format!("{}:{}", row.kind, run_key(workspace, row));
        match sections.housekeeping.iter_mut().find(|g| g.key == key) {
            Some(group) => group.cards.push(card),
            None => sections.housekeeping.push(Group { key, caption: String::new(), cards: vec![card] }),
        }
    }
    for group in &mut sections.housekeeping {
        let first = &group.cards[0];
        group.caption = caption(&first.kind, group.cards.len(), first.run.as_ref());
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-06T12:00:00+00:00";

    fn now() -> i64 {
        time::rfc3339_ms(NOW).unwrap()
    }

    fn item(id: &str, title: &str) -> ItemRef {
        ItemRef { id: id.into(), ref_: id.into(), qualified_ref: format!("r1/{id}"), title: title.into(), ..Default::default() }
    }

    fn row(kind: &str, id: &str) -> InboxRow {
        InboxRow {
            key: format!("{kind}:{id}"),
            kind: kind.into(),
            item: item(id, &format!("Title of {id}")),
            run: Some(ItemRef { id: "run1".into(), title: "The run".into(), ..Default::default() }),
            since: "2026-11-06T11:00:00+00:00".into(),
            call: format!("work_status(ref:\"ws/r1/{id}\")"),
            ..Default::default()
        }
    }

    #[test]
    fn each_kind_falls_back_to_its_own_line_when_the_server_sends_no_detail() {
        let d = |r: &InboxRow, held| detail(r, held, now());
        let mut q = row("question", "1");
        q.what = "Which budget?".into();
        assert_eq!(d(&q, false).as_deref(), Some("Which budget?"));
        q.intent = Some("\nPick a frame budget\nmore".into());
        assert_eq!(d(&q, false).as_deref(), Some("Pick a frame budget"), "the intent's first line first");
        let mut b = row("blocked", "2");
        b.what = "Blocked: the board API".into();
        assert_eq!(d(&b, false).as_deref(), Some("waits on: the board API"));
        let mut f = row("blocker_finding", "3");
        assert_eq!(d(&f, false), None);
        f.target = Some(item("9", "Target"));
        assert_eq!(d(&f, false).as_deref(), Some("blocks r1/9"));
        let mut u = row("done_unverified", "4");
        assert_eq!(d(&u, false).as_deref(), Some("closed, not verified"));
        u.actor = Some("aw-4".into());
        assert_eq!(d(&u, false).as_deref(), Some("closed by aw-4, not verified"));
        let mut s = row("stale_in_progress", "5");
        s.actor = Some("aw-5".into());
        assert_eq!(d(&s, false).as_deref(), Some("aw-5 · idle 1 h · no Supraman session holds it"));
        assert_eq!(d(&s, true).as_deref(), Some("aw-5 · idle 1 h · a Supraman session holds it"));
        u.detail = Some("the server's line\nsecond".into());
        assert_eq!(d(&u, false).as_deref(), Some("the server's line"), "the server's detail wins, one line");
    }

    #[test]
    fn no_card_text_holds_the_resolving_call() {
        for kind in supraman_workman::fake::INBOX_KINDS {
            let mut r = row(kind, "7");
            r.what = r.call.clone();
            r.detail = None;
            let card = card("ws", &r, false, now()).unwrap();
            assert_eq!(card.texts(now())[3..5], [card.title.clone(), card.ref_.clone()], "{kind}: title, then ref");
            assert!(card.texts(now()).iter().all(|t| !t.contains(&r.call)), "{kind}: {:?}", card.texts(now()));
        }
        assert_eq!(card("ws", &row(NOTE, "8"), false, now()), None, "a note is not shown");
    }

    #[test]
    fn needs_you_holds_questions_blocked_and_unheld_stale_work_the_rest_groups_by_kind_and_run() {
        let mut stale_held = row("stale_in_progress", "s1");
        stale_held.actor = Some("fm-live".into());
        let mut stale_held_2 = row("stale_in_progress", "s2");
        stale_held_2.actor = Some("gone".into());
        let mut expired_other_run = row("expired_claim", "e1");
        expired_other_run.run = Some(ItemRef { id: "run2".into(), title: "Other".into(), ..Default::default() });
        let rows = [
            row("question", "q"),
            row("blocked", "b"),
            row("done_unverified", "d1"),
            row(NOTE, "n"),
            row("done_unverified", "d2"),
            stale_held,
            stale_held_2,
            expired_other_run,
            row("blocker_finding", "f"),
        ];
        let refs: Vec<(&str, &InboxRow)> = rows.iter().map(|r| ("ws", r)).collect();
        let s = sections(&refs, &["fm-live".to_string()], now());
        let keys = |cards: &[Card]| cards.iter().map(|c| c.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&s.needs_you), ["question:q", "blocked:b", "expired_claim:e1"]);
        let captions: Vec<&str> = s.housekeeping.iter().map(|g| g.caption.as_str()).collect();
        assert_eq!(captions, ["2 done without verification · The run", "2 stalled · The run", "1 blocker finding · The run"]);
        assert_eq!(s.housekeeping_count(), 5);
        let mut asked = row("assumption_due", "a");
        asked.audience = Some("human".into());
        assert_eq!(keys(&sections(&[("ws", &asked)], &[], now()).needs_you), ["assumption_due:a"], "a human row the server adds later");
    }
}

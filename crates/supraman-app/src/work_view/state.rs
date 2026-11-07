//! One vocabulary of state, the web UI's (`frontend/src/state.ts`, `plan.ts`): what a row's dot, a run's squares,
//! the detail's pill and the list's filters say, so Supraman and the web agree.

use gpui::{App, Hsla};
use supraman_ui::marks::Mark;
use supraman_workman::{Board, BoardRow, ItemPage, ItemResolution, ItemStatus, Verification};
use theme::ActiveTheme as _;

/// The kinds that are a unit of work rather than a container of it.
const LEAF: [&str; 6] = ["task", "followup", "subtask", "spike", "question", "incident"];
/// Containers that are one unit of work while empty.
const EMPTY_COUNTED: [&str; 2] = ["phase", "feature"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKey {
    Gone,
    Stop,
    Doing,
    Proven,
    Claimed,
    Wait,
    Idle,
}

impl StateKey {
    pub fn as_str(self) -> &'static str {
        match self {
            StateKey::Gone => "gone",
            StateKey::Stop => "stop",
            StateKey::Doing => "doing",
            StateKey::Proven => "proven",
            StateKey::Claimed => "claimed",
            StateKey::Wait => "wait",
            StateKey::Idle => "idle",
        }
    }

    /// The legend: Verified success, Done not verified warning, In progress info, Blocked error, Waiting a ring, Open muted.
    pub fn mark(self, cx: &App) -> Mark {
        let (s, c) = (cx.theme().status(), cx.theme().colors());
        match self {
            StateKey::Proven => Mark::Filled(s.success),
            StateKey::Claimed => Mark::Filled(s.warning),
            StateKey::Doing => Mark::Filled(s.info),
            StateKey::Stop => Mark::Filled(s.error),
            StateKey::Wait => Mark::Ring(c.text_muted),
            StateKey::Idle => Mark::Filled(c.text_muted.opacity(0.55)),
            StateKey::Gone => Mark::Filled(c.text_disabled),
        }
    }

    pub fn color(self, cx: &App) -> Hsla {
        self.mark(cx).color()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub key: StateKey,
    pub label: String,
}

pub struct Input<'a> {
    pub kind: &'a str,
    pub status: ItemStatus,
    pub resolution: Option<ItemResolution>,
    pub claim_expired: bool,
    /// `None`: nothing says whether it was verified.
    pub proof: Option<bool>,
    pub waiting_on: Vec<&'a str>,
}

pub fn closed(status: ItemStatus) -> bool {
    matches!(status, ItemStatus::Done | ItemStatus::Cancelled | ItemStatus::Superseded)
}

/// Someone other than the author ran it: a worker's own run is a claim.
pub fn independent(passed: bool, run_by: &str) -> bool {
    passed && run_by != "worker"
}

fn state(label: &str, key: StateKey) -> State {
    State { key, label: label.to_string() }
}

/// The order of the tests is the precedence.
pub fn of(i: Input<'_>) -> State {
    let leaf = LEAF.contains(&i.kind);
    match i.status {
        ItemStatus::Superseded => return state("Superseded", StateKey::Gone),
        ItemStatus::Cancelled => return state("Cancelled", StateKey::Gone),
        ItemStatus::Blocked => return state("Blocked", StateKey::Stop),
        _ => {}
    }
    if i.claim_expired && !closed(i.status) {
        return state("Claim expired", StateKey::Stop);
    }
    match i.status {
        ItemStatus::InProgress => return state("In progress", StateKey::Doing),
        ItemStatus::Review => return state("In review", StateKey::Doing),
        ItemStatus::Done => {
            return match (i.resolution, leaf, i.proof) {
                (Some(ItemResolution::Obsolete), _, _) => state("Moved", StateKey::Gone),
                (_, false, _) => state("Done", StateKey::Proven),
                (_, true, Some(true)) if i.kind == "question" => state("Answered", StateKey::Proven),
                (_, true, Some(true)) => state("Verified", StateKey::Proven),
                (_, true, None) => state("Done", StateKey::Claimed),
                (_, true, Some(false)) => state("Done, not verified", StateKey::Claimed),
            };
        }
        _ => {}
    }
    if !i.waiting_on.is_empty() {
        return State { key: StateKey::Wait, label: format!("Waiting on {}", i.waiting_on.join(", ")) };
    }
    state("Open", StateKey::Idle)
}

pub fn of_row(row: &BoardRow) -> State {
    of(Input {
        kind: &row.item.kind,
        status: row.item.status,
        resolution: row.resolution,
        claim_expired: row.claim.as_ref().is_some_and(|c| c.expired),
        proof: Some(row.verification.as_ref().is_some_and(|v| independent(v.passed, &v.run_by))),
        waiting_on: row.blocked_by.iter().map(|b| b.qualified_ref.as_str()).collect(),
    })
}

pub fn of_page(page: &ItemPage) -> State {
    let latest: Option<&Verification> = page.verifications.iter().max_by(|a, b| a.at.cmp(&b.at));
    of(Input {
        kind: &page.item.kind,
        status: page.item.status,
        resolution: page.resolution,
        claim_expired: page.claim.as_ref().is_some_and(|c| c.expired),
        proof: Some(latest.is_some_and(|v| independent(v.passed, &v.run_by))),
        waiting_on: page.links.in_.iter().filter(|l| l.type_ == "blocks" && !closed(l.item.status)).map(|l| l.item.qualified_ref.as_str()).collect(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    Open,
    All,
    Attention,
}

pub const FILTERS: [(Filter, &str, &str); 3] = [(Filter::Open, "open", "Open"), (Filter::All, "all", "All"), (Filter::Attention, "attention", "Needs attention")];

pub fn keeps(filter: Filter, row: &BoardRow) -> bool {
    if row.item.status == ItemStatus::Cancelled {
        return false;
    }
    match filter {
        Filter::All => true,
        Filter::Open => !closed(row.item.status),
        Filter::Attention => matches!(of_row(row).key, StateKey::Stop | StateKey::Claimed),
    }
}

pub fn rows(board: &Board) -> impl Iterator<Item = &BoardRow> {
    board.rows.iter().filter(move |r| r.item.id != board.run.id)
}

pub fn listed(board: &Board, filter: Filter) -> Vec<&BoardRow> {
    rows(board).filter(|r| keeps(filter, r)).collect()
}

/// The counted items' states in the board's order, gone ones left out, and how many are done.
pub fn tally(board: &Board) -> (Vec<StateKey>, usize) {
    let all: Vec<&BoardRow> = rows(board).collect();
    let counted = all.iter().enumerate().filter(|(i, row)| {
        LEAF.contains(&row.item.kind.as_str())
            || (EMPTY_COUNTED.contains(&row.item.kind.as_str()) && !row.has_children && all.get(i + 1).is_none_or(|next| next.depth <= row.depth))
    });
    let keys: Vec<StateKey> = counted.map(|(_, row)| of_row(row).key).filter(|k| *k != StateKey::Gone).collect();
    let done = keys.iter().filter(|k| matches!(k, StateKey::Proven | StateKey::Claimed)).count();
    (keys, done)
}

#[cfg(test)]
mod tests {
    use supraman_workman::{Claim, ItemRef, RowVerification};

    use super::*;

    fn row(id: &str, kind: &str, status: ItemStatus) -> BoardRow {
        BoardRow { item: ItemRef { id: id.into(), qualified_ref: id.into(), kind: kind.into(), status, ..Default::default() }, depth: 1, ..Default::default() }
    }

    #[test]
    fn the_states_follow_the_web_uis_precedence() {
        let key = |r: &BoardRow| of_row(r).key;
        assert_eq!(key(&row("a", "task", ItemStatus::Blocked)), StateKey::Stop);
        let mut expired = row("b", "task", ItemStatus::InProgress);
        expired.claim = Some(Claim { expired: true, ..Default::default() });
        assert_eq!(of_row(&expired).label, "Claim expired");
        assert_eq!(key(&row("c", "task", ItemStatus::Review)), StateKey::Doing);
        let mut verified = row("d", "task", ItemStatus::Done);
        assert_eq!(of_row(&verified).label, "Done, not verified");
        verified.verification = Some(RowVerification { passed: true, run_by: "worker".into(), ..Default::default() });
        assert_eq!(of_row(&verified).key, StateKey::Claimed, "a worker's own run is a claim");
        verified.verification = Some(RowVerification { passed: true, run_by: "orchestrator".into(), ..Default::default() });
        assert_eq!(of_row(&verified).label, "Verified");
        assert_eq!(key(&row("e", "phase", ItemStatus::Done)), StateKey::Proven, "a container is done when done");
        let mut waiting = row("f", "task", ItemStatus::Open);
        waiting.blocked_by = vec![ItemRef { qualified_ref: "r/1".into(), ..Default::default() }];
        assert_eq!(of_row(&waiting), State { key: StateKey::Wait, label: "Waiting on r/1".into() });
        assert_eq!(key(&row("g", "task", ItemStatus::Ready)), StateKey::Idle);
        assert_eq!(key(&row("h", "task", ItemStatus::Cancelled)), StateKey::Gone);
    }

    #[test]
    fn the_filters_and_the_tally() {
        let run = row("run", "run", ItemStatus::InProgress);
        let rows = vec![
            run.clone(),
            row("1", "task", ItemStatus::Done),
            row("2", "task", ItemStatus::Blocked),
            row("3", "task", ItemStatus::Open),
            row("4", "task", ItemStatus::Cancelled),
        ];
        let board = Board { run: run.item.clone(), rows, ..Default::default() };
        let ids = |f| listed(&board, f).iter().map(|r| r.item.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(Filter::Open), ["2", "3"]);
        assert_eq!(ids(Filter::All), ["1", "2", "3"], "cancelled rows are left out");
        assert_eq!(ids(Filter::Attention), ["1", "2"], "done but unverified, and blocked");
        let (keys, done) = tally(&board);
        assert_eq!((keys, done), (vec![StateKey::Claimed, StateKey::Stop, StateKey::Idle], 1));
    }
}

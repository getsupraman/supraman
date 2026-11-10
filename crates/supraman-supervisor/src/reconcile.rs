use std::path::PathBuf;

use supraman_claude::{Health, Session};

use crate::{Attempts, DesiredState, Millis, NeedsKind, NeedsYou, Policy, Remedy, SECOND, SupervisorMemory, WantedSession};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The wanted name has no row: `claude --bg`.
    Launch(WantedSession),
    /// The row exists but its process is gone: `claude --bg --resume <session_id>` from `cwd`.
    Resume {
        name: String,
        id: String,
        session_id: String,
        cwd: PathBuf,
    },
    /// `claude stop <id>`; never `rm`, so the transcript stays.
    Stop { name: String, id: String },
    /// A due launch/resume deferred; `until` is `None` for [`WaitReason::Cap`].
    Wait {
        name: String,
        until: Option<Millis>,
        reason: WaitReason,
    },
    NeedsYou(NeedsYou),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    Settling,
    Backoff,
    Cap,
    /// Lost its pid moments ago: until the daemon notices the death, a resume starts a copy.
    Respawn,
}

/// A pid, or the fresh pid-less row right after `--bg`; `state` is never consulted.
pub fn is_live(s: &Session, now: Millis) -> bool {
    s.pid.is_some() || matches!(s.health(now), Health::Running | Health::Blocked { .. })
}

/// Pure; `memory` must already have observed this tick. The rule table is in the crate docs.
pub fn reconcile(
    desired: &DesiredState,
    observed: &[Session],
    memory: &SupervisorMemory,
    policy: &Policy,
    free: usize,
    now: Millis,
) -> Vec<Action> {
    let cx = Tick {
        desired,
        memory,
        policy,
        now,
    };
    let mut plan = Plan::default();
    for w in &desired.sessions {
        cx.wanted(w, observed, &mut plan);
    }
    plan.out.extend(retired_stops(desired, observed, now));
    plan.finish(free)
}

struct Tick<'a> {
    desired: &'a DesiredState,
    memory: &'a SupervisorMemory,
    policy: &'a Policy,
    now: Millis,
}

#[derive(Default)]
struct Plan {
    out: Vec<Action>,
    resumes: Vec<Action>,
    launches: Vec<Action>,
    exempt: Vec<Action>,
    last_stops: Vec<Action>,
}

impl Tick<'_> {
    fn wanted(&self, w: &WantedSession, observed: &[Session], plan: &mut Plan) {
        let rows: Vec<&Session> = observed.iter().filter(|s| s.name == w.name).collect();
        if rows.len() > 1 {
            plan.out.push(self.duplicate(w, &rows));
            return;
        }
        let row = rows.first().copied();
        let live = row.filter(|s| is_live(s, self.now));
        if !self.desired.started {
            if let Some(s) = live {
                plan.stop(w, s);
            }
            return;
        }
        if let Some(s) = live {
            plan.out.extend(self.blocked(w, s));
            return;
        }
        if let Some(a) = self.memory.get(&w.name) {
            let held = self.held(w, a);
            if !held.is_empty() {
                plan.out.extend(held);
                return;
            }
        }
        if let Some(wait) = self.respawn_wait(w, row) {
            plan.out.push(wait);
            return;
        }
        plan.due(w, row);
    }

    fn need(&self, kind: NeedsKind, name: &str, since: Millis, detail: String, action: Option<Remedy>) -> Action {
        Action::NeedsYou(NeedsYou {
            kind,
            workspace: self.desired.workspace.clone(),
            name: name.to_string(),
            since,
            detail,
            action,
        })
    }

    fn duplicate(&self, w: &WantedSession, rows: &[&Session]) -> Action {
        let ids: Vec<&str> = rows.iter().map(|s| s.id.as_str()).collect();
        let detail = format!("{} sessions are named {}: {}; remove all but one", rows.len(), w.name, ids.join(", "));
        self.need(NeedsKind::Duplicate, &w.name, self.now, detail, None)
    }

    fn blocked(&self, w: &WantedSession, s: &Session) -> Option<Action> {
        let Health::Blocked { waiting_for } = s.supervised_health(self.now) else {
            return None;
        };
        let detail = format!("waiting for {}", waiting_for.as_deref().unwrap_or("an answer"));
        Some(self.need(NeedsKind::Blocked, &w.name, self.now, detail, Some(Remedy::Attach { id: s.id.clone() })))
    }

    fn held(&self, w: &WantedSession, a: &Attempts) -> Vec<Action> {
        let now = self.now;
        let since = a.last_attempt.unwrap_or(now);
        if let Some(cwd) = &a.trust_needed {
            let detail = format!(
                "workspace {} is not trusted: run `claude` there once, accept the trust prompt, then Retry",
                cwd.display()
            );
            let action = Remedy::RunInTerminal {
                cwd: cwd.clone(),
                command: "claude".into(),
            };
            return vec![self.need(NeedsKind::TrustNeeded, &w.name, since, detail, Some(action))];
        }
        let error = a.last_error.as_deref().unwrap_or("unknown error");
        let (adopting, retry) = match &w.fork_from {
            Some(from) => (format!("adopting {}: ", from.get(..8).unwrap_or(from)), Remedy::Adopt { from: from.clone() }),
            None => (String::new(), Remedy::Retry),
        };
        if a.gave_up {
            let detail = format!("{adopting}gave up after {} failures; last: {error}", a.failures);
            return vec![self.need(NeedsKind::GaveUp, &w.name, since, detail, Some(retry))];
        }
        if let Some(t) = a.started_at
            && now - t < self.policy.settle
        {
            return vec![wait(w, Some(t + self.policy.settle), WaitReason::Settling)];
        }
        if let Some(next) = a.next_allowed
            && now < next
        {
            let detail = format!(
                "{adopting}failure {} of {}: {error}; next try in {} s",
                a.failures,
                self.policy.give_up_after,
                (next - now + SECOND - 1) / SECOND
            );
            let since = a.first_failure.unwrap_or(since);
            return vec![
                wait(w, Some(next), WaitReason::Backoff),
                self.need(NeedsKind::LaunchFailed, &w.name, since, detail, Some(retry)),
            ];
        }
        Vec::new()
    }

    fn respawn_wait(&self, w: &WantedSession, row: Option<&Session>) -> Option<Action> {
        row?;
        let seen = self.memory.get(&w.name).and_then(|a| a.last_alive)?;
        let until = seen + self.policy.respawn_grace;
        (self.now - seen < self.policy.respawn_grace).then(|| wait(w, Some(until), WaitReason::Respawn))
    }
}

fn wait(w: &WantedSession, until: Option<Millis>, reason: WaitReason) -> Action {
    Action::Wait {
        name: w.name.clone(),
        until,
        reason,
    }
}

impl Plan {
    fn stop(&mut self, w: &WantedSession, s: &Session) {
        let stop = Action::Stop {
            name: w.name.clone(),
            id: s.id.clone(),
        };
        // the relay goes last: the others may still be reporting through it
        if w.exempt_from_budget() { self.last_stops.push(stop) } else { self.out.push(stop) }
    }

    fn due(&mut self, w: &WantedSession, row: Option<&Session>) {
        let action = match row {
            Some(s) => Action::Resume {
                name: w.name.clone(),
                id: s.id.clone(),
                session_id: s.session_id.clone(),
                cwd: s.cwd.clone(),
            },
            None => Action::Launch(w.clone()),
        };
        match &action {
            _ if w.exempt_from_budget() => self.exempt.push(action),
            Action::Resume { .. } => self.resumes.push(action),
            _ => self.launches.push(action),
        }
    }

    fn finish(mut self, mut free: usize) -> Vec<Action> {
        self.out.append(&mut self.last_stops);
        self.out.append(&mut self.exempt);
        for action in self.resumes.into_iter().chain(self.launches) {
            if free > 0 {
                free -= 1;
                self.out.push(action);
            } else {
                self.out.push(Action::Wait {
                    name: action_name(&action).to_string(),
                    until: None,
                    reason: WaitReason::Cap,
                });
            }
        }
        self.out
    }
}

fn retired_stops(desired: &DesiredState, observed: &[Session], now: Millis) -> Vec<Action> {
    let mut out = Vec::new();
    for name in desired.retired.iter().filter(|n| !desired.sessions.iter().any(|w| &w.name == *n)) {
        let live: Vec<&Session> = observed.iter().filter(|s| &s.name == name && is_live(s, now)).collect();
        if let [s] = live.as_slice() {
            out.push(Action::Stop {
                name: name.clone(),
                id: s.id.clone(),
            });
        }
    }
    out
}

pub fn action_name(action: &Action) -> &str {
    match action {
        Action::Launch(w) => &w.name,
        Action::Resume { name, .. } | Action::Stop { name, .. } | Action::Wait { name, .. } => name,
        Action::NeedsYou(n) => &n.name,
    }
}

#[cfg(test)]
mod tests {
    use supraman_claude::{Kind, SessionState, Status};

    use super::*;
    use crate::{Budget, Failure};

    const NOW: Millis = 1_800_000_000_000;
    const OLD: Millis = NOW - 3_600_000;

    fn wanted(name: &str) -> WantedSession {
        WantedSession {
            name: name.into(),
            role: "workspace".into(),
            cwd: r"C:\demo\cwd".into(),
            repo: None,
            prompt: "hi".into(),
            permission_mode: "auto".into(),
            add_dirs: Vec::new(),
            resume_prompt: None,
            fork_from: None,
        }
    }

    fn desired(started: bool, names: &[&str]) -> DesiredState {
        DesiredState {
            workspace: "demo".into(),
            started,
            retired: Vec::new(),
            sessions: names.iter().map(|n| wanted(n)).collect(),
        }
    }

    fn row(name: &str, n: u32, state: Option<SessionState>, status: Option<Status>, pid: Option<u32>) -> Session {
        Session {
            id: format!("aaaa{n:04}"),
            session_id: format!("aaaa{n:04}-0000-4000-8000-000000000000"),
            name: name.into(),
            cwd: format!(r"C:\demo\row{n}").into(),
            kind: Kind::Background,
            state,
            status,
            waiting_for: None,
            pid,
            started_at: Some(OLD),
        }
    }

    fn running(name: &str, n: u32) -> Session {
        row(name, n, Some(SessionState::Working), Some(Status::Busy), Some(1000 + n))
    }

    fn stopped(name: &str, n: u32) -> Session {
        row(name, n, Some(SessionState::Stopped), None, None)
    }

    const FREE: usize = 3;

    fn run(d: &DesiredState, observed: &[Session]) -> Vec<Action> {
        reconcile(d, observed, &SupervisorMemory::default(), &Policy::default(), FREE, NOW)
    }

    fn run_with(d: &DesiredState, observed: &[Session], m: &SupervisorMemory) -> Vec<Action> {
        reconcile(d, observed, m, &Policy::default(), FREE, NOW)
    }

    fn kinds(actions: &[Action]) -> Vec<NeedsKind> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::NeedsYou(n) => Some(n.kind),
                _ => None,
            })
            .collect()
    }

    fn resume_of(s: &Session) -> Action {
        Action::Resume {
            name: s.name.clone(),
            id: s.id.clone(),
            session_id: s.session_id.clone(),
            cwd: s.cwd.clone(),
        }
    }

    // R1
    #[test]
    fn started_and_live_needs_nothing_running_or_idle() {
        let d = desired(true, &["fm-demo-workspace"]);
        assert_eq!(run(&d, &[running("fm-demo-workspace", 1)]), []);
        let idle = row("fm-demo-workspace", 1, Some(SessionState::Working), Some(Status::Idle), Some(7));
        assert_eq!(run(&d, &[idle]), []);
    }

    // R1
    #[test]
    fn done_with_live_pid_is_idle_and_never_relaunched() {
        let d = desired(true, &["fm-demo-workspace"]);
        for status in [Some(Status::Idle), None] {
            for state in [SessionState::Done, SessionState::Stopped] {
                let s = row("fm-demo-workspace", 1, Some(state), status.clone(), Some(27304));
                assert_eq!(run(&d, &[s]), [], "{status:?}");
            }
        }
    }

    // R1: the pre-process row right after `--bg`
    #[test]
    fn fresh_row_without_pid_is_starting_not_dead() {
        let d = desired(true, &["fm-demo-workspace"]);
        let mut s = row("fm-demo-workspace", 1, Some(SessionState::Working), None, None);
        s.started_at = Some(NOW - 2_000);
        assert_eq!(run(&d, &[s]), []);
    }

    // R2
    #[test]
    fn absent_is_launched() {
        let d = desired(true, &["fm-demo-workspace"]);
        let other = running("fm-other-workspace", 2);
        assert_eq!(run(&d, &[other]), [Action::Launch(wanted("fm-demo-workspace"))]);
    }

    // R3
    #[test]
    fn stopped_or_orphaned_is_resumed_from_its_own_cwd() {
        let d = desired(true, &["fm-demo-workspace"]);
        let s = stopped("fm-demo-workspace", 1);
        assert_eq!(run(&d, std::slice::from_ref(&s)), [resume_of(&s)]);
        let done = row("fm-demo-workspace", 1, Some(SessionState::Done), None, None);
        assert_eq!(run(&d, std::slice::from_ref(&done)), [resume_of(&done)]);
        let orphan = row("fm-demo-workspace", 1, Some(SessionState::Working), None, None);
        assert_eq!(orphan.health(NOW), Health::Orphaned { last_state: SessionState::Working });
        assert_eq!(run(&d, std::slice::from_ref(&orphan)), [resume_of(&orphan)]);
    }

    // R4
    #[test]
    fn blocked_is_reported_and_never_restarted() {
        let d = desired(true, &["fm-demo-workspace"]);
        let mut s = row("fm-demo-workspace", 1, Some(SessionState::Blocked), Some(Status::Waiting), Some(9));
        s.waiting_for = Some("permission prompt".into());
        match run(&d, &[s]).as_slice() {
            [Action::NeedsYou(n)] => {
                assert_eq!(n.kind, NeedsKind::Blocked);
                assert_eq!(n.workspace, "demo");
                assert_eq!(n.detail, "waiting for permission prompt");
                assert_eq!(n.action, Some(Remedy::Attach { id: "aaaa0001".into() }));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn another_homes_parked_session_raises_nothing() {
        let d = desired(true, &["fm-demo-workspace"]);
        let mut park = row("fm-selftest-abc123-park", 2, Some(SessionState::Blocked), Some(Status::Waiting), Some(9));
        park.waiting_for = Some("permission prompt".into());
        assert_eq!(run(&d, &[running("fm-demo-workspace", 1), park]), []);
    }

    // R4: a Supraman session blocked with nothing it waits for waits on a peer, not on the human
    #[test]
    fn blocked_without_waiting_for_is_not_reported() {
        let d = desired(true, &["fm-demo-workspace"]);
        for (state, status) in [(SessionState::Blocked, None), (SessionState::Done, Some(Status::Waiting))] {
            let s = row("fm-demo-workspace", 1, Some(state), status, Some(9));
            assert_eq!(run(&d, &[s]), [], "live, so never restarted either");
        }
    }

    // R5
    #[test]
    fn not_started_stops_live_wanted_sessions_only() {
        let d = desired(false, &["fm-demo-workspace", "fm-demo-alpha", "fm-demo-beta"]);
        let idle = row("fm-demo-alpha", 2, Some(SessionState::Done), Some(Status::Idle), Some(5));
        let observed = [running("fm-demo-workspace", 1), idle, stopped("fm-demo-beta", 3), running("fm-x-workspace", 4)];
        assert_eq!(
            run(&d, &observed),
            [
                Action::Stop { name: "fm-demo-workspace".into(), id: "aaaa0001".into() },
                Action::Stop { name: "fm-demo-alpha".into(), id: "aaaa0002".into() },
            ]
        );
        assert_eq!(run(&d, &[]), []);
    }

    // R6
    #[test]
    fn duplicates_are_reported_and_left_alone() {
        for started in [true, false] {
            let d = desired(started, &["fm-demo-workspace"]);
            let observed = [running("fm-demo-workspace", 1), stopped("fm-demo-workspace", 2)];
            match run(&d, &observed).as_slice() {
                [Action::NeedsYou(n)] => {
                    assert_eq!(n.kind, NeedsKind::Duplicate);
                    assert_eq!(n.action, None);
                    assert!(n.detail.contains("aaaa0001, aaaa0002"), "{}", n.detail);
                }
                other => panic!("{other:?}"),
            }
        }
    }

    // R7
    #[test]
    fn backoff_defers_and_reports_launch_failed() {
        let d = desired(true, &["fm-demo-workspace"]);
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.record_result("fm-demo-workspace", Err(Failure::Other("boom".into())), &p, NOW - 2 * SECOND);
        let actions = run_with(&d, &[], &m);
        assert_eq!(
            actions[0],
            Action::Wait { name: "fm-demo-workspace".into(), until: Some(NOW + 3 * SECOND), reason: WaitReason::Backoff }
        );
        match &actions[1] {
            Action::NeedsYou(n) => {
                assert_eq!(n.kind, NeedsKind::LaunchFailed);
                assert_eq!(n.detail, "failure 1 of 5: boom; next try in 3 s");
                assert_eq!(n.since, NOW - 2 * SECOND);
                assert_eq!(n.action, Some(Remedy::Retry));
            }
            other => panic!("{other:?}"),
        }
        let later = reconcile(&d, &[], &m, &p, FREE, NOW + 3 * SECOND);
        assert_eq!(later, [Action::Launch(wanted("fm-demo-workspace"))], "due again");
    }

    // R8
    #[test]
    fn gave_up_is_reported_and_not_attempted_until_retry() {
        let d = desired(true, &["fm-demo-workspace"]);
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        for _ in 0..5 {
            m.record_result("fm-demo-workspace", Err(Failure::Other("boom".into())), &p, NOW - 3_600_000);
        }
        let actions = run_with(&d, &[stopped("fm-demo-workspace", 1)], &m);
        assert_eq!(kinds(&actions), [NeedsKind::GaveUp]);
        assert_eq!(actions.len(), 1);
        m.retry("fm-demo-workspace");
        assert_eq!(run_with(&d, &[stopped("fm-demo-workspace", 1)], &m), [resume_of(&stopped("fm-demo-workspace", 1))]);
    }

    // R9
    #[test]
    fn trust_needed_shows_the_command_and_is_never_retried_automatically() {
        let d = desired(true, &["fm-demo-workspace"]);
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.record_result("fm-demo-workspace", Err(Failure::TrustNeeded { cwd: r"C:\demo\cwd".into() }), &p, OLD);
        let actions = run_with(&d, &[], &m);
        match actions.as_slice() {
            [Action::NeedsYou(n)] => {
                assert_eq!(n.kind, NeedsKind::TrustNeeded);
                assert_eq!(n.since, OLD);
                assert_eq!(
                    n.action,
                    Some(Remedy::RunInTerminal { cwd: r"C:\demo\cwd".into(), command: "claude".into() })
                );
            }
            other => panic!("{other:?}"),
        }
        m.retry("fm-demo-workspace");
        assert_eq!(run_with(&d, &[], &m), [Action::Launch(wanted("fm-demo-workspace"))]);
    }

    // R10
    #[test]
    fn just_started_waits_to_settle_instead_of_launching_again() {
        let d = desired(true, &["fm-demo-workspace"]);
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.record_result("fm-demo-workspace", Ok(()), &p, NOW - SECOND);
        assert_eq!(
            run_with(&d, &[], &m),
            [Action::Wait { name: "fm-demo-workspace".into(), until: Some(NOW + 19 * SECOND), reason: WaitReason::Settling }]
        );
    }

    // R11
    #[test]
    fn cap_counts_working_fm_sessions_and_defers_the_rest() {
        let names = ["fm-demo-a", "fm-demo-b", "fm-demo-c"];
        let d = desired(true, &names);
        // only the two working fm- sessions elsewhere count
        let observed = [
            running("fm-x-1", 10),
            running("fm-x-2", 11),
            running("aw-y", 12),
            row("fm-x-3", 13, Some(SessionState::Done), Some(Status::Idle), Some(5)),
            row("fm-x-4", 14, Some(SessionState::Working), None, None),
        ];
        let budget = Budget::new(3, &observed, &["x"], NOW);
        assert_eq!(budget.free, 1);
        let actions = reconcile(&d, &observed, &SupervisorMemory::default(), &Policy::default(), budget.free, NOW);
        assert_eq!(
            actions,
            [
                Action::Launch(wanted("fm-demo-a")),
                Action::Wait { name: "fm-demo-b".into(), until: None, reason: WaitReason::Cap },
                Action::Wait { name: "fm-demo-c".into(), until: None, reason: WaitReason::Cap },
            ]
        );
    }

    // R11: `state` is not the turn
    #[test]
    fn an_idle_working_row_takes_no_slot() {
        // a session that ended its turn waiting for messages stays `state=working`
        let waiting = row("fm-x-1", 1, Some(SessionState::Working), Some(Status::Idle), Some(7));
        assert_eq!(Budget::new(1, std::slice::from_ref(&waiting), &["x"], NOW).free, 1);
        assert!(!crate::mid_turn(&waiting, NOW));
        // mid-turn is `status=busy`, whatever `state` says
        let busy = row("fm-x-2", 2, Some(SessionState::Done), Some(Status::Busy), Some(8));
        assert_eq!(Budget::new(1, &[waiting, busy], &["x"], NOW).free, 0);
        let parked = row("fm-x-3", 3, Some(SessionState::Blocked), Some(Status::Waiting), Some(9));
        assert_eq!(Budget::new(1, &[parked], &["x"], NOW).free, 1);
        let mut fresh = row("fm-x-4", 4, Some(SessionState::Working), None, None);
        fresh.started_at = Some(NOW - 2_000);
        assert_eq!(Budget::new(1, &[fresh], &["x"], NOW).free, 0);
    }

    // R11 for the relay
    #[test]
    fn the_relay_is_outside_the_cap_and_its_working_row_takes_no_slot() {
        let mut d = desired(true, &["fm-demo-workspace"]);
        let relay = crate::relay_session("demo", r"C:\demo\cwd", r"C:\fm\relay\demo", "relay");
        d.sessions.insert(0, relay.clone());
        let actions = reconcile(&d, &[], &SupervisorMemory::default(), &Policy::default(), 0, NOW);
        assert_eq!(
            actions,
            [
                Action::Launch(relay),
                Action::Wait { name: "fm-demo-workspace".into(), until: None, reason: WaitReason::Cap },
            ]
        );
        let armed = [running("fm-demo-relay", 1), running("fm-x-relay", 2)];
        assert_eq!(Budget::new(1, &armed, &["demo", "x"], NOW).free, 1);
    }

    // R11 for a repo whose slug ends in relay
    #[test]
    fn a_repo_session_whose_slug_ends_in_relay_takes_a_slot() {
        let repo = crate::repo_session_name("a", "b-relay").unwrap();
        assert_eq!(repo, "fm-a-b-relay");
        assert_eq!(Budget::new(1, &[running(&repo, 1)], &["a"], NOW).free, 0);
        assert_eq!(Budget::new(1, &[running("fm-a-relay", 2)], &["a"], NOW).free, 1);
    }

    // R12
    #[test]
    fn resumes_take_slots_before_launches() {
        let d = desired(true, &["fm-demo-new", "fm-demo-old"]);
        let p = Policy::default();
        let old = stopped("fm-demo-old", 1);
        let actions = reconcile(&d, std::slice::from_ref(&old), &SupervisorMemory::default(), &p, 1, NOW);
        assert_eq!(
            actions,
            [resume_of(&old), Action::Wait { name: "fm-demo-new".into(), until: None, reason: WaitReason::Cap }]
        );
        let actions = reconcile(&d, &[old], &SupervisorMemory::default(), &p, 0, NOW);
        assert!(actions.iter().all(|a| matches!(a, Action::Wait { reason: WaitReason::Cap, .. })));
    }

    // R13
    #[test]
    fn a_session_that_just_died_is_left_to_the_daemon_for_the_respawn_grace() {
        let d = desired(true, &["fm-demo-workspace"]);
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.observe("fm-demo-workspace", true, &p, NOW - 5 * SECOND);
        // the pid went, the turn's `working` stays: an orphan by age, but alive 5 s ago
        let orphan = row("fm-demo-workspace", 1, Some(SessionState::Working), None, None);
        assert_eq!(
            run_with(&d, std::slice::from_ref(&orphan), &m),
            [Action::Wait { name: "fm-demo-workspace".into(), until: Some(NOW + 25 * SECOND), reason: WaitReason::Respawn }]
        );
        let later = reconcile(&d, std::slice::from_ref(&orphan), &m, &p, FREE, NOW + 25 * SECOND);
        assert_eq!(later, [resume_of(&orphan)], "the daemon did not respawn it: resume");
        // an idle session killed is `done` without pid, and the daemon still counts it running
        let done = row("fm-demo-workspace", 1, Some(SessionState::Done), None, None);
        assert!(matches!(
            run_with(&d, std::slice::from_ref(&done), &m).as_slice(),
            [Action::Wait { reason: WaitReason::Respawn, .. }]
        ));
        // a row never seen alive by this supervisor (e.g. stopped before the app started) resumes at once
        assert_eq!(run(&d, std::slice::from_ref(&done)), [resume_of(&done)]);
    }

    // R14
    #[test]
    fn retired_sessions_are_stopped_before_the_relay_and_never_relaunched() {
        for started in [true, false] {
            let mut d = desired(started, &["fm-demo-workspace"]);
            d.sessions.insert(0, crate::relay_session("demo", r"C:\demo", r"C:\fm\relay\demo", "r"));
            d.unwant("fm-demo-alpha");
            let observed = [running("fm-demo-relay", 1), running("fm-demo-workspace", 2), running("fm-demo-alpha", 3)];
            let actions = run(&d, &observed);
            let stops: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::Stop { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            let expected: &[&str] = if started {
                &["fm-demo-alpha"]
            } else {
                &["fm-demo-workspace", "fm-demo-alpha", "fm-demo-relay"]
            };
            assert_eq!(stops, expected, "started={started}");
            let observed = [running("fm-demo-relay", 1), running("fm-demo-workspace", 2), stopped("fm-demo-alpha", 3)];
            assert!(run(&d, &observed).iter().all(|a| action_name(a) != "fm-demo-alpha"));
        }
        let mut d = desired(true, &[]);
        d.unwant("fm-demo-alpha");
        d.want(wanted("fm-demo-alpha"));
        assert!(d.retired.is_empty());
        assert_eq!(run(&d, &[stopped("fm-demo-alpha", 3)]), [resume_of(&stopped("fm-demo-alpha", 3))]);
    }

    #[test]
    fn unwanted_sessions_are_never_touched() {
        let d = desired(true, &[]);
        assert_eq!(run(&d, &[stopped("fm-demo-workspace", 1), running("fm-demo-x", 2)]), []);
        let d = desired(false, &[]);
        assert_eq!(run(&d, &[running("fm-demo-x", 2)]), []);
    }

    // R2 for an adoption
    #[test]
    fn an_adopted_session_is_launched_as_a_copy_and_a_failed_copy_offers_retry_or_start_fresh() {
        let from = "fc5bf027-48e3-4510-90b2-63a00eb76df8";
        let mut d = desired(true, &["fm-demo-alpha"]);
        d.sessions[0].fork_from = Some(from.into());
        let original = stopped("mine", 1);
        assert_eq!(run(&d, std::slice::from_ref(&original)), [Action::Launch(d.sessions[0].clone())], "never a resume of the original's row");
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.record_result("fm-demo-alpha", Err(Failure::Other("boom".into())), &p, NOW - 2 * SECOND);
        match &run_with(&d, &[original], &m)[1] {
            Action::NeedsYou(n) => {
                assert_eq!(n.kind, NeedsKind::LaunchFailed);
                assert_eq!(n.detail, "adopting fc5bf027: failure 1 of 5: boom; next try in 3 s");
                assert_eq!(n.action, Some(Remedy::Adopt { from: from.into() }));
            }
            other => panic!("{other:?}"),
        }
    }
}

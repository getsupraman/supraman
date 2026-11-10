//! [`supraman_claude::forget_transcript`] is the one write Supraman makes under the claude dir.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use supraman_claude::{
    Claude, Forgotten, Health, Kind, Launcher, Removal, RmRefusal, Session, find_transcript, first_prompt, forget_transcript, is_session_id,
    now_ms,
};
use gpui::Context;

use super::FleetView;
use super::menu::{Confirm, Confirmed};
use crate::fleet::{self, SessionRow};

pub const HIDE_DELETED_MS: i64 = 60_000;

#[derive(Debug)]
pub enum Outcome {
    Deleted(Forgotten),
    Kept(RmRefusal),
    Failed(String),
}

pub fn delete_blocking(claude: &Claude, claude_dir: Option<&Path>, s: &Session, discard: Option<&str>) -> Outcome {
    let Some(dir) = claude_dir else {
        return Outcome::Failed(format!("Cannot delete {}: no claude dir to find its transcript in", label(s)));
    };
    if s.kind == Kind::Background {
        match claude.remove(&s.id, discard) {
            Ok(Removal::Removed(_)) => {}
            Ok(Removal::Kept(refusal)) => return Outcome::Kept(refusal),
            Err(e) => return Outcome::Failed(format!("Cannot delete {}: claude rm: {e}", label(s))),
        }
    }
    match forget_transcript(dir, &s.session_id, Some(&s.cwd)) {
        Ok(gone) => Outcome::Deleted(gone),
        Err(e) if s.kind == Kind::Background => Outcome::Failed(format!("{} was removed, but its transcript was not deleted: {e}", label(s))),
        Err(e) => Outcome::Failed(format!("Cannot delete the transcript of {}: {e}", label(s))),
    }
}

pub fn label(s: &Session) -> String {
    let id = fleet::short_id(s);
    if s.name.is_empty() { id.to_string() } else { format!("{} ({id})", s.name) }
}

pub fn is_closed(row: &SessionRow) -> bool {
    let s = &row.session;
    s.pid.is_none()
        && matches!(row.health, Health::Stopped | Health::Orphaned { .. })
        && is_session_id(&s.session_id)
        && match s.kind {
            Kind::Background => !s.id.is_empty(),
            Kind::Interactive => row.closed,
            Kind::Other(_) => false,
        }
}

pub fn offers_delete(row: &SessionRow, wanted: bool, in_tab: bool) -> bool {
    is_closed(row) && !wanted && !in_tab
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub session: Session,
    pub starred: bool,
    pub foreign: bool,
    pub wanted: bool,
    pub in_tab: bool,
    pub launcher: Option<Launcher>,
    pub transcript_found: bool,
}

/// `live`: lowercased names of the live sessions in the peer registry.
pub fn bulk_plan(candidates: Vec<Candidate>, live: &HashSet<String>) -> (Vec<Session>, Vec<(String, String)>) {
    let (mut delete, mut skipped) = (Vec::new(), Vec::new());
    for c in candidates {
        let why = if c.starred {
            Some("★ starred".to_string())
        } else if c.foreign {
            Some("another Supraman's session".to_string())
        } else if c.wanted {
            Some("this workspace's own session (Start resumes it)".to_string())
        } else if c.in_tab {
            Some("open in a tab".to_string())
        } else if let Some(l) = &c.launcher {
            Some(match (l.controller.as_deref(), l.controller_name()) {
                (Some(c), Some(name)) if live.contains(&name.trim().to_lowercase()) => format!("launched by controller {c}, still running"),
                (Some(c), _) => format!("launched by controller {c}"),
                _ => format!("launched from the brief {}", l.brief.as_deref().map_or_else(String::new, |b| b.display().to_string())),
            })
        } else if !c.transcript_found {
            Some("no transcript found, so who launched it is unknown".to_string())
        } else {
            None
        };
        match why {
            Some(why) => skipped.push((label(&c.session), why)),
            None => delete.push(c.session),
        }
    }
    (delete, skipped)
}

pub fn read_launchers(claude_dir: Option<&Path>, candidates: &mut [Candidate]) {
    for c in candidates {
        let path: Option<PathBuf> = claude_dir.and_then(|d| find_transcript(d, &c.session.session_id, Some(&c.session.cwd)));
        c.transcript_found = path.is_some();
        c.launcher = path.and_then(|p| first_prompt(&p).ok().flatten()).and_then(|p| Launcher::of(&p)).map(Launcher::resolve);
    }
}

fn live_names() -> HashSet<String> {
    use supraman_claude::peer;
    let Some(dir) = peer::sessions_dir() else { return HashSet::new() };
    peer::live_entries(&dir, &peer::System).into_iter().map(|e| e.name.trim().to_lowercase()).collect()
}

fn listed(items: &[(String, String)]) -> String {
    items.iter().map(|(who, why)| format!("{who}: {why}")).collect::<Vec<_>>().join("; ")
}

impl FleetView {
    fn wanted(&self, name: &str) -> bool {
        !name.is_empty() && self.desired.values().any(|(_, wanted)| wanted.iter().any(|w| w == name))
    }

    fn in_tab(&self, s: &Session) -> bool {
        self.tabs.iter().any(|t| Self::tab_runs(t, s))
    }

    pub fn deletable(&self, row: &SessionRow) -> bool {
        offers_delete(row, self.wanted(&row.session.name), self.in_tab(&row.session)) && !self.deleting.contains(&row.session.session_id)
    }

    fn closed_background(&self, workspace: &str) -> Vec<SessionRow> {
        let tree = self.search_tree();
        let Some(node) = tree.workspaces.into_iter().find(|w| w.name == workspace) else { return Vec::new() };
        node.into_rows()
            .filter(|r| r.session.kind == Kind::Background && is_closed(r))
            .collect()
    }

    pub fn has_closed_background(&self, workspace: &str) -> bool {
        !self.closed_background(workspace).is_empty()
    }

    pub(super) fn ask_delete(&mut self, session: Session) {
        let what = match session.kind {
            Kind::Background => "claude rm removes it, then its transcript and its tool files under the claude dir are deleted",
            _ => "Its transcript and its tool files under the claude dir are deleted, so it can no longer be resumed",
        };
        let kind = if session.kind == Kind::Background { "background session" } else { "closed session" };
        let foreign = if fleet::is_foreign(&session.name, &self.workspaces, Some(&self.desired), self.own_prefix.as_deref()) { " It is another Supraman's session." } else { "" };
        self.confirm = Some(Confirm::ask(
            format!("Delete {kind} {}? {what}. This cannot be undone.{foreign}", label(&session)),
            "Delete",
            Confirmed::Delete { session, discard: None },
        ));
    }

    pub(super) fn delete_now(&mut self, session: Session, discard: Option<String>, cx: &mut Context<Self>) {
        let sid = session.session_id.clone();
        if !self.deleting.insert(sid.clone()) {
            return;
        }
        let claude_dir = self.claude_dir.clone();
        let target = session.clone();
        let task = cx
            .background_executor()
            .spawn(async move { delete_blocking(&Claude::default(), claude_dir.as_deref(), &target, discard.as_deref()) });
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            this.update(cx, |this, cx| {
                this.deleting.remove(&sid);
                match outcome {
                    Outcome::Deleted(gone) => {
                        this.forget_rows(&[session.session_id.clone()], cx);
                        let note = if gone.transcripts.is_empty() { " (no transcript was found)" } else { "" };
                        this.show_toast(format!("Deleted {}{note}", label(&session)), cx);
                    }
                    Outcome::Kept(refusal) => this.ask_discard(session, refusal),
                    Outcome::Failed(e) => this.action_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn ask_discard(&mut self, session: Session, refusal: RmRefusal) {
        let commits = match refusal.unpushed {
            Some(1) => "1 unpushed commit".to_string(),
            Some(n) => format!("{n} unpushed commits"),
            None => "unpushed commits".to_string(),
        };
        let at = refusal.worktree.as_deref().map(|w| format!(" in {}", w.display())).unwrap_or_default();
        match refusal.discard {
            Some(value) => {
                self.confirm = Some(Confirm::ask(
                    format!(
                        "claude rm kept {}: its worktree{at} has {commits} that exist on no remote. Discard the worktree and its commits, then delete the session and its transcript?",
                        label(&session)
                    ),
                    format!("Discard {commits} and delete"),
                    Confirmed::Delete { session, discard: Some(value) },
                ))
            }
            None => self.action_error = Some(format!("claude rm kept {} ({commits}{at}): {}", label(&session), refusal.text)),
        }
    }

    pub(super) fn ask_delete_closed(&mut self, workspace: String, cx: &mut Context<Self>) {
        let mut candidates: Vec<Candidate> = self
            .closed_background(&workspace)
            .into_iter()
            .map(|r| Candidate {
                wanted: self.wanted(&r.session.name),
                in_tab: self.in_tab(&r.session),
                starred: r.starred,
                foreign: r.foreign,
                launcher: None,
                transcript_found: false,
                session: r.session,
            })
            .collect();
        let claude_dir = self.claude_dir.clone();
        let task = cx.background_executor().spawn(async move {
            read_launchers(claude_dir.as_deref(), &mut candidates);
            bulk_plan(candidates, &live_names())
        });
        cx.spawn(async move |this, cx| {
            let (sessions, skipped) = task.await;
            this.update(cx, |this, cx| {
                let skip = if skipped.is_empty() { String::new() } else { format!(" Skipped {}: {}.", skipped.len(), listed(&skipped)) };
                this.confirm = Some(if sessions.is_empty() {
                    Confirm::report(format!("No closed background session of {workspace} can be deleted.{skip}"))
                } else {
                    let n = sessions.len();
                    Confirm::ask(
                        format!(
                            "Delete {n} closed background session{} of {workspace}? claude rm removes each, then its transcript is deleted. A session with unpushed commits is kept, never discarded.{skip}",
                            if n == 1 { "" } else { "s" }
                        ),
                        format!("Delete {n}"),
                        Confirmed::DeleteClosed { workspace, sessions, skipped },
                    )
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn delete_closed_now(&mut self, workspace: String, sessions: Vec<Session>, mut skipped: Vec<(String, String)>, cx: &mut Context<Self>) {
        let sessions: Vec<Session> = sessions.into_iter().filter(|s| self.deleting.insert(s.session_id.clone())).collect();
        let claude_dir = self.claude_dir.clone();
        let todo = sessions.clone();
        let task = cx.background_executor().spawn(async move {
            let claude = Claude::default();
            todo.iter().map(|s| (s.clone(), delete_blocking(&claude, claude_dir.as_deref(), s, None))).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let results = task.await;
            this.update(cx, |this, cx| {
                let (mut deleted, mut failed) = (Vec::new(), Vec::new());
                for (s, outcome) in results {
                    this.deleting.remove(&s.session_id);
                    match outcome {
                        Outcome::Deleted(_) => deleted.push(s.session_id.clone()),
                        Outcome::Kept(r) => {
                            let commits = r.unpushed.map_or("unpushed commits".to_string(), |n| format!("{n} unpushed commit{}", if n == 1 { "" } else { "s" }));
                            skipped.push((label(&s), format!("{commits}, kept by claude rm")));
                        }
                        Outcome::Failed(e) => failed.push(e),
                    }
                }
                this.forget_rows(&deleted, cx);
                let mut text = format!("Deleted {} closed background session{} of {workspace}.", deleted.len(), if deleted.len() == 1 { "" } else { "s" });
                if !skipped.is_empty() {
                    text.push_str(&format!(" Skipped {}: {}.", skipped.len(), listed(&skipped)));
                }
                if !failed.is_empty() {
                    text.push_str(&format!(" Failed: {}.", failed.join("; ")));
                }
                this.confirm = Some(Confirm::report(text));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn forget_rows(&mut self, session_ids: &[String], cx: &mut Context<Self>) {
        if session_ids.is_empty() {
            return;
        }
        let now = now_ms();
        for sid in session_ids {
            self.deleted.insert(sid.clone(), now);
        }
        self.drop_deleted();
        if let Some(hands) = self.lifecycle.hands.clone() {
            let ids = session_ids.to_vec();
            let task = cx.background_executor().spawn(async move {
                let mut hands = hands.lock().unwrap_or_else(|e| e.into_inner());
                ids.iter().flat_map(|sid| hands.release_session(sid)).collect::<Vec<String>>()
            });
            cx.spawn(async move |this, cx| {
                let notices = task.await;
                if !notices.is_empty() {
                    this.update(cx, |this, cx| {
                        this.hand_notices.extend(notices);
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }
        self.polls.poll_nudge.wake();
        self.polls.transcripts_nudge.wake();
    }

    pub(super) fn drop_deleted(&mut self) {
        let now = now_ms();
        self.deleted.retain(|_, at| now - *at < HIDE_DELETED_MS);
        if self.deleted.is_empty() {
            return;
        }
        let deleted = &self.deleted;
        self.sessions.retain(|s| !deleted.contains_key(&s.session_id));
        self.closed.retain(|s| !deleted.contains_key(&s.session_id));
        self.invalidate_tree();
    }
}

#[cfg(test)]
mod tests {
    use supraman_claude::parse;

    use super::*;

    fn session(json: &str) -> Session {
        parse(&format!("[{json}]")).unwrap().remove(0)
    }

    fn bg(id: &str, name: &str, state: &str, pid: Option<u32>) -> Session {
        let pid = pid.map_or(String::new(), |p| format!(",\"pid\":{p},\"status\":\"idle\""));
        session(&format!(
            r#"{{"id":"{id}","sessionId":"{id}-0000-4000-8000-000000000001","name":"{name}","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"{state}"{pid}}}"#
        ))
    }

    fn row(s: Session, closed: bool) -> SessionRow {
        let health = s.health(now_ms());
        SessionRow { closed, ..SessionRow::new(s, health) }
    }

    #[test]
    fn delete_is_offered_only_for_a_closed_session_nobody_holds() {
        let stopped = row(bg("aaaa0001", "old", "stopped", None), false);
        assert!(offers_delete(&stopped, false, false));
        assert!(!offers_delete(&stopped, true, false), "the supervisor wants it");
        assert!(!offers_delete(&stopped, false, true), "open in a tab");
        assert!(offers_delete(&SessionRow { foreign: true, ..stopped.clone() }, false, false), "another Supraman's: the question says so");
        let done = row(bg("aaaa0002", "x", "done", None), false);
        assert!(offers_delete(&done, false, false));
        let gone = row(bg("aaaa0003", "x", "working", None), false);
        assert!(matches!(gone.health, Health::Orphaned { .. }) && offers_delete(&gone, false, false), "interrupted");
        assert!(!offers_delete(&row(bg("aaaa0004", "x", "done", Some(7)), false), false, false), "a live process");
        let mut no_uuid = bg("aaaa0005", "x", "stopped", None);
        no_uuid.session_id = "aaaa0005-x".into();
        assert!(!offers_delete(&row(no_uuid, false), false, false));

        let interactive = session(r#"{"sessionId":"bbbb0005-0000-4000-8000-000000000005","name":"term","cwd":"C:\\demo","kind":"interactive","startedAt":1,"status":"idle","pid":5}"#);
        assert!(!offers_delete(&row(interactive.clone(), false), false, false), "running interactive");
        let mut closed = interactive;
        closed.pid = None;
        closed.status = None;
        closed.state = Some(supraman_claude::SessionState::Stopped);
        assert!(offers_delete(&row(closed.clone(), true), false, false), "closed, from its transcript");
        assert!(!offers_delete(&row(closed, false), false, false), "only a transcript row is closed");
    }

    fn candidate(id: &str, name: &str) -> Candidate {
        Candidate { session: bg(id, name, "stopped", None), starred: false, foreign: false, wanted: false, in_tab: false, launcher: None, transcript_found: true }
    }

    #[test]
    fn the_bulk_delete_skips_stars_other_controllers_and_the_unknown_and_says_why() {
        let header = Launcher { controller: Some("autowork-b0 [408d24]".into()), brief: None };
        let gone_controller = Launcher { controller: Some("old-ctl [aaaaaa]".into()), brief: None };
        let brief_only = Launcher { controller: None, brief: Some(PathBuf::from(r"C:\r\.claude\briefs\brief-x.txt")) };
        let plan = bulk_plan(
            vec![
                candidate("aaaa0001", "plain"),
                Candidate { starred: true, ..candidate("aaaa0002", "star") },
                Candidate { launcher: Some(header), ..candidate("aaaa0003", "aw-m5-38") },
                Candidate { launcher: Some(gone_controller), ..candidate("aaaa0004", "aw-old") },
                Candidate { launcher: Some(brief_only), ..candidate("aaaa0005", "briefed") },
                Candidate { transcript_found: false, ..candidate("aaaa0006", "lost") },
                Candidate { foreign: true, ..candidate("aaaa0007", "fm-selftest-x") },
                Candidate { wanted: true, ..candidate("aaaa0008", "fm-demo-alpha") },
                Candidate { in_tab: true, ..candidate("aaaa0009", "tabbed") },
                candidate("aaaa000a", ""),
            ],
            &HashSet::from(["autowork-b0".to_string()]),
        );
        let ids: Vec<&str> = plan.0.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["aaaa0001", "aaaa000a"]);
        assert_eq!(
            plan.1,
            [
                ("star (aaaa0002)".to_string(), "★ starred".to_string()),
                ("aw-m5-38 (aaaa0003)".into(), "launched by controller autowork-b0 [408d24], still running".into()),
                ("aw-old (aaaa0004)".into(), "launched by controller old-ctl [aaaaaa]".into()),
                ("briefed (aaaa0005)".into(), r"launched from the brief C:\r\.claude\briefs\brief-x.txt".into()),
                ("lost (aaaa0006)".into(), "no transcript found, so who launched it is unknown".into()),
                ("fm-selftest-x (aaaa0007)".into(), "another Supraman's session".into()),
                ("fm-demo-alpha (aaaa0008)".into(), "this workspace's own session (Start resumes it)".into()),
                ("tabbed (aaaa0009)".into(), "open in a tab".into()),
            ]
        );
    }

    #[test]
    fn launchers_are_read_from_the_fake_claude_dirs_transcripts() {
        let tmp = tempfile::tempdir().unwrap();
        let mut a = candidate("aaaa0001", "launched");
        a.session.cwd = PathBuf::from(r"C:\work\a");
        let dir = tmp.path().join("projects").join(supraman_claude::transcript::slug(&a.session.cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let prompt = format!("{}ctl [abcdef]'. It started you", supraman_claude::transcript::CONTROLLER_HEADER);
        let line = serde_json::json!({"type":"user","message":{"role":"user","content":prompt},"cwd":"C:\\work\\a"});
        std::fs::write(dir.join(format!("{}.jsonl", a.session.session_id)), format!("{line}\n")).unwrap();
        let b = candidate("bbbb0002", "nowhere");
        let mut both = [a, b];
        read_launchers(Some(tmp.path()), &mut both);
        assert!(both[0].transcript_found);
        assert_eq!(both[0].launcher.as_ref().and_then(|l| l.controller.as_deref()), Some("ctl [abcdef]"));
        assert!(!both[1].transcript_found && both[1].launcher.is_none());
        read_launchers(None, &mut both);
        assert!(!both[0].transcript_found, "no claude dir, nothing found");
    }

    #[test]
    fn a_delete_without_a_claude_dir_fails_before_rm_runs() {
        let s = bg("aaaa0001", "x", "stopped", None);
        let missing = Claude::new(r"C:\no\such\claude.exe");
        assert!(matches!(delete_blocking(&missing, None, &s, None), Outcome::Failed(e) if e.contains("no claude dir")));
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(delete_blocking(&missing, Some(tmp.path()), &s, None), Outcome::Failed(e) if e.contains("claude rm")));
    }

    #[test]
    fn a_closed_interactive_delete_takes_its_transcript_and_dir_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = session(r#"{"sessionId":"cccc0001-0000-4000-8000-000000000001","name":"t","cwd":"C:\\work\\a","kind":"interactive","startedAt":1}"#);
        s.state = Some(supraman_claude::SessionState::Stopped);
        let dir = tmp.path().join("projects").join("C--work-a");
        std::fs::create_dir_all(dir.join(&s.session_id).join("subagents")).unwrap();
        std::fs::write(dir.join(format!("{}.jsonl", s.session_id)), "{}\n").unwrap();
        std::fs::write(dir.join("dddd0001-0000-4000-8000-000000000001.jsonl"), "{}\n").unwrap();
        let missing = Claude::new(r"C:\no\such\claude.exe");
        let Outcome::Deleted(gone) = delete_blocking(&missing, Some(tmp.path()), &s, None) else { panic!("rm is not run for an interactive session") };
        assert_eq!((gone.transcripts.len(), gone.dirs.len()), (1, 1));
        assert!(dir.join("dddd0001-0000-4000-8000-000000000001.jsonl").exists());
        assert!(!dir.join(&s.session_id).exists());
    }
}

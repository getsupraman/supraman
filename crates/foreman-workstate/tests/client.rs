use foreman_workstate::fake::{Answer, FakeWorkState};
use foreman_workstate::{
    BriefFor, Client, Error, ItemResolution, ItemStatus, JournalQuery, SessionsQuery,
};

const TOKEN: &str = "fake-reader-token";
const RUN: &str = "22222222-0000-4000-8000-000000000004";
const TASK: &str = "33333333-0000-4000-8000-000000000001";
const PROJECT: &str = "11111111-0000-4000-8000-000000000001";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn route(target: impl Into<String>, fixture_name: &str) -> (String, Answer) {
    (target.into(), Answer::json(fixture(fixture_name)))
}

fn server(routes: Vec<(String, Answer)>) -> (FakeWorkState, Client) {
    let fake = FakeWorkState::start(routes);
    let client = Client::new(fake.base(), TOKEN);
    (fake, client)
}

/// Every request carried the bearer, and nothing but `GET` was sent (C21).
fn assert_read_only_with_bearer(fake: &FakeWorkState) {
    for s in fake.seen() {
        assert_eq!(s.method, "GET", "{s:?}");
        assert_eq!(s.authorization.as_deref(), Some("Bearer fake-reader-token"), "{s:?}");
    }
}

#[test]
fn client_overview_decodes_the_fixture() {
    let (fake, c) = server(vec![route("/api/v1/overview", "overview.json")]);
    let o = c.overview().unwrap();
    assert_eq!((o.seq, o.as_of.as_str(), o.inbox_count), (1234, "2026-10-26T10:05:00+00:00", 3));
    let p = &o.projects[0];
    assert_eq!((p.item.ref_.as_str(), p.item.kind.as_str(), p.item.status), ("autowork", "project", ItemStatus::InProgress));
    assert_eq!((p.rollup.total, p.rollup.done, p.rollup.counted()), (14, 10, 13));
    assert_eq!(p.runs[0].branch.as_deref(), Some("m4/1-workstate-client"));
    assert!(p.runs[0].last_verification.as_ref().unwrap().passed);
    assert_eq!((p.runs[1].branch.as_deref(), p.runs[1].last_verification.as_ref()), (None, None));
    assert!(o.projects[1].runs.is_empty());
    let claim = &o.working_now[0];
    assert_eq!((claim.holder(), claim.item.qualified_ref.as_str(), claim.expired), ("aw-m4-1", "m4/1", false));
    assert_eq!(claim.session.as_ref().unwrap().claude_session_id, None);
    assert_eq!((o.honesty.forced, o.honesty.followups_open), (1, 4));
    assert_eq!(fake.seen()[0].target, "/api/v1/overview");
    assert_read_only_with_bearer(&fake);
}

#[test]
fn client_board_goes_by_run_id_and_tolerates_future_values() {
    let (fake, c) = server(vec![route(format!("/api/v1/runs/{RUN}/board"), "board.json")]);
    let b = c.board(RUN).unwrap();
    assert_eq!((b.run.id.as_str(), b.project.as_ref().unwrap().ref_.as_str(), b.worktree.as_deref()), (RUN, "autowork", None));
    assert_eq!(b.rows.len(), 3);
    let (r0, r1, r2) = (&b.rows[0], &b.rows[1], &b.rows[2]);
    assert_eq!((r0.depth, r0.seq.as_str(), r0.has_children, r0.criteria.checked, r0.criteria.total), (1, "1", true, 1, 4));
    assert_eq!(r0.rollup.unwrap().done, 1);
    assert_eq!(r0.claim.as_ref().unwrap().holder(), "aw-m4-1");
    assert_eq!(r1.parent_id.as_deref(), Some(TASK));
    assert_eq!((r1.resolution, r1.commit.as_deref()), (Some(ItemResolution::Accepted), Some("a439808")));
    assert_eq!(r1.verification.as_ref().unwrap().run_by, "orchestrator");
    assert_eq!(r2.resolution, Some(ItemResolution::Unknown), "a resolution added later");
    assert_eq!((r2.item.status, r2.severity.as_deref()), (ItemStatus::Blocked, Some("blocker")));
    assert_eq!(r2.blocked_by[0].id, TASK);
    assert_eq!(r2.flags, vec!["done_unverified", "a_future_flag"]);
    assert_read_only_with_bearer(&fake);
}

#[test]
fn client_item_and_brief_go_by_item_id() {
    let (fake, c) = server(vec![
        route(format!("/api/v1/items/{TASK}"), "item.json"),
        (format!("/api/v1/items/{TASK}/brief?for=worker"), Answer::text(fixture("brief.txt"))),
        (format!("/api/v1/items/{TASK}/brief?for=resume"), Answer::text("resume brief")),
    ]);
    let i = c.item(TASK).unwrap();
    assert_eq!(i.path.iter().map(|p| p.ref_.as_str()).collect::<Vec<_>>(), ["autowork", "m4"]);
    assert_eq!(i.verify_cmd.as_deref(), Some("cargo test -p foreman-workstate"));
    assert_eq!((i.criteria.len(), i.criteria[0].checked, i.criteria[1].evidence.as_deref()), (2, true, None));
    assert_eq!(i.reports[0].red_check.as_deref(), Some("dropped the header check: 2 failed"));
    assert_eq!((i.verifications[0].result_line.as_str(), i.verifications[0].passed), ("Passed! 308", true));
    assert_eq!(i.artifacts[0].value, "a439808");
    assert_eq!(i.children[0].criteria.total, 2);
    assert_eq!(i.findings[0].severity.as_deref(), Some("blocker"));
    assert_eq!((i.notes[0].from.as_ref(), i.notes[0].to.as_ref().unwrap().id.as_str()), (None, TASK));
    assert_eq!((i.links.out[0].type_.as_str(), i.links.in_.len()), ("blocks", 0));
    assert_eq!(i.assumptions[0].ref_, "A1");
    assert_eq!((i.events[0].payload["to"].as_str(), i.events_has_more), (Some("in_progress"), true));
    assert!(c.brief(TASK, BriefFor::Worker).unwrap().starts_with("TASK 1  ·  run m4"));
    assert_eq!(c.brief(TASK, BriefFor::Resume).unwrap(), "resume brief");
    assert_read_only_with_bearer(&fake);
}

#[test]
fn client_inbox_journal_backlog_sessions_take_ids_in_the_query() {
    let (fake, c) = server(vec![
        route("/api/v1/inbox", "inbox.json"),
        route(format!("/api/v1/inbox?project={PROJECT}"), "inbox.json"),
        route(format!("/api/v1/journal?scope={RUN}&since=1200&limit=50&kinds=transition%2Cnote"), "journal.json"),
        route("/api/v1/journal", "journal.json"),
        route(format!("/api/v1/backlog?project={PROJECT}"), "backlog.json"),
        route("/api/v1/sessions?active=true&name=aw-m4-1", "sessions.json"),
        route("/api/v1/sessions?claudeSessionId=aaaa%2F1", "sessions.json"),
    ]);
    let inbox = c.inbox(None).unwrap();
    assert_eq!(inbox.rows[0].call, r#"work_status(ref:"m4/2", to:"in_progress")"#);
    assert_eq!(inbox.rows[1].kind, "a_future_kind");
    c.inbox(Some(PROJECT)).unwrap();

    let q = JournalQuery { scope: Some(RUN), since: Some(1200), limit: Some(50), kinds: &["transition", "note"] };
    let j = c.journal(&q).unwrap();
    assert_eq!((j.cursor, j.has_more, j.events.len()), (1234, false, 2));
    assert_eq!((j.events[1].item.as_ref(), j.events[1].session.as_ref()), (None, None));
    c.journal(&JournalQuery::default()).unwrap();

    let b = c.backlog(PROJECT).unwrap();
    assert_eq!(b.run.unwrap().qualified_ref, "autowork/backlog");
    assert_eq!((b.groups[0].origin.as_ref().unwrap().ref_.as_str(), b.groups[0].items[0].kind.as_str()), ("m3", "followup"));
    assert_eq!(b.groups[1].origin, None);

    let s = c.sessions(&SessionsQuery { active: true, name: Some("aw-m4-1"), claude_session_id: None }).unwrap();
    assert_eq!((s.sessions[0].session.external_role.as_deref(), s.sessions[0].claims.len()), (Some("repo"), 1));
    c.sessions(&SessionsQuery { claude_session_id: Some("aaaa/1"), ..Default::default() }).unwrap();

    let targets: Vec<String> = fake.seen().into_iter().map(|s| s.target).collect();
    assert_eq!(targets.len(), 7, "{targets:?}");
    assert_read_only_with_bearer(&fake);
}

#[test]
fn client_resolve_encodes_the_typed_ref() {
    let (fake, c) = server(vec![route("/api/v1/resolve?ref=backlog", "resolve-backlog.json"), route("/api/v1/resolve?ref=m0%2Fbacklog", "resolve-autowork.json")]);
    let r = c.resolve("backlog").unwrap();
    assert_eq!(r.matches.iter().map(|m| (m.kind.as_str(), m.qualified_ref.as_str())).collect::<Vec<_>>(), [("run", "autowork/backlog"), ("phase", "m0/backlog")]);
    c.resolve("m0/backlog").unwrap();
    assert_eq!(fake.seen()[1].target, "/api/v1/resolve?ref=m0%2Fbacklog");
}

#[test]
fn client_maps_errors_and_never_says_the_token() {
    let (_fake, c) = server(vec![
        ("/api/v1/overview".into(), Answer::status(401, r#"{"error":"bad token fake-reader-token"}"#)),
        ("/api/v1/inbox".into(), Answer::json(fixture("inbox.json")).api(None)),
        ("/api/v1/journal".into(), Answer::json(fixture("journal.json")).api(Some("2"))),
        ("/api/v1/backlog?project=x".into(), Answer::json("not json")),
        ("/api/v1/sessions".into(), Answer::status(500, "").api(None)),
    ]);
    let e = c.overview().unwrap_err();
    assert_eq!(e, Error::Http { status: 401, reason: "bad token <redacted>".into() });
    let e404 = c.item("no-such-id").unwrap_err();
    assert_eq!(e404, Error::Http { status: 404, reason: "no route /api/v1/items/no-such-id".into() });
    assert!(matches!(c.inbox(None).unwrap_err(), Error::Api(m) if m.contains("without X-WorkState-Api")));
    assert!(matches!(c.journal(&JournalQuery::default()).unwrap_err(), Error::Api(m) if m.contains("speaks read API 2")));
    assert!(matches!(c.backlog("x").unwrap_err(), Error::Decode(_)));
    assert_eq!(c.sessions(&SessionsQuery::default()).unwrap_err(), Error::Http { status: 500, reason: String::new() });
    assert_eq!(e.to_string(), "WorkState answered HTTP 401: bad token <redacted>");

    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let dead = Client::new(format!("http://{free}"), TOKEN);
    let e = dead.overview().unwrap_err();
    assert!(matches!(e, Error::Transport { .. }), "{e:?}");
    for e in [e.to_string(), format!("{e:?}"), format!("{dead:?}")] {
        assert!(!e.contains(TOKEN), "{e}");
    }
}

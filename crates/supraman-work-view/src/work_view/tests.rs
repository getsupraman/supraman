use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gpui::{Bounds, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use supraman_supervisor::MintRepo;
use supraman_workman::fake::{Answer, FakeWorkman};

use super::state::StateKey;
use super::*;

const PA: &str = "aaaa0000-0000-4000-8000-00000000aa01";
const PB: &str = "bbbb0000-0000-4000-8000-000000000001";
const PC: &str = "cccc0000-0000-4000-8000-000000000001";
const RA: &str = "aaaa0000-0000-4000-8000-0000000000a1";
const RA_DONE: &str = "aaaa0000-0000-4000-8000-0000000000a2";
const RB: &str = "bbbb0000-0000-4000-8000-0000000000b1";
const RC: &str = "cccc0000-0000-4000-8000-0000000000c1";

fn id(n: u32) -> String {
    format!("aaaa0000-0000-4000-8000-00000000{n:04}")
}

fn item(id: &str, qref: &str, kind: &str, title: &str, status: &str) -> Value {
    let ref_ = qref.rsplit('/').next().unwrap_or(qref);
    json!({ "id": id, "ref": ref_, "qualifiedRef": qref, "kind": kind, "title": title, "status": status })
}

fn run_item(id: &str, qref: &str, status: &str) -> Value {
    item(id, qref, "run", &format!("Run {qref}"), status)
}

fn project(id: &str, qref: &str, repos: Value, runs: Vec<Value>) -> Value {
    let runs: Vec<Value> = runs.into_iter().map(|r| json!({ "item": r, "rollup": { "total": 4, "done": 1, "inProgress": 1, "blocked": 1, "open": 1 } })).collect();
    json!({ "item": item(id, qref, "project", &format!("Project {qref}"), "in_progress"), "rollup": { "total": 5, "done": 2, "open": 2, "blocked": 1 }, "repos": repos, "runs": runs })
}

fn row(item: Value, depth: u32) -> Value {
    json!({ "item": item, "depth": depth, "hasChildren": false, "criteria": { "checked": 0, "total": 0 }, "blockedBy": [], "flags": [] })
}

/// pa/ra's rows, one per state of the legend, and a cancelled one.
fn board() -> String {
    let mut verified = row(item(&id(1), "pa/ra/1", "task", "Verified by the controller", "done"), 1);
    verified["verification"] = json!({ "passed": true, "runBy": "orchestrator", "at": "2026-11-06T10:00:00Z" });
    verified["criteria"] = json!({ "checked": 2, "total": 3 });
    let claimed = row(item(&id(2), "pa/ra/2", "task", "Done but only claimed", "done"), 1);
    let doing = row(item(&id(3), "pa/ra/3", "task", "In progress now", "in_progress"), 1);
    let blocked = row(item(&id(4), "pa/ra/4", "task", "Blocked on the API", "blocked"), 1);
    let mut waiting = row(item(&id(5), "pa/ra/5", "task", "Waits on another", "open"), 1);
    waiting["blockedBy"] = json!([item(&id(4), "pa/ra/4", "task", "Blocked on the API", "blocked")]);
    let open = row(item(&id(6), "pa/ra/6", "task", "Still open", "open"), 1);
    let cancelled = row(item(&id(7), "pa/ra/7", "task", "Dropped", "cancelled"), 1);
    let run = row(run_item(RA, "pa/ra", "in_progress"), 0);
    json!({ "asOf": "x", "seq": 5, "run": run_item(RA, "pa/ra", "in_progress"), "project": item(PA, "pa", "project", "Project pa", "in_progress"),
            "rows": [run, verified, claimed, doing, blocked, waiting, open, cancelled] })
    .to_string()
}

fn page(item: Value, extra: Value) -> String {
    let mut page = json!({ "asOf": "x", "seq": 5, "item": item, "path": [], "openedAt": "2026-11-06T09:00:00Z" });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        page[k] = v;
    }
    page.to_string()
}

fn claim(item: Value, run: Value, claude_session_id: &str) -> Value {
    json!({ "item": item, "run": run, "role": "worker", "agent": "aw", "tree": "p:/wt/ra",
            "session": { "id": "s1", "name": "fm-w-ra", "role": "worker", "claudeSessionId": claude_session_id, "firstSeenAt": "x", "lastSeenAt": "x" },
            "acquiredAt": "2026-11-06T10:00:00Z", "heartbeatAt": "2026-11-06T10:00:00Z", "expiresAt": "2026-11-07T00:41:00Z", "expired": false })
}

/// Six questions in pa, the newest first, and one blocked item in pb.
fn inboxes() -> (String, String) {
    let row = |n: u32, kind: &str, run: Value| {
        json!({ "key": format!("{kind}:{}", id(20 + n)), "kind": kind, "item": item(&id(20 + n), &format!("pa/ra/q{n}"), "question", &format!("Question {n}?"), "open"),
                "run": run, "what": "w", "since": format!("2026-11-06T1{n}:00:00Z"), "call": "" })
    };
    let pa: Vec<Value> = (1..=6).rev().map(|n| row(n, "question", run_item(RA, "pa/ra", "in_progress"))).collect();
    let mut blocked = row(0, "blocked", run_item(RB, "pb/rb", "open"));
    blocked["item"] = item(&id(40), "pb/rb/1", "task", "Blocked in beta", "blocked");
    blocked["key"] = json!(format!("blocked:{}", id(40)));
    let inbox = |rows: Vec<Value>| json!({ "asOf": "x", "seq": 5, "rows": rows }).to_string();
    (inbox(pa), inbox(vec![blocked]))
}

fn journal(events: Vec<(i64, &str, &str, &str)>) -> String {
    let events: Vec<Value> = events
        .into_iter()
        .map(|(seq, at, qref, summary)| json!({ "seq": seq, "at": at, "item": item(&id(seq as u32), qref, "task", "t", "open"), "actor": "aw-1", "role": "worker", "kind": "transition", "summary": summary }))
        .collect();
    json!({ "asOf": "x", "seq": 20, "cursor": 20, "hasMore": false, "events": events }).to_string()
}

fn served() -> FakeWorkman {
    let overview = json!({
        "asOf": "2026-11-07T00:15:00Z", "seq": 5,
        "workingNow": [claim(item(&id(3), "pa/ra/3", "task", "In progress now", "in_progress"), run_item(RA, "pa/ra", "in_progress"), "uuid-live"),
                       claim(item(&id(50), "pc/rc/1", "task", "Elsewhere", "in_progress"), run_item(RC, "pc/rc", "in_progress"), "uuid-other")],
        "honesty": { "doneUnverified": 1, "forced": 4, "ungated": 0, "followupsOpen": 3 },
        "projects": [
            project(PA, "pa", json!([{ "kind": "remote", "key": "github.com/acme/alpha" }]), vec![run_item(RA, "pa/ra", "in_progress"), run_item(RA_DONE, "pa/old", "done")]),
            project(PC, "pc", json!([{ "kind": "root", "key": "p:/w/gamma" }]), vec![run_item(RC, "pc/rc", "in_progress")]),
            project(PB, "pb", json!([{ "kind": "root", "key": "p:/w/beta" }]), vec![run_item(RB, "pb/rb", "open")]),
        ]
    });
    let t1 = page(item(&id(1), "pa/ra/1", "task", "Verified by the controller", "done"), json!({
        "intent": "Why it exists", "verifyCmd": "cargo test",
        "criteria": [{ "id": "c1", "text": "one", "checked": true, "evidence": "merged" }],
        "reports": [{ "attempt": 1, "status": "done", "files": ["a.rs"], "at": "2026-11-06T10:00:00Z" }],
        "verifications": [{ "command": "cargo test", "resultLine": "ok", "passed": true, "runBy": "orchestrator", "at": "2026-11-06T10:05:00Z" }],
        "notes": [{ "id": "n1", "kind": "decision", "body": "Keep it.", "createdAt": "2026-11-06T10:01:00Z" }],
        "events": [{ "seq": 7, "at": "2026-11-06T10:00:00Z", "actor": "aw-1", "kind": "report", "summary": "reported" },
                   { "seq": 8, "at": "2026-11-06T10:05:00Z", "actor": "controller", "kind": "verification", "summary": "verified" }]
    }));
    let prompts = |scope: Value, list: Value| json!({ "asOf": "x", "seq": 5, "scope": scope, "prompts": list }).to_string();
    let run_prompts = json!([
        { "key": "resume", "label": "Resume", "count": null, "text": "Resume Workman run `pa/ra`." },
        { "key": "finalize", "label": "Finalize run", "count": 5, "text": "Finalize run `pa/ra`: close what is proven." },
        { "key": "cheat", "label": "Did we cheat?", "count": null, "text": "Check run `pa/ra` for unproven claims." }
    ]);
    let (inbox_a, inbox_b) = inboxes();
    let search = json!({ "asOf": "x", "seq": 5, "hits": [
        { "item": item(&id(41), "pb/rb/2", "task", "Zebra crossing", "open"), "project": item(PB, "pb", "project", "Project pb", "in_progress"), "match": "text", "snippet": "the zebra" },
        { "item": item(&id(51), "pc/rc/2", "task", "Zebra elsewhere", "open"), "project": item(PC, "pc", "project", "Project pc", "in_progress"), "match": "text", "snippet": null }
    ] });
    let journal_a = journal(vec![(10, "2026-11-06T10:00:00Z", "pa/ra/1", "1 open → in_progress"), (11, "2026-11-07T10:00:00Z", "pa/ra/2", "2 in_progress → done")]);
    let journal_b = journal(vec![(12, "2026-11-07T11:00:00Z", "pb/rb/1", "1 open → blocked")]);
    let cleanup = json!([{ "key": "cleanup", "label": "Clean up", "count": 7, "text": "Clean up pa." }]);
    FakeWorkman::start(vec![
        ("/api/v1/overview".into(), Answer::json(overview.to_string())),
        (format!("/api/v1/inbox?project={PA}"), Answer::json(inbox_a)),
        (format!("/api/v1/inbox?project={PB}"), Answer::json(inbox_b)),
        (format!("/api/v1/journal?scope={PA}&limit=150"), Answer::json(journal_a)),
        (format!("/api/v1/journal?scope={PB}&limit=150"), Answer::json(journal_b)),
        ("/api/v1/search?q=zebra&limit=20".into(), Answer::json(search.to_string())),
        (format!("/api/v1/prompts?scope={PA}"), Answer::json(prompts(item(PA, "pa", "project", "Project pa", "in_progress"), cleanup))),
        (format!("/api/v1/runs/{RA}/board"), Answer::json(board())),
        (format!("/api/v1/items/{RA}"), Answer::json(page(run_item(RA, "pa/ra", "in_progress"), json!({ "children": [{ "item": item(&id(1), "pa/ra/1", "task", "Verified by the controller", "done"), "criteria": {} }] })))),
        (format!("/api/v1/items/{}", id(1)), Answer::json(t1)),
        (format!("/api/v1/items/{}/brief?for=worker", id(1)), Answer::text("BRIEF for pa/ra/1")),
        (format!("/api/v1/items/{}", id(4)), Answer::json(page(item(&id(4), "pa/ra/4", "task", "Blocked on the API", "blocked"), json!({})))),
        (format!("/api/v1/items/{}", id(3)), Answer::json(page(item(&id(3), "pa/ra/3", "task", "In progress now", "in_progress"), json!({})))),
        (format!("/api/v1/prompts?scope={RA}"), Answer::json(prompts(run_item(RA, "pa/ra", "in_progress"), run_prompts))),
        (format!("/api/v1/prompts?scope={}", id(4)), Answer::json(prompts(item(&id(4), "pa/ra/4", "task", "B", "blocked"), json!([{ "key": "unblock", "label": "Unblock", "count": 1, "text": "Unblock pa/ra/4." }])))),
    ])
}

fn repo_of(dir: &Path) -> Option<MintRepo> {
    match dir.to_str()? {
        "alpha" => Some(MintRepo { root: r"P:\W\Alpha".into(), remote: Some("git@github.com:Acme/alpha.git".into()) }),
        "beta" => Some(MintRepo { root: r"P:\W\Beta".into(), remote: None }),
        _ => None,
    }
}

thread_local! {
    static OPENED: RefCell<Vec<Link>> = const { RefCell::new(Vec::new()) };
    static ATTACHED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn record(link: &Link, _: &mut App) {
    OPENED.with(|o| o.borrow_mut().push(link.clone()));
}

fn opened() -> Vec<Link> {
    OPENED.with(|o| o.borrow().clone())
}

struct Holder(Entity<WorkView>, Entity<WorkInbox>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1500.)).h(px(900.)).child(self.0.clone())
    }
}

struct Opened<'a> {
    view: Entity<WorkView>,
    fake: FakeWorkman,
    inbox: Entity<WorkInbox>,
    cx: &'a mut VisualTestContext,
}

fn folders() -> Vec<std::path::PathBuf> {
    vec!["alpha".into(), "beta".into(), "plain".into()]
}

/// The tab of workspace `w` (folders alpha and beta bound to pa and pb), with its Inbox and one live Supraman session.
fn open(cx: &mut TestAppContext) -> Opened<'_> {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
    });
    let fake = served();
    let client = Client::new(fake.base(), "fake-test-reader");
    let (holder, cx) = cx.add_window_view(|_, cx| {
        let hub = WorkHub::ready(client, cx);
        let inbox = WorkInbox::for_test(hub.clone(), cx);
        inbox.update(cx, |i, cx| i.set_workspaces(vec![("w".into(), folders())], repo_of, cx));
        let live = vec![LiveSession { claude_session_id: "uuid-live".into(), id: "live0001".into(), name: "fm-w-ra".into() }];
        let opening = Opening {
            workspace: "w".into(),
            folders: folders(),
            home: None,
            hits: HitTargets::default(),
            link: FleetLink { live: Rc::new(RefCell::new(live)), attach: Rc::new(|id, _, _| ATTACHED.with(|a| a.borrow_mut().push(id.to_string()))) },
            open_link_with: record,
            repo_of,
        };
        let view = cx.new(|cx| {
            let mut view = WorkView::for_test(opening, hub, cx);
            cx.observe(&inbox, |_, _, cx| cx.notify()).detach();
            view.inbox = Some(inbox.clone());
            view
        });
        Holder(view, inbox)
    });
    let (view, inbox) = holder.read_with(cx, |h, _| (h.0.clone(), h.1.clone()));
    settle(&view, cx);
    Opened { view, fake, inbox, cx }
}

fn settle(view: &Entity<WorkView>, cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        view.update(cx, |v, cx| {
            v.hits.borrow_mut().clear();
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|_, _| ());
    }
}

fn hit(view: &Entity<WorkView>, key: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds))
}

fn bounds(view: &Entity<WorkView>, key: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let painted: Vec<String> = view.read_with(cx, |v, _| v.hits.borrow().keys().filter(|k| k.starts_with("work-")).cloned().collect());
    hit(view, key, cx).unwrap_or_else(|| panic!("{key} is painted; painted: {painted:?}"))
}

fn click(view: &Entity<WorkView>, key: &str, cx: &mut VisualTestContext) {
    let at = bounds(view, key, cx).center();
    cx.simulate_click(at, Modifiers::none());
    settle(view, cx);
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

#[gpui::test]
fn c1_each_folders_project_shows_its_runs(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    let ids: Vec<String> = view.read_with(cx, |v, _| v.projects.iter().map(|p| p.item.id.clone()).collect());
    assert_eq!(ids, [PA, PB], "alpha's remote binds pa, beta's root binds pb, pc is no folder's");
    for key in ["work-project-pa", "work-project-pb", "work-nav-run-pa/ra", "work-nav-run-pb/rb", "work-nav-overview", "work-nav-activity", "work-nav-inbox=7"] {
        bounds(&view, key, cx);
    }
    assert!(hit(&view, "work-project-pc", cx).is_none() && hit(&view, "work-nav-run-pc/rc", cx).is_none());
    assert!(hit(&view, "work-nav-run-pa/old", cx).is_none(), "a closed run waits under N closed");
    click(&view, "work-closed-pa", cx);
    bounds(&view, "work-nav-run-pa/old", cx);
    click(&view, "work-nav-run-pb/rb", cx);
    assert_eq!(view.read_with(cx, |v, _| v.page.clone()), Page::Run(RB.into()));
}

#[gpui::test]
fn c2_a_selected_row_shows_in_the_detail_beside_the_list(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    assert_eq!(view.read_with(cx, |v, _| v.page.clone()), Page::Run(RA.into()), "the first active run opens");
    bounds(&view, "work-pill-pa/ra=doing", cx);
    let row = "work-row-pa/ra/3".to_string();
    click(&view, &row, cx);
    let (nav, list, detail) = (bounds(&view, "work-nav-run-pa/ra", cx), bounds(&view, &row, cx), bounds(&view, "work-pill-pa/ra/3=doing", cx));
    assert!(nav.right() <= list.left() && list.right() <= detail.left(), "navigator | list | detail: {nav:?} {list:?} {detail:?}");
    assert_eq!(view.read_with(cx, |v, _| (v.selected.clone(), v.page.clone())), (Some(id(3)), Page::Run(RA.into())));
    bounds(&view, "work-row-pa/ra/4", cx);
    view.update(cx, |v, cx| assert!(v.escape(cx)));
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), None);
    bounds(&view, "work-pill-pa/ra=doing", cx);
}

#[gpui::test]
fn c3_filters_count_and_rows_show_only_dot_ref_title_and_criteria(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    let (labels, texts) = view.read_with(cx, |v, _| {
        let board = v.board().expect("ra's board");
        (run_list::filter_labels(board), state::rows(board).map(run_list::row_texts).collect::<Vec<_>>())
    });
    assert_eq!(labels, ["Open 4", "All 6", "Needs attention 2"]);
    assert_eq!(texts[0], ["1", "Verified by the controller", "2/3"], "the criteria count only with criteria");
    assert_eq!(texts[1], ["2", "Done but only claimed"]);
    assert!(texts.iter().flatten().all(|t| !["done", "open", "blocked", "in_progress", "Verified", "claim"].contains(&t.as_str())), "no status words: {texts:?}");
    for key in ["work-filter-open", "work-filter-all", "work-filter-attention", "work-row-pa/ra/3", "work-dot-pa/ra/3=doing"] {
        bounds(&view, key, cx);
    }
    assert!(hit(&view, "work-row-pa/ra/1", cx).is_none(), "Open leaves the done rows out");
    click(&view, "work-filter-attention", cx);
    bounds(&view, "work-row-pa/ra/2", cx);
    bounds(&view, "work-row-pa/ra/4", cx);
    assert!(hit(&view, "work-row-pa/ra/3", cx).is_none());
    click(&view, "work-filter-all", cx);
    bounds(&view, "work-row-pa/ra/1", cx);
    assert!(hit(&view, "work-row-pa/ra/7", cx).is_none(), "a cancelled row is never listed");
    view.update(cx, |v, cx| {
        if let Some(BoardState::Ready(b)) = v.boards.get_mut(RA) {
            b.rows.retain(|r| r.item.kind == "run" || r.item.status == supraman_workman::ItemStatus::InProgress);
        }
        v.set_filter(Filter::Attention, cx);
    });
    settle(&view, cx);
    bounds(&view, "work-list-empty", cx);
    assert_eq!(run_list::NOTHING_MATCHES, "Nothing matches this filter.");
}

#[gpui::test]
fn c4_copy_ref_and_open_in_web_from_row_breadcrumb_and_header(cx: &mut TestAppContext) {
    let Opened { view, fake, cx, .. } = open(cx);
    let toast = |view: &Entity<WorkView>, cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.toast.text().map(str::to_string));
    click(&view, "work-row-copy-pa/ra/3", cx);
    assert_eq!((clipboard(cx), toast(&view, cx)), (Some("pa/ra/3".into()), Some("Copied pa/ra/3".into())));
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), None, "the copy icon does not select its row");
    click(&view, "work-crumb-copy", cx);
    assert_eq!((clipboard(cx), toast(&view, cx)), (Some("pa/ra".into()), Some("Copied pa/ra".into())));
    click(&view, "work-row-pa/ra/3", cx);
    click(&view, "work-detail-copy", cx);
    assert_eq!((clipboard(cx), toast(&view, cx)), (Some("pa/ra/3".into()), Some("Copied pa/ra/3".into())));

    let base = fake.base().trim_end_matches('/').to_string();
    click(&view, "work-crumb-web", cx);
    click(&view, "work-detail-web", cx);
    let row = bounds(&view, "work-row-pa/ra/4", cx).center();
    cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::none());
    settle(&view, cx);
    click(&view, "menu-work-web", cx);
    let urls: Vec<Link> = opened();
    assert_eq!(urls, [format!("{base}/r/pa/ra"), format!("{base}/r/pa/ra/3"), format!("{base}/r/pa/ra/4")].map(Link::Url));
    cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::none());
    settle(&view, cx);
    click(&view, "menu-work-copy-link", cx);
    assert_eq!(clipboard(cx), Some(format!("{base}/r/pa/ra/4")));
    cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::none());
    settle(&view, cx);
    click(&view, "menu-work-copy-title", cx);
    assert_eq!(clipboard(cx), Some("Blocked on the API".into()));
}

#[gpui::test]
fn c5_the_details_tabs_count_and_the_dots_follow_the_legend(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    click(&view, "work-filter-all", cx);
    for (n, key) in [(1, "proven"), (2, "claimed"), (3, "doing"), (4, "stop"), (5, "wait"), (6, "idle")] {
        bounds(&view, &format!("work-dot-pa/ra/{n}={key}"), cx);
    }
    view.update(cx, |_, cx| {
        let (s, c) = (cx.theme().status().clone(), cx.theme().colors().clone());
        use supraman_ui::marks::Mark;
        assert_eq!(StateKey::Proven.mark(cx), Mark::Filled(s.success), "Verified");
        assert_eq!(StateKey::Claimed.mark(cx), Mark::Filled(s.warning), "Done, not verified");
        assert_eq!(StateKey::Doing.mark(cx), Mark::Filled(s.info), "In progress");
        assert_eq!(StateKey::Stop.mark(cx), Mark::Filled(s.error), "Blocked");
        assert!(matches!(StateKey::Wait.mark(cx), Mark::Ring(_)), "Waiting: a hollow ring");
        assert_eq!(StateKey::Idle.mark(cx).color(), c.text_muted.opacity(0.55), "Open: muted");
    });
    click(&view, "work-row-pa/ra/1", cx);
    bounds(&view, "work-pill-pa/ra/1=proven", cx);
    for key in ["work-tab-overview", "work-tab-evidence", "work-tab-notes", "work-tab-activity", "work-tab-brief", "work-detail-section-intent", "work-detail-section-criteria-check-0=done", "work-detail-section-verify"] {
        bounds(&view, key, cx);
    }
    let labels = view.read_with(cx, |v, _| {
        let page = v.item.as_ref().and_then(|o| o.page()).expect("t1's page");
        detail::TABS.iter().map(|(t, _, _)| detail::tab_label(*t, page)).collect::<Vec<_>>()
    });
    assert_eq!(labels, ["Overview", "Evidence 2", "Notes 1", "Activity 2", "Brief"]);
    click(&view, "work-tab-evidence", cx);
    let (claim, proof) = (bounds(&view, "work-evidence-report-1", cx), bounds(&view, "work-evidence-verification-0=pass", cx));
    assert!(claim.right() <= proof.left(), "what was reported beside what was verified: {claim:?} {proof:?}");
    click(&view, "work-tab-notes", cx);
    bounds(&view, "work-detail-section-notes", cx);
    click(&view, "work-tab-activity", cx);
    assert!(bounds(&view, "work-event-8", cx).top() < bounds(&view, "work-event-7", cx).top(), "newest first");
    click(&view, "work-tab-brief", cx);
    click(&view, "work-detail-section-brief-copy-0", cx);
    assert_eq!(clipboard(cx), Some("BRIEF for pa/ra/1".into()), "the worker brief verbatim");
}

#[gpui::test]
fn c7_prompt_chips_are_the_servers_in_order_and_copy_their_text(cx: &mut TestAppContext) {
    let Opened { view, fake, cx, .. } = open(cx);
    let chips: Vec<Bounds<Pixels>> = ["resume", "finalize", "cheat"].iter().map(|k| bounds(&view, &format!("work-prompt-run-{k}"), cx)).collect();
    assert!(chips[0].right() <= chips[1].left() && chips[1].right() <= chips[2].left(), "in the server's order: {chips:?}");
    let labels = view.read_with(cx, |v, _| match v.prompts.by_scope.get(RA) {
        Some(prompts::PromptsState::Ready(p)) => p.iter().map(prompts::chip_label).collect::<Vec<_>>(),
        _ => Vec::new(),
    });
    assert_eq!(labels, ["Resume", "Finalize run (5)", "Did we cheat?"]);
    click(&view, "work-prompt-run-finalize", cx);
    click(&view, "work-prompt-run-copy", cx);
    assert_eq!(clipboard(cx), Some("Finalize run `pa/ra`: close what is proven.".into()));
    click(&view, "work-prompt-run-close", cx);
    assert!(hit(&view, "work-prompt-run-copy", cx).is_none());
    click(&view, "work-row-pa/ra/4", cx);
    bounds(&view, "work-prompt-item-unblock", cx);
    let asked: Vec<String> = fake.seen().into_iter().map(|s| s.target).filter(|t| t.starts_with("/api/v1/prompts")).collect();
    assert_eq!(asked, [format!("/api/v1/prompts?scope={RA}"), format!("/api/v1/prompts?scope={}", id(4))]);
}

#[gpui::test]
fn keys_move_in_the_list_and_into_the_detail(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    click(&view, "work-row-pa/ra/3", cx);
    cx.simulate_keystrokes("down");
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), Some(id(4)));
    cx.simulate_keystrokes("up up");
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), Some(id(3)), "the first row stays the first");
    cx.simulate_keystrokes("enter");
    settle(&view, cx);
    bounds(&view, "work-detail=focused", cx);
    cx.simulate_keystrokes("escape");
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| (v.detail_focused, v.selected.clone())), (false, Some(id(3))), "Esc gives the list the keys back");
    cx.simulate_keystrokes("escape");
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), None, "a second Esc clears the selection");
}

fn show_inbox_count(view: &Entity<WorkView>, cx: &mut VisualTestContext) -> Rc<std::cell::Cell<usize>> {
    let shown = Rc::new(std::cell::Cell::new(0));
    let counted = shown.clone();
    cx.update(|_, cx| {
        cx.subscribe(view, move |_, e: &WorkEvent, _| {
            if matches!(e, WorkEvent::ShowInbox) {
                counted.set(counted.get() + 1);
            }
        })
        .detach()
    });
    shown
}

fn palette_refs(view: &Entity<WorkView>, cx: &mut VisualTestContext) -> (Vec<String>, Option<String>) {
    view.read_with(cx, |v, cx| {
        let palette = v.palette.as_ref().expect("the palette is open").read(cx);
        (palette.shown().iter().map(|(f, _)| f.item.qualified_ref.clone()).collect(), palette.selected().map(|f| f.item.qualified_ref.clone()))
    })
}

/// The overview's `asOf`.
fn served_now() -> i64 {
    supraman_core::time::rfc3339_ms("2026-11-07T00:15:00Z").unwrap()
}

/// The page lies right of the navigator to the tab's right edge, and no detail column is painted.
fn spans_list_and_detail(view: &Entity<WorkView>, cx: &mut VisualTestContext) {
    let (nav, page) = (bounds(view, "work-nav-overview", cx), bounds(view, "work-page", cx));
    assert!(page.left() >= nav.right() && f32::from(page.right()) >= 1500. - 1., "the page spans list and detail: {page:?}");
    assert!(f32::from(page.size.width) > layout::LIST_MIN + layout::DETAIL_MIN, "{page:?}");
    assert!(hit(view, "work-detail-column", cx).is_none(), "no detail column beside the page");
}

fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    inner.left() >= outer.left() && inner.right() <= outer.right() && inner.top() >= outer.top() && inner.bottom() <= outer.bottom()
}

#[gpui::test]
fn the_overview_spans_both_columns_and_its_tiles_and_cards_fit(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    bounds(&view, "work-detail-column", cx);
    view.update(cx, |v, _| v.now = served_now);
    click(&view, "work-nav-overview", cx);
    spans_list_and_detail(&view, cx);
    for (label, n) in [("closed without independent proof", 1), ("forced closures", 4), ("follow-ups open", 3), ("tasks open", 6)] {
        let tile = bounds(&view, &format!("work-tile-{label}={n}"), cx);
        for i in 0..label.split(' ').count() {
            let word = bounds(&view, &format!("work-tile-word-{label}-{i}"), cx);
            assert!(inside(word, tile), "word {i} of {label:?} lies inside its tile: {word:?} {tile:?}");
        }
    }
    for p in ["pa", "pb"] {
        let (runs, card) = (bounds(&view, &format!("work-overview-runs-{p}"), cx), bounds(&view, &format!("work-overview-project-{p}"), cx));
        assert!(inside(runs, card), "{p}'s runs line fits its card: {runs:?} {card:?}");
    }
    bounds(&view, "work-working-facts-pa/ra/3=worker · pa/ra/3 · p:/wt/ra · lease ends in 26m", cx);
}

#[gpui::test]
fn c2_the_overview_says_what_needs_you_and_who_works_on_what(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    let shown = show_inbox_count(&view, cx);
    click(&view, "work-nav-overview", cx);
    bounds(&view, "work-overview-sentence=6 tasks open across 2 projects. 7 things need you.", cx);
    for n in 2..=6 {
        bounds(&view, &format!("work-overview-needs-pa/ra/q{n}"), cx);
    }
    assert!(hit(&view, "work-overview-needs-pa/ra/q1", cx).is_none() && hit(&view, "work-overview-needs-pb/rb/1", cx).is_none(), "the top five, newest first");
    click(&view, "work-overview-more=+2 more in the inbox", cx);
    assert_eq!(shown.get(), 1, "+2 more opens the Inbox");

    bounds(&view, "work-working-pa/ra/3", cx);
    assert!(hit(&view, "work-working-pc/rc/1", cx).is_none(), "a claim in a project of no folder is not this workspace's");
    click(&view, "work-working-session-pa/ra/3", cx);
    assert_eq!(ATTACHED.with(|a| a.borrow().clone()), ["live0001"], "the claim's Supraman session opens");

    for key in ["work-tile-closed without independent proof=1", "work-tile-forced closures=4", "work-tile-follow-ups open=3", "work-tile-tasks open=6"] {
        bounds(&view, key, cx);
    }
    for key in ["work-overview-project-pa", "work-overview-project-pb", "work-overview-run-pa/ra", "work-overview-run-pb/rb", "work-prompt-project-pa-cleanup", "work-legend"] {
        bounds(&view, key, cx);
    }
    assert!(hit(&view, "work-overview-run-pa/old", cx).is_none(), "a closed run is counted, not listed");
    click(&view, "work-overview-run-pb/rb", cx);
    assert_eq!(view.read_with(cx, |v, _| v.page.clone()), Page::Run(RB.into()));
    click(&view, "work-nav-overview", cx);
    click(&view, "work-overview-needs-pa/ra/q6", cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected.clone()), Some(id(26)));
}

#[gpui::test]
fn c2_activity_groups_the_projects_events_by_day_newest_first(cx: &mut TestAppContext) {
    let Opened { view, cx, .. } = open(cx);
    click(&view, "work-nav-activity", cx);
    let day = |at: &str| supraman_core::time::day(supraman_core::time::rfc3339_ms(at).unwrap());
    let (newer, older) = (bounds(&view, &format!("work-activity-day-{}=2", day("2026-11-07T10:00:00Z")), cx), bounds(&view, &format!("work-activity-day-{}=1", day("2026-11-06T10:00:00Z")), cx));
    let rows: Vec<Bounds<Pixels>> = [12, 11, 10].iter().map(|seq| bounds(&view, &format!("work-activity-{seq}"), cx)).collect();
    spans_list_and_detail(&view, cx);
    assert!(rows.iter().all(|r| f32::from(r.size.height) < 30.), "a row is one line: {rows:?}");
    assert!(newer.top() < rows[0].top() && rows[0].top() < rows[1].top() && rows[1].top() < older.top() && older.top() < rows[2].top(), "both projects' events, newest first under their day");
    click(&view, "work-activity-copy-12", cx);
    assert_eq!(clipboard(cx), Some("pb/rb/1".into()));
}

#[gpui::test]
fn c6_ctrl_p_finds_an_item_by_ref_and_enter_shows_it(cx: &mut TestAppContext) {
    let Opened { view, fake, cx, .. } = open(cx);
    click(&view, "work-row-pa/ra/3", cx);
    view.update(cx, |v, cx| v.clear_selection(cx));
    cx.simulate_keystrokes("ctrl-p");
    settle(&view, cx);
    bounds(&view, "work-find", cx);
    cx.simulate_input("pa/ra/");
    settle(&view, cx);
    let (refs, selected) = palette_refs(&view, cx);
    assert_eq!(refs[..3], ["pa/ra/1", "pa/ra/2", "pa/ra/3"], "the board's rows in order: {refs:?}");
    assert_eq!(selected.as_deref(), Some("pa/ra/1"));
    bounds(&view, &format!("work-find-row-{}", id(1)), cx);
    cx.simulate_keystrokes("down down");
    settle(&view, cx);
    assert_eq!(palette_refs(&view, cx).1.as_deref(), Some("pa/ra/3"));
    cx.simulate_keystrokes("up");
    settle(&view, cx);
    assert_eq!(palette_refs(&view, cx).1.as_deref(), Some("pa/ra/2"));
    cx.simulate_keystrokes("enter");
    settle(&view, cx);
    assert!(view.read_with(cx, |v, _| v.palette.is_none()), "Enter closes the palette");
    assert_eq!(view.read_with(cx, |v, _| (v.page.clone(), v.selected.clone())), (Page::Run(RA.into()), Some(id(2))), "the item selected in its run");

    cx.simulate_keystrokes("ctrl-p");
    settle(&view, cx);
    cx.simulate_keystrokes("escape");
    settle(&view, cx);
    assert!(view.read_with(cx, |v, _| v.palette.is_none()), "Esc closes it");

    cx.simulate_keystrokes("ctrl-p");
    settle(&view, cx);
    cx.simulate_input("zebra");
    settle(&view, cx);
    assert_eq!(palette_refs(&view, cx).0, ["pb/rb/2"], "Workman's hit in a bound project; pc's is not this workspace's");
    assert!(fake.seen().iter().any(|s| s.target == "/api/v1/search?q=zebra&limit=20"));
    click(&view, &format!("work-find-row-{}", id(41)), cx);
    assert_eq!(view.read_with(cx, |v, _| (v.palette.is_none(), v.selected.clone())), (true, Some(id(41))), "a click opens it");
}

#[gpui::test]
fn the_inbox_reads_every_project_its_folders_bind(cx: &mut TestAppContext) {
    let Opened { fake, inbox, cx, .. } = open(cx);
    let rows: Vec<String> = inbox.read_with(cx, |i, _| i.rows().iter().map(|(w, r)| format!("{w}:{}", r.item.qualified_ref)).collect());
    assert_eq!(rows, ["w:pa/ra/q6", "w:pa/ra/q5", "w:pa/ra/q4", "w:pa/ra/q3", "w:pa/ra/q2", "w:pa/ra/q1", "w:pb/rb/1"], "pa's and pb's rows, newest first");
    let mut asked: Vec<String> = fake.seen().into_iter().map(|s| s.target).filter(|t| t.starts_with("/api/v1/inbox") || t.starts_with("/api/v1/resolve")).collect();
    asked.dedup();
    assert_eq!(asked, [format!("/api/v1/inbox?project={PA}"), format!("/api/v1/inbox?project={PB}")], "never a project found by the workspace's name");
}

#[gpui::test]
fn needs_you_brings_the_inbox_to_its_top_and_highlights_it_for_a_moment(cx: &mut TestAppContext) {
    let Opened { inbox, cx, .. } = open(cx);
    inbox.update(cx, |i, _| i.scroll.set_offset(gpui::point(px(0.), px(-120.))));
    inbox.update(cx, |i, cx| i.show(cx));
    assert_eq!(inbox.read_with(cx, |i, _| (i.scroll.offset().y, i.flashing())), (px(0.), true));
    cx.executor().advance_clock(super::inbox::FLASH + Duration::from_millis(10));
    cx.run_until_parked();
    assert!(!inbox.read_with(cx, |i, _| i.flashing()));
}

fn bound(inbox: &Entity<WorkInbox>, folder: &str, cx: &mut VisualTestContext) -> Binding {
    let task = inbox.update(cx, |i, cx| i.bind_folder(folder.into(), cx));
    let slot = Rc::new(RefCell::new(None));
    let put = slot.clone();
    cx.update(|_, cx| cx.spawn(async move |_| *put.borrow_mut() = Some(task.await)).detach());
    cx.run_until_parked();
    slot.take().expect("the binding is read")
}

fn overview_reads(fake: &FakeWorkman) -> usize {
    fake.seen().into_iter().filter(|s| s.target == "/api/v1/overview").count()
}

#[gpui::test]
fn a_folder_binds_the_project_of_its_repo_through_the_kept_overview(cx: &mut TestAppContext) {
    let Opened { fake, inbox, cx, .. } = open(cx);
    let before = overview_reads(&fake);
    assert!(matches!(bound(&inbox, "alpha", cx), Binding::Project(p) if p.id == PA), "alpha's remote binds pa");
    assert!(matches!(bound(&inbox, "beta", cx), Binding::Project(p) if p.id == PB), "beta's root binds pb");
    assert_eq!(bound(&inbox, "plain", cx), Binding::Unbound, "no repo, no project");
    assert_eq!(overview_reads(&fake), before, "the Inbox's overview is used, never read again");
}

#[gpui::test]
fn without_a_kept_overview_a_folder_is_bound_by_one_read_and_unreachable_workman_says_why(cx: &mut TestAppContext) {
    let Opened { fake, cx, .. } = open(cx);
    let hub = cx.update(|_, cx| WorkHub::ready(Client::new(fake.base(), "fake-test-reader"), cx));
    let fresh = cx.update(|_, cx| WorkInbox::of_repos(hub, repo_of, cx));
    let before = overview_reads(&fake);
    assert!(matches!(bound(&fresh, "beta", cx), Binding::Project(p) if p.id == PB));
    assert!(matches!(bound(&fresh, "alpha", cx), Binding::Project(p) if p.id == PA));
    assert_eq!(overview_reads(&fake), before + 1, "read once, then kept");
    let closed = cx.update(|_, cx| WorkHub::ready(Client::new(&supraman_workman::fake::closed_base(), "fake-test-reader"), cx));
    let unreachable = cx.update(|_, cx| WorkInbox::of_repos(closed, repo_of, cx));
    match bound(&unreachable, "alpha", cx) {
        Binding::Unavailable(why) => assert!(!why.is_empty(), "Workman's words"),
        other => panic!("an unreachable Workman is unavailable, not {other:?}"),
    }
}

#[gpui::test]
fn show_project_lands_on_its_first_active_run_else_the_overview(cx: &mut TestAppContext) {
    let Opened { view, inbox, cx, .. } = open(cx);
    view.update(cx, |v, cx| v.show_project(PB.into(), cx));
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.page.clone()), Page::Run(RB.into()));
    view.update(cx, |v, cx| v.show_project(PC.into(), cx));
    settle(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.page.clone()), Page::Overview, "a project the workspace does not bind lands on the Overview");
    let hub = view.read_with(cx, |v, _| v.hub.clone());
    let opening = Opening {
        workspace: "w".into(),
        folders: folders(),
        home: None,
        hits: HitTargets::default(),
        link: FleetLink { live: Rc::new(RefCell::new(Vec::new())), attach: Rc::new(|_, _, _| {}) },
        open_link_with: record,
        repo_of,
    };
    let fresh = cx.update(|_, cx| {
        cx.new(|cx| {
            let mut v = WorkView::for_test(opening, hub, cx);
            v.inbox = Some(inbox.clone());
            v.show_project(PB.into(), cx);
            v
        })
    });
    cx.run_until_parked();
    assert_eq!(fresh.read_with(cx, |v, _| v.page.clone()), Page::Run(RB.into()), "asked before its projects were bound, it lands once they are");
}

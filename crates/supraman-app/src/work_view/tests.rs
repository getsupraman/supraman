use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use gpui::{Bounds, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext};
use serde_json::{Value, json};
use supraman_supervisor::MintRepo;
use supraman_workman::fake::{Answer, FakeWorkman};

use super::state::StateKey;
use super::*;

const PA: &str = "aaaa0000-0000-4000-8000-000000000001";
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
    json!({ "item": item(id, qref, "project", &format!("Project {qref}"), "in_progress"), "rollup": {}, "repos": repos, "runs": runs })
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

fn served() -> FakeWorkman {
    let overview = json!({
        "asOf": "x", "seq": 5,
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
    FakeWorkman::start(vec![
        ("/api/v1/overview".into(), Answer::json(overview.to_string())),
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
}

fn record(link: &Link, _: &mut App) {
    OPENED.with(|o| o.borrow_mut().push(link.clone()));
}

fn opened() -> Vec<Link> {
    OPENED.with(|o| o.borrow().clone())
}

struct Holder(Entity<WorkView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1500.)).h(px(900.)).child(self.0.clone())
    }
}

struct Opened<'a> {
    view: Entity<WorkView>,
    fake: FakeWorkman,
    cx: &'a mut VisualTestContext,
}

fn open(cx: &mut TestAppContext) -> Opened<'_> {
    cx.update(|cx| supraman_ui::init_globals(cx, "test"));
    let fake = served();
    let client = Client::new(fake.base(), "fake-test-reader");
    let (holder, cx) = cx.add_window_view(|_, cx| {
        let hub = WorkHub::ready(client, cx);
        let opening = Opening {
            workspace: "w".into(),
            folders: vec!["alpha".into(), "beta".into(), "plain".into()],
            home: None,
            hits: HitTargets::default(),
            link: FleetLink { live: Rc::default(), attach: Rc::new(|_, _, _| {}) },
            open_link_with: record,
            repo_of,
        };
        Holder(cx.new(|cx| WorkView::for_test(opening, hub, cx)))
    });
    let view = holder.read_with(cx, |h, _| h.0.clone());
    settle(&view, cx);
    Opened { view, fake, cx }
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
    for key in ["work-project-pa", "work-project-pb", "work-nav-run-pa/ra", "work-nav-run-pb/rb", "work-nav-overview", "work-nav-activity", "work-nav-inbox=0"] {
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
    let Opened { view, fake, cx } = open(cx);
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
    let Opened { view, fake, cx } = open(cx);
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

#[test]
fn the_inboxs_workspace_is_the_one_project_among_the_matches() {
    let iref = |id: &str, kind: &str| ItemRef { id: id.into(), ref_: "w".into(), qualified_ref: "w".into(), kind: kind.into(), ..Default::default() };
    let resolved = |m: Vec<ItemRef>| supraman_workman::Resolved { matches: m, ..Default::default() };
    assert_eq!(pick_project(resolved(vec![iref("r", "run"), iref("p", "project")]), "w").unwrap().id, "p");
    assert!(pick_project(resolved(vec![iref("r", "run")]), "w").unwrap_err().contains("no project with ref 'w'"));
    assert!(pick_project(resolved(vec![iref("a", "project"), iref("b", "project")]), "w").unwrap_err().contains("names 2 Workman projects"));
}

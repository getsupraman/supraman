use std::cell::RefCell;

use gpui::{Bounds, Entity, Modifiers, Render, TestAppContext, VisualTestContext};
use serde_json::json;
use supraman_workman::Client;
use supraman_workman::fake::{Answer, FakeWorkman};

use super::*;

const KINDS: [&str; 7] = ["question", "blocked", "blocker_finding", "done_unverified", "stale_in_progress", "expired_claim", "assumption_due"];
const LONG: &str = "A title long enough to need more than two lines in a narrow sidebar card, so the card cuts it with an ellipsis and keeps the whole of it for a tooltip";

fn call(kind: &str) -> String {
    format!("work_status(ref:\"ws/r1/{kind}\", to:\"in_progress\")")
}

/// One row per kind as Workman 3.0.3 serves it; the blocked row's title is long.
fn served() -> Vec<InboxRow> {
    let rows: Vec<_> = KINDS
        .iter()
        .map(|kind| {
            let title = if *kind == "blocked" { LONG.to_string() } else { format!("Title of {kind}") };
            json!({
                "key": format!("{kind}:id-{kind}"), "kind": kind,
                "item": { "id": format!("id-{kind}"), "ref": kind, "qualifiedRef": format!("r1/{kind}"), "kind": "task", "title": title, "status": "open" },
                "run": { "id": "run1", "ref": "r1", "qualifiedRef": "r1", "kind": "run", "title": "The run", "status": "in_progress" },
                "what": call(kind), "since": "2026-11-06T11:00:00+00:00", "call": call(kind),
                "audience": if matches!(*kind, "question" | "blocked") { "human" } else { "controller" },
                "detail": if *kind == "assumption_due" { json!("revisit: the board API ships") } else { json!(null) },
                "actor": "aw-1", "tree": null,
                "target": if *kind == "blocker_finding" { json!({ "id": "t", "ref": "9", "qualifiedRef": "r1/9", "kind": "task", "title": "T", "status": "open" }) } else { json!(null) },
                "count": 1, "url": format!("https://work.example/r/ws/r1/{kind}")
            })
        })
        .collect();
    let fake = FakeWorkman::start(vec![("/api/v1/inbox".into(), Answer::json(json!({ "asOf": "x", "seq": 1, "rows": rows }).to_string()))]);
    Client::new(fake.base(), "t").inbox(None).expect("the fake's inbox").rows
}

struct Holder {
    cards: Vec<Card>,
    hits: HitTargets,
    done: Rc<RefCell<Vec<(String, CardAction)>>>,
}

impl Render for Holder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = supraman_core::time::rfc3339_ms("2026-11-06T12:00:00+00:00").unwrap();
        div().w(px(320.)).flex().flex_col().gap_1().children(self.cards.iter().map(|card| {
            let (done, key) = (self.done.clone(), card.key.clone());
            let on: OnCard = Rc::new(move |action, _, _| done.borrow_mut().push((key.clone(), action)));
            render(card, &self.hits, now, on, cx)
        }))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Holder>, &mut VisualTestContext) {
    cx.update(|cx| supraman_ui::init_globals(cx, "test"));
    let now = supraman_core::time::rfc3339_ms("2026-11-06T12:00:00+00:00").unwrap();
    let cards: Vec<Card> = served().iter().filter_map(|row| cards::card("ws", row, false, now)).collect();
    assert_eq!(cards.len(), KINDS.len());
    let (view, cx) = cx.add_window_view(|_, _| Holder { cards, hits: HitTargets::default(), done: Rc::default() });
    redraw(&view, cx);
    redraw(&view, cx);
    (view, cx)
}

fn redraw(view: &Entity<Holder>, cx: &mut VisualTestContext) {
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|_, _| ());
}

fn bounds(view: &Entity<Holder>, key: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    view.read_with(cx, |h, _| h.hits.borrow().get(key).map(|h| h.bounds)).unwrap_or_else(|| panic!("{key} is painted"))
}

fn click(view: &Entity<Holder>, key: &str, cx: &mut VisualTestContext) {
    let at = bounds(view, key, cx).center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn every_kinds_card_leads_with_the_items_title_then_its_ref_and_never_paints_the_call(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let cards = view.read_with(cx, |h, _| h.cards.clone());
    let now = supraman_core::time::rfc3339_ms("2026-11-06T12:00:00+00:00").unwrap();
    for card in &cards {
        let (head, title, ref_) = (bounds(&view, &part_key("head", card), cx), bounds(&view, &part_key("title", card), cx), bounds(&view, &part_key("ref", card), cx));
        assert!(head.bottom() <= title.top() + px(1.) && title.bottom() <= ref_.top() + px(1.), "{}: head {head:?}, title {title:?}, ref {ref_:?}", card.kind);
        if let Some(detail) = card.detail.as_ref() {
            assert!(bounds(&view, &part_key("detail", card), cx).top() >= ref_.bottom() - px(1.), "{}: the detail {detail:?} under the ref", card.kind);
        }
        let texts = card.texts(now);
        assert_eq!(texts.iter().position(|t| t == &card.title), Some(3), "{}: workspace, time and badge on the head line, then the title: {texts:?}", card.kind);
        assert!(texts.iter().all(|t| !t.contains("work_status")), "{}: {texts:?}", card.kind);
    }
    let blocked = cards.iter().find(|c| c.kind == "blocked").unwrap();
    let one = bounds(&view, &part_key("title", &cards[0]), cx).size.height;
    let long = bounds(&view, &part_key("title", blocked), cx).size.height;
    assert!(long > one * 1.5 && long < one * 2.5, "a long title takes two lines, no more: {long:?} against one line's {one:?}");
    assert!(supraman_ui::clipped::cut_at(&part_key("title", blocked)).is_some(), "the cut title shows its whole text as a tooltip");
    assert!(supraman_ui::clipped::cut_at(&part_key("title", &cards[0])).is_none());
    assert_eq!(cards.iter().find(|c| c.kind == "blocker_finding").unwrap().detail.as_deref(), Some("blocks r1/9"));
    assert_eq!(cards.iter().find(|c| c.kind == "assumption_due").unwrap().detail.as_deref(), Some("revisit: the board API ships"));
}

#[gpui::test]
fn copy_puts_the_call_on_the_clipboard_and_each_action_reaches_the_owner(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let card = view.read_with(cx, |h, _| h.cards.iter().find(|c| c.kind == "done_unverified").cloned().unwrap());
    click(&view, &part_key("copy", &card), cx);
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(call("done_unverified")));
    click(&view, &part_key("web", &card), cx);
    click(&view, &part_key("open", &card), cx);
    click(&view, &work_view::dismiss_key_of(&card.key), cx);
    click(&view, &part_key("title", &card), cx);
    let done = view.read_with(cx, |h, _| h.done.borrow().clone());
    let key = card.key.clone();
    assert_eq!(
        done,
        [(key.clone(), CardAction::Copied), (key.clone(), CardAction::Web), (key.clone(), CardAction::Open), (key.clone(), CardAction::Dismiss), (key, CardAction::Open)],
        "a hover button's click stays its own; a click on the card opens it"
    );
}

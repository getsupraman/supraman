use std::rc::Rc;

use supraman_ui::session::LiveSession;
use supraman_workman::{Claim, ItemPage};

use super::*;

#[derive(Clone)]
pub struct FleetLink {
    pub live: Rc<dyn Fn() -> Vec<LiveSession>>,
    pub attach: Rc<dyn Fn(&str, &mut Window, &mut App)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ItemState {
    Loading,
    Ready(Box<ItemPage>),
    Failed(String),
}

/// The item the detail shows: the selected row, else the run.
pub struct OpenItem {
    pub id: String,
    pub(crate) state: ItemState,
}

impl OpenItem {
    pub(crate) fn page(&self) -> Option<&ItemPage> {
        match &self.state {
            ItemState::Ready(page) => Some(page),
            _ => None,
        }
    }
}

pub(crate) fn run_of(page: &ItemPage) -> Option<&str> {
    if page.item.kind == "run" {
        return Some(page.item.id.as_str());
    }
    page.path.iter().rev().find(|p| p.kind == "run").map(|p| p.id.as_str())
}

pub(crate) fn item_to_reload(event: &StreamEvent, open: &OpenItem) -> bool {
    if event.item_id.as_deref() == Some(open.id.as_str()) {
        return true;
    }
    match (&open.state, event.run_id.as_deref()) {
        (ItemState::Ready(page), Some(run)) => run_of(page) == Some(run),
        _ => false,
    }
}

pub(crate) fn attach_target<'a>(claim: &Claim, live: &'a [LiveSession]) -> Option<&'a LiveSession> {
    let sid = claim.session.as_ref()?.claude_session_id.as_deref().filter(|s| !s.is_empty())?;
    live.iter().find(|s| s.claude_session_id == sid)
}

impl WorkView {
    pub(crate) fn show_detail(&mut self, id: String, cx: &mut Context<Self>) {
        if self.item.as_ref().is_some_and(|open| open.id == id) {
            return;
        }
        self.item = Some(OpenItem { id, state: ItemState::Loading });
        self.detail_tab = DetailTab::Overview;
        self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.read_item(cx);
    }

    /// The Inbox's Open: the item's run in the list, the item selected once its run is known.
    pub fn open_from_inbox(&mut self, id: String, cx: &mut Context<Self>) {
        self.page_chosen = true;
        self.selected = Some(id.clone());
        self.show_detail(id, cx);
        cx.notify();
    }

    pub(super) fn schedule_item(&mut self, cx: &mut Context<Self>) {
        if !self.item_pending {
            self.item_pending = true;
            after_debounce(cx, |this, cx| {
                this.item_pending = false;
                this.read_item(cx);
            });
        }
    }

    pub(super) fn read_item(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.item.as_ref().map(|o| o.id.clone()) else { return };
        let Some(client) = self.client(cx) else { return };
        self.item_reads += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let asked = id.clone();
            let read = cx.background_executor().spawn(async move { client.item(&asked).map_err(|e| format!("Cannot read this item: {e}")) }).await;
            this.update(cx, |this, cx| this.apply_item(&id, read, cx)).ok();
        })
        .detach();
    }

    fn apply_item(&mut self, id: &str, read: Result<ItemPage, String>, cx: &mut Context<Self>) {
        let Some(open) = self.item.as_mut().filter(|o| o.id == id) else { return };
        match (read, &open.state) {
            (Ok(page), ItemState::Ready(shown)) if page.seq < shown.seq => {}
            (Ok(page), _) => open.state = ItemState::Ready(Box::new(page)),
            // A failed re-read keeps the page on screen; the next event reads it again.
            (Err(_), ItemState::Ready(_)) => {}
            (Err(message), _) => open.state = ItemState::Failed(message),
        }
        let page = open.page();
        let run = page.and_then(run_of).map(str::to_string);
        let blocked = page.is_some_and(|p| p.item.kind != "run" && state::of_page(p).key == state::StateKey::Stop);
        if blocked {
            self.read_prompts(id.to_string(), cx);
        }
        if let Some(run) = run.filter(|run| self.selected.as_deref() == Some(id) && self.shown_run() != Some(run.as_str())) {
            let selected = self.selected.take();
            self.open_run(run, cx);
            self.selected = selected;
            self.show_detail(id.to_string(), cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use supraman_workman::Session;

    fn iref(id: &str, kind: &str) -> ItemRef {
        ItemRef { id: id.into(), ref_: id.into(), qualified_ref: id.into(), kind: kind.into(), ..Default::default() }
    }

    fn page() -> ItemPage {
        ItemPage { item: iref("t", "task"), path: vec![iref("p", "project"), iref("r", "run"), iref("ph", "phase")], ..Default::default() }
    }

    #[test]
    fn an_event_reloads_the_open_item_when_it_names_the_item_or_its_run() {
        let open = |state| OpenItem { id: "t".into(), state };
        let event = |run: Option<&str>, item: Option<&str>| StreamEvent {
            seq: 1,
            run_id: run.map(str::to_string),
            item_id: item.map(str::to_string),
            kind: "transition".into(),
        };
        let ready = open(ItemState::Ready(Box::new(page())));
        assert!(item_to_reload(&event(None, Some("t")), &ready));
        assert!(item_to_reload(&event(Some("r"), Some("other")), &ready));
        assert!(!item_to_reload(&event(Some("r2"), Some("other")), &ready));
        assert!(!item_to_reload(&event(None, None), &ready));
        let loading = open(ItemState::Loading);
        assert!(item_to_reload(&event(None, Some("t")), &loading));
        assert!(!item_to_reload(&event(Some("r"), None), &loading));
    }

    #[test]
    fn a_run_is_its_own_run() {
        let mut run = page();
        run.item = iref("r9", "run");
        assert_eq!(run_of(&run), Some("r9"));
        assert_eq!(run_of(&page()), Some("r"));
        assert_eq!(run_of(&ItemPage::default()), None);
    }

    fn live_session(claude_session_id: &str, attach: &str, name: &str) -> LiveSession {
        LiveSession { name: name.into(), claude_session_id: claude_session_id.into(), cwd: Default::default(), activity: supraman_ui::marks::Activity::Working, attach: Some(attach.into()) }
    }

    #[test]
    fn a_claim_joins_a_live_session_on_its_claude_session_id() {
        let live = vec![
            live_session("uuid-a", "aaaaaaaa", "aw-a"),
            live_session("uuid-b", "bbbbbbbb", "aw-b"),
        ];
        let claim = |sid: Option<&str>| Claim {
            session: Some(Session { name: "aw-b".into(), claude_session_id: sid.map(str::to_string), ..Default::default() }),
            ..Default::default()
        };
        assert_eq!(attach_target(&claim(Some("uuid-b")), &live).and_then(|s| s.attach.as_deref()), Some("bbbbbbbb"));
        assert_eq!(attach_target(&claim(Some("uuid-gone")), &live), None, "the name alone is not a join");
        assert_eq!(attach_target(&claim(None), &live), None);
        assert_eq!(attach_target(&claim(Some("")), &[LiveSession { claude_session_id: String::new(), ..live[0].clone() }]), None);
        assert_eq!(attach_target(&Claim::default(), &live), None);
    }
}
